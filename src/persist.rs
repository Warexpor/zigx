use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::model::{AppState, Col, Density, Page, ProcView, Section, Sort};

fn config_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg).join("zigx");
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home).join(".config").join("zigx")
}

pub fn load_ui(state: &mut AppState) {
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

pub fn save_ui(state: &AppState) -> io::Result<()> {
    let dir = config_dir();
    fs::create_dir_all(&dir)?;
    let path = dir.join("ui.txt");
    let sort_dir = if state.sort.desc { "desc" } else { "asc" };
    let body = format!(
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
    );
    atomic_write(&path, &body)
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
