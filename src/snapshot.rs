//! Renders fixed frames through the real GPU path into PNGs, for eyeballing
//! graph, stroke and chrome changes without running the app:
//!
//! ```sh
//! cargo test --bin zigx snapshots -- --ignored
//! ```
//!
//! Output lands in `target/snapshots/`: each page at 1x, plus its detail pane
//! at 4x where individual stroke pixels are visible.

use std::path::{Path, PathBuf};
use std::time::Instant;

use zigx::*;

use crate::gfx::Gfx;

const W: f32 = 1240.0;
const H: f32 = 780.0;
const ZOOM: f32 = 4.0;

/// Bursty disk traffic in KB/s: lone one-sample spikes, plateaus and ramps.
const READ_KB: [f32; 60] = [
    2.0, 1.0, 3.0, 2.0, 1.0, 2.0, 30.0, 32.0, 31.0, 45.0, 12.0, 10.0, 9.0, 8.0, 2.0, 1.0, 1.0,
    40.0, 2.0, 1.0, 2.0, 3.0, 18.0, 18.0, 17.0, 2.0, 1.0, 0.0, 0.0, 1.0, 2.0, 38.0, 36.0, 35.0,
    36.0, 60.0, 34.0, 30.0, 12.0, 3.0, 2.0, 1.0, 2.0, 3.0, 2.0, 100.0, 2.0, 1.0, 2.0, 3.0, 20.0,
    22.0, 21.0, 2.0, 1.0, 1.0, 2.0, 1.0, 0.0, 0.0,
];
const WRITE_KB: [f32; 60] = [
    1.0, 1.0, 2.0, 1.0, 1.0, 1.0, 25.0, 22.0, 12.0, 11.0, 10.0, 9.0, 9.0, 8.0, 7.0, 1.0, 1.0, 8.0,
    9.0, 1.0, 1.0, 2.0, 12.0, 13.0, 12.0, 2.0, 1.0, 0.0, 0.0, 1.0, 1.0, 20.0, 24.0, 25.0, 30.0,
    38.0, 30.0, 20.0, 8.0, 2.0, 1.0, 1.0, 1.0, 2.0, 1.0, 40.0, 1.0, 1.0, 1.0, 2.0, 10.0, 12.0,
    11.0, 1.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0,
];

/// History long enough to fill the window plus the playback delay, ending
/// with `tail`, and a snapshot whose newest sample landed just now.
fn history(tail: &[f32]) -> Vec<f32> {
    let lead = HIST_CAP.saturating_sub(tail.len());
    let mut h = vec![tail[0]; lead];
    h.extend_from_slice(tail);
    h
}

fn base_snap() -> Snap {
    let mut snap = Snap::placeholder();
    snap.hist_seq = 10_000;
    snap.hist_at = Instant::now();
    snap
}

fn disk_snap() -> Snap {
    let mut snap = base_snap();
    let kb = |v: &[f32]| v.iter().map(|x| x * 1024.0).collect::<Vec<_>>();
    snap.disks = vec![Disk {
        name: "nvme0n1".into(),
        read_bps: 2048.0,
        write_bps: 1024.0,
        read_hist: history(&kb(&READ_KB)),
        write_hist: history(&kb(&WRITE_KB)),
    }];
    snap
}

/// A large burst early in the window, then quiet traffic: the I/O scale holds
/// at the burst until it has scrolled off the left edge.
fn disk_burst_snap() -> Snap {
    let mut snap = base_snap();
    let mut read = [3.0_f32; 60];
    let mut write = [1.0_f32; 60];
    read[8..16].copy_from_slice(&[20.0, 60.0, 180.0, 240.0, 220.0, 90.0, 30.0, 8.0]);
    write[9..15].copy_from_slice(&[15.0, 70.0, 110.0, 80.0, 25.0, 5.0]);
    read[30] = 120.0;
    read[44..50].copy_from_slice(&[12.0, 25.0, 18.0, 30.0, 14.0, 6.0]);
    write[45..49].copy_from_slice(&[8.0, 12.0, 10.0, 4.0]);
    let kb = |v: &[f32]| v.iter().map(|x| x * 1024.0).collect::<Vec<_>>();
    snap.disks = vec![Disk {
        name: "nvme0n1".into(),
        read_bps: 6144.0,
        write_bps: 1024.0,
        read_hist: history(&kb(&read)),
        write_hist: history(&kb(&write)),
    }];
    snap
}

