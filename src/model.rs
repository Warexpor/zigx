use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Instant;

pub const HIST_CAP: usize = 120;

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
        }
    }
}

#[derive(Clone, Debug)]
pub struct StartupEntry {
    pub name: String,
    pub exec: String,
    pub path: PathBuf,
    pub enabled: bool,
}

#[derive(Clone, Debug)]
pub struct Revert {
    pub path: PathBuf,
    pub previous: String,
}

#[derive(Clone, Copy, Debug)]
pub enum Drag {
    Nav { x0: f32, w0: f32 },
    Sub { x0: f32, w0: f32 },
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
    pub anchor: Option<i32>,
    pub scroll: f32,
    pub perf_scroll: f32,
    pub startup_scroll: f32,
    pub nav_w: f32,
    pub sub_w: f32,
    pub drag: Option<Drag>,
    pub armed: Option<Armed>,
    pub undo: Option<Undo>,
    pub user_open: bool,
    pub system_open: bool,
    pub width: f32,
    pub height: f32,
    pub always_on_top: bool,
    pub visible_pids: Vec<i32>,
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
            anchor: None,
            scroll: 0.0,
            perf_scroll: 0.0,
            startup_scroll: 0.0,
            nav_w: 200.0,
            sub_w: 196.0,
            drag: None,
            armed: None,
            undo: None,
            user_open: true,
            system_open: true,
            width,
            height,
            always_on_top: false,
            visible_pids: Vec::new(),
        }
    }
}

pub mod theme {
    pub type Rgba = [u8; 4];

    // x.ai dark tokens: jet canvas, charcoal cards, nimbus ink.
    // Hierarchy is alpha-stepped, exactly like text-primary/60 and /30.
    pub const INK: Rgba = [238, 240, 246, 255];
    pub const DIM: Rgba = [238, 240, 246, 160];
    pub const MUTED: Rgba = [238, 240, 246, 108];
    pub const FAINT: Rgba = [238, 240, 246, 70];

    // Surfaces. Jet glass root; charcoal for lifted cards and pills.
    pub const WELL: Rgba = [10, 10, 10, 205];
    pub const CARD: Rgba = [26, 26, 26, 225];
    pub const WELL_BORDER: Rgba = [238, 240, 246, 18];
    pub const DIVIDER: Rgba = [238, 240, 246, 12];
    pub const HOVER: Rgba = [238, 240, 246, 9];
    pub const SELECTED: Rgba = [238, 240, 246, 15];
    pub const SOFT: Rgba = [238, 240, 246, 8];
    pub const SOFT_BORDER: Rgba = [238, 240, 246, 16];

    // White is the accent; solid white pill + near-black text is the
    // primary-action signature.
    pub const ACCENT: Rgba = [245, 246, 250, 255];
    pub const ACCENT_DIM: Rgba = [238, 240, 246, 180];
    pub const ACCENT_SOFT: Rgba = [238, 240, 246, 15];
    pub const ACCENT_LINE: Rgba = [238, 240, 246, 44];
    pub const ON_ACCENT: Rgba = [12, 12, 14, 255];

    // Heat: the only chroma, and only when something runs hot.
    pub const WARN: Rgba = [240, 183, 104, 255];
    pub const HOT: Rgba = [255, 112, 112, 255];
    pub const DANGER_SOFT: Rgba = [255, 100, 100, 24];
    pub const DANGER_INK: Rgba = [255, 168, 168, 255];

    // Traces.
    pub const TRACE: Rgba = [238, 240, 246, 220];
    pub const TRACE_SOFT: Rgba = [238, 240, 246, 115];
    pub const TRACE_DIM: Rgba = [140, 142, 150, 165];

    // Floating toast.
    pub const TOAST: Rgba = [22, 22, 24, 245];
}
