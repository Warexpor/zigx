use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::Instant;

use crate::anim::{Anim, Spring};
use crate::settings::Settings;

/// Longest graph window in samples: 2 min of history at 0.5 s updates. The
/// window actually drawn comes from [`Settings::window`].
pub const MAX_WINDOW: usize = 240;
/// Ring capacity: the window plus the playback delay and interpolation taps.
pub const HIST_CAP: usize = MAX_WINDOW + 6;
/// Graphs play back this many samples behind the newest one. A curve segment
/// depends on the sample after its end, so the right edge must stay two
/// samples back for every drawn segment to be final: nothing already on screen
/// reshapes when a new sample lands. The fraction over 2.0 absorbs jitter.
pub const GRAPH_DELAY: f64 = 2.15;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Page {
    Processes,
    Performance,
    Startup,
    Settings,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Section {
    Cpu,
    Memory,
    Gpu,
    Disk,
    Net,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ProcView {
    Grouped,
    Flat,
    User,
    System,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Density {
    Comfortable,
    Compact,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Col {
    Name,
    Cpu,
    Gpu,
    Memory,
    Disk,
    Pid,
    User,
    Threads,
}

#[derive(Clone, Copy, Debug)]
pub struct Sort {
    pub col: Col,
    pub desc: bool,
}

#[derive(Clone, Debug)]
pub struct Proc {
    pub pid: i32,
    pub uid: u32,
    pub user: String,
    pub name: String,
    pub cpu: f32,
    pub gpu: f32,
    pub rss: u64,
    pub read_bps: Option<f64>,
    pub write_bps: Option<f64>,
    pub threads: u32,
    pub is_user: bool,
    /// Stopped by a signal (state `T`), e.g. suspended from the context menu.
    pub stopped: bool,
}

#[derive(Clone, Debug)]
pub struct Disk {
    pub name: String,
    pub read_bps: f64,
    pub write_bps: f64,
    pub read_hist: Vec<f32>,
    pub write_hist: Vec<f32>,
}

#[derive(Clone, Debug)]
pub struct Net {
    pub name: String,
    pub rx_bps: f64,
    pub tx_bps: f64,
    pub rx_hist: Vec<f32>,
    pub tx_hist: Vec<f32>,
}

#[derive(Clone, Debug)]
pub struct Gpu {
    pub name: String,
    pub util: Option<f32>,
    pub mem_used: u64,
    pub mem_total: u64,
    pub temp_c: Option<u32>,
    pub power_w: Option<f32>,
    pub clk_core: Option<u32>,
    pub clk_mem: Option<u32>,
    pub enc: Option<u32>,
    pub dec: Option<u32>,
    pub integrated: bool,
    pub util_hist: Vec<f32>,
}

#[derive(Clone, Debug)]
pub struct Snap {
    pub cpu_model: String,
    pub cpu_total: f32,
    pub cpu_per: Vec<f32>,
    pub cpu_freq_mhz: Vec<f32>,
    pub cpu_hist: Vec<f32>,
    pub cpu_per_hist: Vec<Vec<f32>>,
    pub mem_total: u64,
    pub mem_used: u64,
    pub mem_available: u64,
    pub mem_cached: u64,
    pub mem_buffers: u64,
    pub swap_total: u64,
    pub swap_used: u64,
    pub mem_hist: Vec<f32>,
    pub swap_hist: Vec<f32>,
    pub disks: Vec<Disk>,
    pub nets: Vec<Net>,
    pub gpus: Vec<Gpu>,
    pub procs: Vec<Proc>,
    pub proc_count: u32,
    pub thread_count: u32,
    pub kthreads: u32,
    pub uptime_secs: u64,
    pub load: [f32; 3],
    pub sample_ms: f32,
    /// Instant the ring buffers last advanced.
    pub hist_at: Instant,
    /// Count of ring advances; the newest ring entry carries this sequence number.
    pub hist_seq: u64,
}

impl Snap {
    pub fn placeholder() -> Self {
        Self {
            cpu_model: "Reading hardware".into(),
            cpu_total: 0.0,
            cpu_per: Vec::new(),
            cpu_freq_mhz: Vec::new(),
            cpu_hist: Vec::new(),
            cpu_per_hist: Vec::new(),
            mem_total: 0,
            mem_used: 0,
            mem_available: 0,
            mem_cached: 0,
            mem_buffers: 0,
            swap_total: 0,
            swap_used: 0,
            mem_hist: Vec::new(),
            swap_hist: Vec::new(),
            disks: Vec::new(),
            nets: Vec::new(),
            gpus: Vec::new(),
            procs: Vec::new(),
            proc_count: 0,
            thread_count: 0,
            kthreads: 0,
            uptime_secs: 0,
            load: [0.0; 3],
            sample_ms: 0.0,
            hist_at: Instant::now(),
            hist_seq: 0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct StartupEntry {
    pub name: String,
    pub exec: String,
    /// User-level file in `~/.config/autostart`. May not exist yet when the
    /// entry comes from a system directory; toggling creates it.
    pub path: PathBuf,
    /// System-level definition in `/etc/xdg/autostart` (or `$XDG_CONFIG_DIRS`).
    pub system_path: Option<PathBuf>,
    pub enabled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ScrollBar {
    Processes,
    Performance,
    Startup,
    Settings,
}

#[derive(Clone, Copy, Debug)]
pub struct ScrollGeom {
    pub which: ScrollBar,
    pub track_y: f32,
    pub track_h: f32,
    pub thumb_y: f32,
    pub thumb_h: f32,
    pub max_scroll: f32,
}

#[derive(Clone, Copy, Debug)]
pub enum Drag {
    Nav {
        x0: f32,
        w0: f32,
    },
    Sub {
        x0: f32,
        w0: f32,
    },
    Scroll {
        which: ScrollBar,
        y0: f32,
        scroll0: f32,
        track: f32,
        max_scroll: f32,
    },
}

#[derive(Clone, Debug)]
pub struct Armed {
    pub until: Instant,
    pub pids: BTreeSet<i32>,
}

/// Transient status line shown as a toast, e.g. the result of a signal.
#[derive(Clone, Debug)]
pub struct Notice {
    pub until: Instant,
    pub label: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MenuAction {
    EndTask,
    ForceKill,
    Suspend,
    Resume,
    OpenLocation,
    CopyPid,
    CopyCommand,
}

/// Right-click menu on the process list. Acts on `pids`, the selection at the
/// moment it opened.
#[derive(Clone, Debug)]
pub struct ContextMenu {
    /// Pointer position at open, design pixels.
    pub x: f32,
    pub y: f32,
    pub pids: Vec<i32>,
    pub opened: Instant,
    /// Actions in display order, set while building the frame.
    pub items: Vec<MenuAction>,
    /// Keyboard highlight, an index into `items`.
    pub focus: Option<usize>,
    /// Force kill was clicked once and waits for a second click.
    pub confirm_kill: bool,
}

pub struct AppState {
    pub page: Page,
    pub section: Section,
    pub view: ProcView,
    pub density: Density,
    pub sort: Sort,
    pub query: String,
    pub search_focused: bool,
    pub selected: BTreeSet<i32>,
    /// Selected PIDs stay at these `visible_pids` indices until deselected.
    pub pinned: BTreeMap<i32, usize>,
    /// Order captured when Space is held on Processes. Rows stay in this
    /// arrangement until the key is released; numbers keep updating.
    pub held: Option<Vec<i32>>,
    pub anchor: Option<i32>,
    pub scroll: f32,
    pub perf_scroll: f32,
    pub startup_scroll: f32,
    pub settings_scroll: f32,
    pub settings: Settings,
    /// Sampling is halted. Session only: ZIGX always launches live.
    pub paused: bool,
    /// Reset on the Settings page was clicked once and waits for a second click.
    pub reset_armed: Option<Instant>,
    pub nav_w: f32,
    pub sub_w: f32,
    pub drag: Option<Drag>,
    /// Geometry of the active page scrollbar (set while building the frame).
    pub scroll_bar: Option<ScrollGeom>,
    pub armed: Option<Armed>,
    pub notice: Option<Notice>,
    pub menu: Option<ContextMenu>,
    /// A menu that just closed, drawn fading out without hit targets.
    pub menu_ghost: Option<ContextMenu>,
    /// Last search keystroke or focus; the caret blinks from here.
    pub typed_at: Instant,
    pub user_open: bool,
    pub system_open: bool,
    pub width: f32,
    pub height: f32,
    /// Interface zoom. Layout stays in design pixels; this scales them onto the window.
    pub ui_scale: f32,
    pub visible_pids: Vec<i32>,
    /// Graph playback clock and eased bars; advanced on each paint of that page.
    pub perf_smooth: PerfSmooth,
    /// Interface animation values, carried from frame to frame.
    pub anim: Anim,
}

/// Display-side motion for the Performance page.
///
/// Text reads the snapshot directly, so numbers change once per sample. Only
/// graphics move between samples: graphs scroll on a steady playback clock and
/// bars and graph scales ease toward each new sample.
#[derive(Clone, Debug)]
pub struct PerfSmooth {
    primed: bool,
    at: Instant,
    /// Playback position in sample sequence units; the right edge of every graph.
    pos: f64,
    seq: u64,
    /// Reduced motion: the right edge moves in whole samples.
    stepped: bool,
    window: usize,
    /// VRAM fill fraction per GPU.
    pub vram: Vec<f32>,
    /// Graph scale per I/O device, keyed `disk:<name>` / `net:<name>`.
    /// Followed in log space so a 100x rescale reads as an even zoom
    /// instead of an instant squash.
    io_max: BTreeMap<String, Spring>,
}

impl Default for PerfSmooth {
    fn default() -> Self {
        Self {
            primed: false,
            at: Instant::now(),
            pos: 0.0,
            seq: 0,
            stepped: false,
            window: 30,
            vram: Vec::new(),
            io_max: BTreeMap::new(),
        }
    }
}

/// Time constants, seconds.
const CLOCK_TAU: f32 = 0.6;
const BAR_TAU: f32 = 0.14;
/// I/O scale smooth times. Growing is quicker so the scale is ready before a
/// burst is drawn; shrinking is slow so the graph relaxes after it leaves.
const SCALE_GROW: f32 = 0.35;
const SCALE_SHRINK: f32 = 0.9;

impl PerfSmooth {
    pub fn tick(&mut self, snap: &Snap, settings: &Settings, paused: bool) {
        let now = Instant::now();
        let dt = now.saturating_duration_since(self.at).as_secs_f32();
        self.at = now;
        let period = settings.speed.ms() as f64 / 1000.0;
        let reduced = settings.reduced();
        self.stepped = reduced;
        self.window = settings.window();
        // Capped so a resume after a pause glides back instead of jumping ahead.
        let lag = (snap.hist_at.elapsed().as_secs_f64() / period).min(1.5);
        let target = snap.hist_seq as f64 - GRAPH_DELAY + lag;

        // Advance at exactly one sample per period, then bleed off drift and
        // jitter slowly so the scroll speed never visibly changes. Paused
        // holds the playhead where it is.
        let fresh = !self.primed || (target - self.pos).abs() > 2.0;
        if fresh {
            self.pos = target;
        } else if !paused {
            self.pos += dt as f64 / period;
            self.pos += (target - self.pos) * rate(dt, CLOCK_TAU) as f64;
        }
        self.pos = self.pos.min(snap.hist_seq as f64);
        self.seq = snap.hist_seq;

        let bar = if fresh || reduced {
            1.0
        } else {
            rate(dt, BAR_TAU)
        };
        let vram: Vec<f32> = snap
            .gpus
            .iter()
            .map(|g| {
                if g.mem_total == 0 {
                    0.0
                } else {
                    (g.mem_used as f32 / g.mem_total as f32).clamp(0.0, 1.0)
                }
            })
            .collect();
        ease_vec(&mut self.vram, &vram, bar);

        let head = self.head();
        let mut keep = BTreeSet::new();
        let devs = snap
            .disks
            .iter()
            .map(|d| (format!("disk:{}", d.name), &d.read_hist, &d.write_hist))
            .chain(
                snap.nets
                    .iter()
                    .map(|n| (format!("net:{}", n.name), &n.rx_hist, &n.tx_hist)),
            );
        for (key, a, b) in devs {
            let goal = io_scale(a, b, head, self.window).ln();
            let m = self.io_max.entry(key.clone()).or_insert(Spring::new(goal));
            if fresh || reduced {
                *m = Spring::new(goal);
            } else {
                let t = if goal > m.value {
                    SCALE_GROW
                } else {
                    SCALE_SHRINK
                };
                m.step(goal, t, dt, 0.0);
            }
            keep.insert(key);
        }
        self.io_max.retain(|k, _| keep.contains(k));
        self.primed = true;
    }

    /// Right edge of the graphs, in samples relative to the newest one (<= 0).
    pub fn head(&self) -> f32 {
        let pos = if self.stepped {
            self.pos.floor()
        } else {
            self.pos
        };
        (pos - self.seq as f64) as f32
    }

    /// Samples spanned by a graph, right edge to left edge.
    pub fn window(&self) -> usize {
        self.window
    }

    pub fn io_max(&self, key: &str) -> Option<f32> {
        self.io_max.get(key).map(|d| d.value.exp())
    }
}

/// Target scale for a pair of rate histories: the nice ceiling of what the
/// graph shows at `head`, plus the samples the curve is heading into, so the
/// scale grows before a burst is drawn and relaxes once it scrolls out.
pub fn io_scale(a: &[f32], b: &[f32], head: f32, window: usize) -> f32 {
    crate::format::nice_ceil(window_peak(a, head, window).max(window_peak(b, head, window)))
}

fn window_peak(h: &[f32], head: f32, window: usize) -> f32 {
    if h.is_empty() {
        return 0.0;
    }
    let right = (h.len() - 1) as f32 + head;
    let lo = ((right - window as f32).floor() - 1.0).max(0.0) as usize;
    let hi = ((right.floor() + 2.0).max(0.0) as usize).min(h.len() - 1);
    if lo > hi {
        return 0.0;
    }
    h[lo..=hi].iter().copied().fold(0.0_f32, f32::max)
}

/// Frame-rate independent exponential approach factor.
fn rate(dt: f32, tau: f32) -> f32 {
    1.0 - (-dt / tau.max(0.001)).exp()
}

fn ease_vec(dst: &mut Vec<f32>, src: &[f32], a: f32) {
    if dst.len() != src.len() {
        *dst = src.to_vec();
        return;
    }
    for (d, s) in dst.iter_mut().zip(src) {
        *d += (*s - *d) * a;
    }
}

impl AppState {
    pub fn new(width: f32, height: f32) -> Self {
        Self {
            page: Page::Processes,
            section: Section::Cpu,
            view: ProcView::Grouped,
            density: Density::Comfortable,
            sort: Sort {
                col: Col::Cpu,
                desc: true,
            },
            query: String::new(),
            search_focused: false,
            selected: BTreeSet::new(),
            pinned: BTreeMap::new(),
            held: None,
            anchor: None,
            scroll: 0.0,
            perf_scroll: 0.0,
            startup_scroll: 0.0,
            settings_scroll: 0.0,
            settings: Settings::default(),
            paused: false,
            reset_armed: None,
            nav_w: 200.0,
            sub_w: 196.0,
            drag: None,
            scroll_bar: None,
            armed: None,
            notice: None,
            menu: None,
            menu_ghost: None,
            typed_at: Instant::now(),
            user_open: true,
            system_open: true,
            width,
            height,
            ui_scale: 1.0,
            visible_pids: Vec::new(),
            perf_smooth: PerfSmooth::default(),
            anim: Anim::default(),
        }
    }
}

/// Discrete zoom stops. Geometric so each Ctrl++ step is a similar jump.
const UI_STEPS: [f32; 8] = [0.8, 0.9, 1.0, 1.1, 1.25, 1.4, 1.6, 1.8];

pub fn snap_ui_scale(scale: f32) -> f32 {
    UI_STEPS
        .into_iter()
        .min_by(|a, b| {
            (*a - scale)
                .abs()
                .partial_cmp(&(*b - scale).abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .unwrap_or(1.0)
}

/// `delta` is +1 to enlarge, -1 to shrink. The ends of the stop list clamp.
pub fn step_ui_scale(scale: f32, delta: i32) -> f32 {
    let current = snap_ui_scale(scale);
    let i = UI_STEPS
        .iter()
        .position(|s| (*s - current).abs() < 0.001)
        .unwrap_or(2);
    let next = (i as i32 + delta).clamp(0, UI_STEPS.len() as i32 - 1) as usize;
    UI_STEPS[next]
}

pub mod theme {
    pub type Rgba = [u8; 4];

    // Mission-control monochrome. One ink, spectral white, stepped by alpha
    // (100 / 67 / 46 / 31). Surfaces are hairlines and ghost fills on black
    // glass; nothing is lifted, shaded, or tinted.
    const SPECTRAL: [u8; 3] = [240, 240, 250];
    const fn ink(a: u8) -> Rgba {
        [SPECTRAL[0], SPECTRAL[1], SPECTRAL[2], a]
    }

    pub const INK: Rgba = ink(255);
    pub const INK_2: Rgba = ink(170);
    pub const INK_3: Rgba = ink(118);
    pub const INK_4: Rgba = ink(80);

    // Glass root and its edge.
    pub const CANVAS: Rgba = [0, 0, 0, 200];
    pub const CANVAS_LINE: Rgba = ink(36);

    // Structure.
    pub const HAIRLINE: Rgba = ink(30);
    pub const GRID: Rgba = ink(14);
    pub const GHOST: Rgba = ink(22);
    pub const GHOST_LINE: Rgba = ink(66);
    pub const HOVER: Rgba = ink(16);
    pub const SELECTED: Rgba = ink(30);

    // The one filled element: white pill, black label.
    pub const ACCENT: Rgba = [255, 255, 255, 255];
    pub const ACCENT_LINE: Rgba = ink(140);
    pub const ON_ACCENT: Rgba = [0, 0, 0, 255];

    // Status chroma, and only status.
    pub const WARN: Rgba = [245, 166, 35, 255];
    pub const HOT: Rgba = [255, 92, 92, 255];
    pub const DANGER_LINE: Rgba = [255, 92, 92, 150];
    pub const DANGER_INK: Rgba = [255, 138, 138, 255];

    // Traces.
    pub const TRACE: Rgba = ink(235);
    pub const TRACE_2: Rgba = ink(110);

    // Floating notice: solid black glass, ghost edge.
    pub const TOAST: Rgba = [0, 0, 0, 230];
    // Menus sit over dense rows; near-opaque so nothing reads through.
    pub const MENU: Rgba = [0, 0, 0, 250];
}

#[cfg(test)]
mod tests {
    use super::{snap_ui_scale, step_ui_scale};

    #[test]
    fn zoom_steps_and_clamps() {
        assert!((step_ui_scale(1.0, 1) - 1.1).abs() < 0.001);
        assert!((step_ui_scale(1.0, -1) - 0.9).abs() < 0.001);
        assert!((step_ui_scale(0.8, -1) - 0.8).abs() < 0.001);
        assert!((step_ui_scale(1.8, 1) - 1.8).abs() < 0.001);
        assert!((snap_ui_scale(1.12) - 1.1).abs() < 0.001);
    }
}