fn cpu_snap() -> Snap {
    let mut snap = base_snap();
    let mut total: Vec<f32> = READ_KB.iter().map(|v| v.min(100.0)).collect();
    // End busy so the per-core bars, which read the playhead, have height.
    total[54..].copy_from_slice(&[35.0, 48.0, 62.0, 55.0, 70.0, 64.0]);
    snap.cpu_model = "Snapshot CPU".into();
    snap.cpu_hist = history(&total);
    snap.cpu_total = *total.last().unwrap();
    snap.cpu_per_hist = (0..8)
        .map(|c| {
            let core: Vec<f32> = total
                .iter()
                .enumerate()
                .map(|(i, v)| if (i + c) % 3 == 0 { *v } else { v * 0.3 })
                .collect();
            history(&core)
        })
        .collect();
    snap.cpu_per = snap.cpu_per_hist.iter().map(|h| h[h.len() - 1]).collect();
    snap.cpu_freq_mhz = vec![3200.0; 8];
    snap
}

fn frame(section: Section, snap: &Snap) -> (DrawList, Rect) {
    let mut state = AppState::new(W, H);
    state.page = Page::Performance;
    state.section = section;
    state.settings.animations = false;
    // One sample per second over a minute: where lone bursts are narrowest.
    state.settings.history = History::S60;
    let draw = build(&mut state, snap, &[], [-1.0, -1.0]);
    let detail = draw.detail_rect.expect("performance detail pane");
    (draw, detail)
}

fn write_png(path: &Path, w: u32, h: u32, rgba: &[u8]) {
    let file = std::fs::File::create(path).expect("create snapshot");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()
        .and_then(|mut wr| wr.write_image_data(rgba))
        .expect("write snapshot");
}

fn crop(rgba: &[u8], stride: u32, r: [u32; 4]) -> Vec<u8> {
    let [x, y, w, h] = r;
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for row in y..y + h {
        let start = ((row * stride + x) * 4) as usize;
        out.extend_from_slice(&rgba[start..start + (w * 4) as usize]);
    }
    out
}

#[test]
#[ignore = "writes target/snapshots/ and needs a GPU"]
fn snapshots() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/snapshots");
    std::fs::create_dir_all(&dir).expect("snapshot dir");
    let (Some(mut gfx), Some(mut zoomed)) = (
        Gfx::headless(W as u32, H as u32),
        Gfx::headless((W * ZOOM) as u32, (H * ZOOM) as u32),
    ) else {
        eprintln!("no GPU adapter; skipping snapshots");
        return;
    };
    for (name, section, snap) in [
        ("disk", Section::Disk, disk_snap()),
        ("disk-burst", Section::Disk, disk_burst_snap()),
        ("cpu", Section::Cpu, cpu_snap()),
    ] {
        let (draw, detail) = frame(section, &snap);
        let rgba = gfx.capture(&draw, 1.0);
        write_png(&dir.join(format!("{name}.png")), W as u32, H as u32, &rgba);

        let big = zoomed.capture(&draw, ZOOM);
        let r = [
            (detail.x * ZOOM) as u32,
            (detail.y * ZOOM) as u32,
            (detail.w * ZOOM) as u32,
            (detail.h * ZOOM).min(H * ZOOM - detail.y * ZOOM) as u32,
        ];
        let part = crop(&big, (W * ZOOM) as u32, r);
        write_png(&dir.join(format!("{name}@4x.png")), r[2], r[3], &part);
        println!("wrote {}", dir.join(format!("{name}.png")).display());
    }

    // The nav foot at rest and with the pointer on the Keys pill, then the
    // shortcut sheet it opens.
    let snap = base_snap();
    let foot_h = 60.0;
    for (name, mouse) in [
        ("nav-foot", [-1.0, -1.0]),
        ("nav-foot-hover", [150.0, H - 23.0]),
    ] {
        let mut state = AppState::new(W, H);
        state.settings.animations = false;
        let draw = build(&mut state, &snap, &[], mouse);
        let big = zoomed.capture(&draw, ZOOM);
        let r = [
            0,
            ((H - foot_h) * ZOOM) as u32,
            (state.nav_w * ZOOM) as u32,
            (foot_h * ZOOM) as u32,
        ];
        let part = crop(&big, (W * ZOOM) as u32, r);
        write_png(&dir.join(format!("{name}@4x.png")), r[2], r[3], &part);
        println!("wrote {}", dir.join(format!("{name}@4x.png")).display());
    }
    let mut state = AppState::new(W, H);
    state.settings.animations = false;
    state.keys_open = true;
    let draw = build(&mut state, &snap, &[], [-1.0, -1.0]);
    let rgba = gfx.capture(&draw, 1.0);
    write_png(&dir.join("keys.png"), W as u32, H as u32, &rgba);
    println!("wrote {}", dir.join("keys.png").display());
}
