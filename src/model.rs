use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::Instant;

pub const HIST_CAP: usize = 150;
/// Graph sampler period. History covers `HIST_CAP * SAMPLE_PERIOD_MS` ms.
pub const SAMPLE_PERIOD_MS: u64 = 200;
pub const HIST_SECS: u64 = HIST_CAP as u64 * SAMPLE_PERIOD_MS / 1000;
/// How often Performance readouts pick up a new target value.
pub const METRIC_PERIOD_MS: u64 = 1000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Processes,
    Performance,
    Startup,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Section {
    Cpu,
    Memory,
    Gpu,
    Disk,
    Net,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
    /// Instant the ring buffers last advanced. Frame uses this for scroll phase.
    pub hist_at: Instant,
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

#[derive(Clone, Debug)]
pub struct Revert {
    pub path: PathBuf,
    /// `None` means the file did not exist before; undo removes it.
    pub previous: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollBar {
    Processes,
    Performance,
    Startup,
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
    Nav { x0: f32, w0: f32 },
    Sub { x0: f32, w0: f32 },
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

#[derive(Clone, Debug)]
pub struct Undo {
    pub until: Instant,
    pub label: String,
    pub revert: Option<Revert>,
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
    pub anchor: Option<i32>,
    pub scroll: f32,
    pub perf_scroll: f32,
    pub startup_scroll: f32,
    pub nav_w: f32,
    pub sub_w: f32,
    pub drag: Option<Drag>,
    /// Geometry of the active page scrollbar (set while building the frame).
    pub scroll_bar: Option<ScrollGeom>,
    pub armed: Option<Armed>,
    pub undo: Option<Undo>,
    pub user_open: bool,
    pub system_open: bool,
    pub width: f32,
    pub height: f32,
    /// Interface zoom. Layout stays in design pixels; this scales them onto the window.
    pub ui_scale: f32,
    pub visible_pids: Vec<i32>,
    /// EMA-smoothed performance readouts; advanced on each paint of that page.
    pub perf_smooth: PerfSmooth,
}

/// Display-side smoothing for the Performance page.
///
/// Readout targets refresh once a second. Graph rings follow every sampler
/// tick; only the tip eases so new samples glide in without morphing history.
#[derive(Clone, Debug)]
pub struct PerfSmooth {
    pub primed: bool,
    pub at: Instant,
    pub hist_at: Option<Instant>,
    pub cpu: f32,
    pub mem_used: f32,
    pub gpu: f32,
    pub cpu_per: Vec<f32>,
    pub cpu_hist: Vec<f32>,
    pub mem_hist: Vec<f32>,
    pub swap_hist: Vec<f32>,
    pub gpu_hist: Vec<Vec<f32>>,
    pub disk_hist: BTreeMap<String, (Vec<f32>, Vec<f32>)>,
    pub net_hist: BTreeMap<String, (Vec<f32>, Vec<f32>)>,
    metric_at: Instant,
    tgt_cpu: f32,
    tgt_mem: f32,
    tgt_gpu: f32,
    tgt_cpu_per: Vec<f32>,
}

impl Default for PerfSmooth {
    fn default() -> Self {
        Self {
            primed: false,
            at: Instant::now(),
            hist_at: None,
            cpu: 0.0,
            mem_used: 0.0,
            gpu: 0.0,
            cpu_per: Vec::new(),
            cpu_hist: Vec::new(),
            mem_hist: Vec::new(),
            swap_hist: Vec::new(),
            gpu_hist: Vec::new(),
            disk_hist: BTreeMap::new(),
            net_hist: BTreeMap::new(),
            metric_at: Instant::now(),
            tgt_cpu: 0.0,
            tgt_mem: 0.0,
            tgt_gpu: 0.0,
            tgt_cpu_per: Vec::new(),
        }
    }
}

impl PerfSmooth {
    /// `metric_tau` / `tip_tau` are half-lives in seconds.
    pub fn tick(&mut self, snap: &Snap, metric_tau: f32, tip_tau: f32) {
        let now = Instant::now();
        let dt = if self.primed {
            now.saturating_duration_since(self.at).as_secs_f32()
        } else {
            1.0
        };
        self.at = now;
        let am = 1.0 - (-dt / metric_tau.max(0.001)).exp();
        let at = 1.0 - (-dt / tip_tau.max(0.001)).exp();
        let gpu = snap.gpus.first().and_then(|g| g.util).unwrap_or(0.0);
        let advanced = self.hist_at != Some(snap.hist_at);
        self.hist_at = Some(snap.hist_at);

        if !self.primed {
            self.cpu = snap.cpu_total;
            self.mem_used = snap.mem_used as f32;
            self.gpu = gpu;
            self.cpu_per = snap.cpu_per.clone();
            self.tgt_cpu = self.cpu;
            self.tgt_mem = self.mem_used;
            self.tgt_gpu = self.gpu;
            self.tgt_cpu_per = self.cpu_per.clone();
            self.metric_at = now;
            self.cpu_hist = snap.cpu_hist.clone();
            self.mem_hist = snap.mem_hist.clone();
            self.swap_hist = snap.swap_hist.clone();
            self.gpu_hist = snap.gpus.iter().map(|g| g.util_hist.clone()).collect();
            self.disk_hist = snap
                .disks
                .iter()
                .map(|d| (d.name.clone(), (d.read_hist.clone(), d.write_hist.clone())))
                .collect();
            self.net_hist = snap
                .nets
                .iter()
                .map(|n| (n.name.clone(), (n.rx_hist.clone(), n.tx_hist.clone())))
                .collect();
            self.primed = true;
            return;
        }

        if self.metric_at.elapsed().as_millis() >= METRIC_PERIOD_MS as u128 {
            self.tgt_cpu = snap.cpu_total;
            self.tgt_mem = snap.mem_used as f32;
            self.tgt_gpu = gpu;
            self.tgt_cpu_per = snap.cpu_per.clone();
            self.metric_at = now;
        }

        self.cpu += (self.tgt_cpu - self.cpu) * am;
        self.mem_used += (self.tgt_mem - self.mem_used) * am;
        self.gpu += (self.tgt_gpu - self.gpu) * am;
        ease_vec(&mut self.cpu_per, &self.tgt_cpu_per, am);

        track_ring(&mut self.cpu_hist, &snap.cpu_hist, advanced, at);
        track_ring(&mut self.mem_hist, &snap.mem_hist, advanced, at);
        track_ring(&mut self.swap_hist, &snap.swap_hist, advanced, at);

        if self.gpu_hist.len() != snap.gpus.len() {
            self.gpu_hist = snap.gpus.iter().map(|g| g.util_hist.clone()).collect();
        } else {
            for (dst, g) in self.gpu_hist.iter_mut().zip(&snap.gpus) {
                track_ring(dst, &g.util_hist, advanced, at);
            }
        }

        sync_pair_map(
            &mut self.disk_hist,
            snap.disks
                .iter()
                .map(|d| (d.name.as_str(), d.read_hist.as_slice(), d.write_hist.as_slice())),
            advanced,
            at,
        );
        sync_pair_map(
            &mut self.net_hist,
            snap.nets
                .iter()
                .map(|n| (n.name.as_str(), n.rx_hist.as_slice(), n.tx_hist.as_slice())),
            advanced,
            at,
        );
    }

    pub fn disk_pair(&self, name: &str) -> Option<(&[f32], &[f32])> {
        self.disk_hist
            .get(name)
            .map(|(a, b)| (a.as_slice(), b.as_slice()))
    }

    pub fn net_pair(&self, name: &str) -> Option<(&[f32], &[f32])> {
        self.net_hist
            .get(name)
            .map(|(a, b)| (a.as_slice(), b.as_slice()))
    }
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

/// Keep the ribbon shape locked to the sampler ring; only the tip eases.
fn track_ring(dst: &mut Vec<f32>, src: &[f32], advanced: bool, tip_a: f32) {
    if src.len() < 2 {
        *dst = src.to_vec();
        return;
    }
    if dst.len() != src.len() {
        *dst = src.to_vec();
        return;
    }
    if advanced {
        let tip = dst.last().copied().unwrap_or(0.0);
        dst.remove(0);
        dst.push(tip);
    }
    let last = dst.len() - 1;
    dst[..last].copy_from_slice(&src[..last]);
    dst[last] += (src[last] - dst[last]) * tip_a;
}

fn sync_pair_map<'a, I>(
    dst: &mut BTreeMap<String, (Vec<f32>, Vec<f32>)>,
    src: I,
    advanced: bool,
    tip_a: f32,
) where
    I: IntoIterator<Item = (&'a str, &'a [f32], &'a [f32])>,
{
    let mut keep = BTreeSet::new();
    for (name, a_hist, b_hist) in src {
        keep.insert(name.to_string());
        let entry = dst.entry(name.to_string()).or_default();
        track_ring(&mut entry.0, a_hist, advanced, tip_a);
        track_ring(&mut entry.1, b_hist, advanced, tip_a);
    }
    dst.retain(|k, _| keep.contains(k));
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
            anchor: None,
            scroll: 0.0,
            perf_scroll: 0.0,
            startup_scroll: 0.0,
            nav_w: 200.0,
            sub_w: 196.0,
            drag: None,
            scroll_bar: None,
            armed: None,
            undo: None,
            user_open: true,
            system_open: true,
            width,
            height,
            ui_scale: 1.0,
            visible_pids: Vec::new(),
            perf_smooth: PerfSmooth::default(),
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
    // (100 / 62 / 38 / 22). Surfaces are hairlines and ghost fills on black
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
