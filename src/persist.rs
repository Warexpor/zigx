use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use crate::model::{AppState, Col, Density, Page, ProcView, Section, Sort};
use crate::settings::OpenOn;

pub(crate) fn config_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg).join("zigx");
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home).join(".config").join("zigx")
}

/// Load settings, then the remembered layout, then apply "Open on".
pub fn load_ui(state: &mut AppState) {
    state.settings = crate::settings::load_settings();
    load_layout(state);
    state.page = match state.settings.open_on {
        OpenOn::Last => state.page,
        OpenOn::Processes => Page::Processes,
        OpenOn::Performance => Page::Performance,
        OpenOn::Startup => Page::Startup,
    };
}

/// Persist both files. Settings failures are the ones worth reporting.
pub fn save_ui(state: &AppState) -> io::Result<()> {
    save_layout(state)?;
    crate::settings::save_settings(&state.settings)
}

/// Ticket for each background save, and the newest one that has landed.
/// Saves are serialized under the lock so two quick clicks cannot fight over
/// the same temp file, and ordered so an older state never overwrites a newer
/// one that happened to finish first.
static SAVE_SEQ: AtomicU64 = AtomicU64::new(0);
static SAVE_DONE: Mutex<u64> = Mutex::new(0);

/// Serialize on the UI thread, fsync on a worker so page/section clicks are
/// not stalled by disk (atomic_write syncs before rename).
pub fn save_ui_bg(state: &AppState) {
    let layout = layout_body(state);
    let settings = state.settings.render();
    let seq = SAVE_SEQ.fetch_add(1, Ordering::Relaxed) + 1;
    let _ = std::thread::Builder::new()
        .name("zigx-save".into())
        .spawn(move || {
            let mut done = SAVE_DONE.lock().unwrap_or_else(|e| e.into_inner());
            if seq <= *done {
                // A newer state is already on disk.
                return;
            }
            *done = seq;
            if let Err(err) = commit_save(&layout, &settings) {
                eprintln!("zigx: could not save settings: {err}");
            }
        });
}

fn layout_body(state: &AppState) -> String {
    let sort_dir = if state.sort.desc { "desc" } else { "asc" };
    format!(
        "nav_w={}\nsub_w={}\npage={}\nsection={}\nview={}\ndensity={}\nsort={},{sort_dir}\nui={}\n",
        state.nav_w.round() as i32,
        state.sub_w.round() as i32,
        page_name(state.page),
        section_name(state.section),
        view_name(state.view),
        if state.density == Density::Compact {
            "compact"
        } else {
            "comfortable"
        },
        col_name(state.sort.col),
        (state.ui_scale * 100.0).round() as i32,
    )
}

fn commit_save(layout: &str, settings: &str) -> io::Result<()> {
    let dir = config_dir();
    fs::create_dir_all(&dir)?;
    atomic_write(&dir.join("ui.txt"), layout)?;
    atomic_write(&dir.join("settings.txt"), settings)
}

fn load_layout(state: &mut AppState) {
    let path = config_dir().join("ui.txt");
    let Ok(text) = fs::read_to_string(path) else {
        return;
    };
    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        match k {
            "nav_w" => {
                if let Ok(n) = v.parse::<f32>() {
                    state.nav_w = n.clamp(160.0, 300.0);
                }
            }
            "sub_w" => {
                if let Ok(n) = v.parse::<f32>() {
                    state.sub_w = n.clamp(140.0, 260.0);
                }
            }
            "page" => {
                state.page = match v {
                    "performance" => Page::Performance,
                    "startup" => Page::Startup,
                    "settings" => Page::Settings,
                    _ => Page::Processes,
                };
            }
            "section" => {
                state.section = match v {
                    "memory" => Section::Memory,
                    "gpu" => Section::Gpu,
                    "disk" => Section::Disk,
                    "net" => Section::Net,
                    _ => Section::Cpu,
                };
            }
            "view" => {
                state.view = match v {
                    "flat" => ProcView::Flat,
                    "user" => ProcView::User,
                    "system" => ProcView::System,
                    _ => ProcView::Grouped,
                };
            }
            "ui" => {
                if let Ok(n) = v.parse::<f32>() {
                    state.ui_scale = crate::model::snap_ui_scale(n / 100.0);
                }
            }
            "density" => {
                state.density = if v == "compact" {
                    Density::Compact
                } else {
                    Density::Comfortable
                };
            }
            "sort" => {
                if let Some((col, dir)) = v.split_once(',') {
                    state.sort = Sort {
                        col: parse_col(col),
                        desc: dir != "asc",
                    };
                }
            }
            _ => {}
        }
    }
}

fn save_layout(state: &AppState) -> io::Result<()> {
    let dir = config_dir();
    fs::create_dir_all(&dir)?;
    atomic_write(&dir.join("ui.txt"), &layout_body(state))
}

pub fn atomic_write(path: &Path, data: &str) -> io::Result<()> {
    let tmp = {
        let mut name = path.as_os_str().to_os_string();
        name.push(".zigx-tmp");
        PathBuf::from(name)
    };
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(data.as_bytes())?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)
}

fn page_name(p: Page) -> &'static str {
    match p {
        Page::Processes => "processes",
        Page::Performance => "performance",
        Page::Startup => "startup",
        Page::Settings => "settings",
    }
}

fn section_name(s: Section) -> &'static str {
    match s {
        Section::Cpu => "cpu",
        Section::Memory => "memory",
        Section::Gpu => "gpu",
        Section::Disk => "disk",
        Section::Net => "net",
    }
}

fn view_name(v: ProcView) -> &'static str {
    match v {
        ProcView::Grouped => "grouped",
        ProcView::Flat => "flat",
        ProcView::User => "user",
        ProcView::System => "system",
    }
}

fn col_name(c: Col) -> &'static str {
    match c {
        Col::Name => "name",
        Col::Cpu => "cpu",
        Col::Gpu => "gpu",
        Col::Memory => "memory",
        Col::Disk => "disk",
        Col::Pid => "pid",
        Col::User => "user",
        Col::Threads => "threads",
    }
}

fn parse_col(s: &str) -> Col {
    match s {
        "name" => Col::Name,
        "gpu" => Col::Gpu,
        "memory" => Col::Memory,
        "disk" => Col::Disk,
        "pid" => Col::Pid,
        "user" => Col::User,
        "threads" => Col::Threads,
        _ => Col::Cpu,
    }
}
