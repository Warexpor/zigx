use std::time::{Duration, Instant};

use crate::model::{AppState, Col, Density, Drag, Page, ScrollBar};

use super::frame::HitKind;

#[derive(Clone, Copy, Debug)]
pub enum KeyIn {
    Char(char),
    Backspace,
    Escape,
    Delete,
    Enter,
    PageUp,
    PageDown,
}

pub enum Effect {
    Exit,
    Minimize,
    DragWindow,
    Kill(Vec<i32>),
    FlipStartup(usize),
    UndoStartup,
    Persist,
}

pub fn expire(state: &mut AppState) {
    let now = Instant::now();
    if state.armed.as_ref().is_some_and(|a| a.until <= now) {
        state.armed = None;
    }
    if state.undo.as_ref().is_some_and(|u| u.until <= now) {
        state.undo = None;
    }
}

pub fn on_press(
    state: &mut AppState,
    kind: HitKind,
    ctrl: bool,
    shift: bool,
    mouse: [f32; 2],
) -> Vec<Effect> {
    if !matches!(kind, HitKind::Search) {
        state.search_focused = false;
    }
    match kind {
        HitKind::DragWindow => vec![Effect::DragWindow],
        HitKind::Close => vec![Effect::Exit],
        HitKind::Minimize => vec![Effect::Minimize],
        HitKind::Page(page) => {
            state.page = page;
            state.search_focused = false;
            vec![Effect::Persist]
        }
        HitKind::Section(section) => {
            state.section = section;
            state.perf_scroll = 0.0;
            vec![Effect::Persist]
        }
        HitKind::View(view) => {
            state.view = view;
            state.scroll = 0.0;
            state.pinned.clear();
            vec![Effect::Persist]
        }
        HitKind::Density => {
            state.density = match state.density {
                Density::Comfortable => Density::Compact,
                Density::Compact => Density::Comfortable,
            };
            vec![Effect::Persist]
        }
        HitKind::Sort(col) => {
            if state.sort.col == col {
                state.sort.desc = !state.sort.desc;
            } else {
                state.sort.col = col;
                state.sort.desc = col != Col::Name && col != Col::User;
            }
            vec![Effect::Persist]
        }
        HitKind::Search => {
            state.search_focused = true;
            vec![]
        }
        HitKind::EndTask => end_task(state),
        HitKind::Undo => vec![Effect::UndoStartup],
        HitKind::Group(user_group) => {
            if user_group {
                state.user_open = !state.user_open;
            } else {
                state.system_open = !state.system_open;
            }
            // Visible indices shift; drop pins and re-capture after the next layout.
            state.pinned.clear();
            vec![]
        }
        HitKind::Proc { pid } => {
            select_proc(state, pid, ctrl, shift);
            vec![]
        }
        HitKind::Deselect => {
            clear_selection(state);
            vec![]
        }
        HitKind::Scroll(which) => {
            begin_scroll_drag(state, which, mouse[1]);
            vec![]
        }
        HitKind::Startup(index) => vec![Effect::FlipStartup(index)],
        HitKind::DragNav => {
            state.drag = Some(Drag::Nav {
                x0: 0.0,
                w0: state.nav_w,
            });
            vec![]
        }
        HitKind::DragSub => {
            state.drag = Some(Drag::Sub {
                x0: 0.0,
                w0: state.sub_w,
            });
            vec![]
        }
    }
}

/// Press positions are applied by [`note_drag_origin`] so splitters track the cursor.
pub fn note_drag_origin(state: &mut AppState, x: f32, y: f32) {
    if let Some(drag) = state.drag.as_mut() {
        match drag {
            Drag::Nav { x0, .. } | Drag::Sub { x0, .. } => *x0 = x,
            Drag::Scroll { y0, .. } => *y0 = y,
        }
    }
}

pub fn on_release(state: &mut AppState) -> Vec<Effect> {
    match state.drag.take() {
        Some(Drag::Nav { .. } | Drag::Sub { .. }) => vec![Effect::Persist],
        Some(Drag::Scroll { .. }) | None => vec![],
    }
}

pub fn on_move(state: &mut AppState, x: f32, y: f32) -> bool {
    match state.drag {
        Some(Drag::Nav { x0, w0 }) => {
            state.nav_w = (w0 + (x - x0)).clamp(160.0, 300.0);
            true
        }
        Some(Drag::Sub { x0, w0 }) => {
            state.sub_w = (w0 + (x - x0)).clamp(140.0, 260.0);
            true
        }
        Some(Drag::Scroll {
            which,
            y0,
            scroll0,
            track,
            max_scroll,
        }) => {
            let t = if track > 0.0 {
                (scroll0 + (y - y0) / track * max_scroll).clamp(0.0, max_scroll)
            } else {
                scroll0
            };
            set_scroll(state, which, t);
            true
        }
        None => false,
    }
}

pub fn on_wheel(
    state: &mut AppState,
    over_list: bool,
    over_detail: bool,
    over_startup: bool,
    dy: f32,
) {
    if over_list {
        state.scroll = (state.scroll + dy).max(0.0);
    } else if over_detail {
        state.perf_scroll = (state.perf_scroll + dy).max(0.0);
    } else if over_startup {
        state.startup_scroll = (state.startup_scroll + dy).max(0.0);
    }
}

pub fn on_key(state: &mut AppState, key: KeyIn, ctrl: bool) -> Vec<Effect> {
    match key {
        KeyIn::Char(c) => {
            if ctrl && (c == '+' || c == '=') {
                return zoom(state, 1);
            }
            if ctrl && c == '-' {
                return zoom(state, -1);
            }
            if ctrl && c == '0' {
                return zoom_reset(state);
            }
            if ctrl && (c == 'f' || c == 'F') {
                state.search_focused = true;
                return vec![];
            }
            if ctrl {
                return vec![];
            }
            if !state.search_focused {
                match c {
                    '1' => {
                        state.page = Page::Processes;
                        return vec![Effect::Persist];
                    }
                    '2' => {
                        state.page = Page::Performance;
                        return vec![Effect::Persist];
                    }
                    '3' => {
                        state.page = Page::Startup;
                        return vec![Effect::Persist];
                    }
                    'c' | 'C' if state.page == Page::Performance => {
                        state.section = crate::model::Section::Cpu;
                        state.perf_scroll = 0.0;
                        return vec![Effect::Persist];
                    }
                    'm' | 'M' if state.page == Page::Performance => {
                        state.section = crate::model::Section::Memory;
                        state.perf_scroll = 0.0;
                        return vec![Effect::Persist];
                    }
                    'g' | 'G' if state.page == Page::Performance => {
                        state.section = crate::model::Section::Gpu;
                        state.perf_scroll = 0.0;
                        return vec![Effect::Persist];
                    }
                    'd' | 'D' if state.page == Page::Performance => {
                        state.section = crate::model::Section::Disk;
                        state.perf_scroll = 0.0;
                        return vec![Effect::Persist];
                    }
                    'n' | 'N' if state.page == Page::Performance => {
                        state.section = crate::model::Section::Net;
                        state.perf_scroll = 0.0;
                        return vec![Effect::Persist];
                    }
                    _ => state.search_focused = true,
                }
            }
            if !c.is_control() {
                state.query.push(c);
                state.scroll = 0.0;
            }
            vec![]
        }
        KeyIn::Backspace => {
            if state.search_focused {
                if ctrl {
                    state.query.clear();
                } else {
                    state.query.pop();
                }
                state.scroll = 0.0;
            }
            vec![]
        }
        KeyIn::Escape => {
            if state.search_focused {
                state.query.clear();
                state.search_focused = false;
                state.scroll = 0.0;
            }
            state.armed = None;
            vec![]
        }
        KeyIn::Delete => {
            if state.search_focused {
                vec![]
            } else {
                end_task(state)
            }
        }
        KeyIn::Enter => {
            if state.armed.is_some() && !state.search_focused {
                end_task(state)
            } else {
                state.search_focused = false;
                vec![]
            }
        }
        KeyIn::PageUp => {
            nudge_scroll(state, -240.0);
            vec![]
        }
        KeyIn::PageDown => {
            nudge_scroll(state, 240.0);
            vec![]
        }
    }
}

fn zoom(state: &mut AppState, delta: i32) -> Vec<Effect> {
    let next = crate::model::step_ui_scale(state.ui_scale, delta);
    if (next - state.ui_scale).abs() < 0.001 {
        return vec![];
    }
    state.ui_scale = next;
    vec![Effect::Persist]
}

fn zoom_reset(state: &mut AppState) -> Vec<Effect> {
    if (state.ui_scale - 1.0).abs() < 0.001 {
        return vec![];
    }
    state.ui_scale = 1.0;
    vec![Effect::Persist]
}

fn nudge_scroll(state: &mut AppState, dy: f32) {
    match state.page {
        Page::Processes => state.scroll = (state.scroll + dy).max(0.0),
        Page::Performance => state.perf_scroll = (state.perf_scroll + dy).max(0.0),
        Page::Startup => state.startup_scroll = (state.startup_scroll + dy).max(0.0),
    }
}

fn select_proc(state: &mut AppState, pid: i32, ctrl: bool, shift: bool) {
    if shift {
        if let Some(anchor) = state.anchor {
            if let (Some(a), Some(b)) = (
                state.visible_pids.iter().position(|p| *p == anchor),
                state.visible_pids.iter().position(|p| *p == pid),
            ) {
                let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
                if !ctrl {
                    state.selected.clear();
                }
                for p in &state.visible_pids[lo..=hi] {
                    state.selected.insert(*p);
                }
                sync_pins(state);
                return;
            }
        }
    }
    if ctrl {
        if !state.selected.remove(&pid) {
            state.selected.insert(pid);
        }
        state.anchor = Some(pid);
        sync_pins(state);
        return;
    }
    state.selected.clear();
    state.selected.insert(pid);
    state.anchor = Some(pid);
    sync_pins(state);
}

pub fn clear_selection(state: &mut AppState) {
    state.selected.clear();
    state.pinned.clear();
    state.anchor = None;
    state.armed = None;
}

fn begin_scroll_drag(state: &mut AppState, which: ScrollBar, y: f32) {
    let Some(g) = state.scroll_bar.filter(|g| g.which == which) else {
        return;
    };
    let track = (g.track_h - g.thumb_h).max(1.0);
    let scroll0 = if y < g.thumb_y || y > g.thumb_y + g.thumb_h {
        // Click in the track: jump so the thumb centers on the pointer.
        let t = ((y - g.track_y - g.thumb_h * 0.5) / track).clamp(0.0, 1.0);
        let s = t * g.max_scroll;
        set_scroll(state, which, s);
        s
    } else {
        get_scroll(state, which)
    };
    state.drag = Some(Drag::Scroll {
        which,
        y0: y,
        scroll0,
        track,
        max_scroll: g.max_scroll,
    });
}

fn get_scroll(state: &AppState, which: ScrollBar) -> f32 {
    match which {
        ScrollBar::Processes => state.scroll,
        ScrollBar::Performance => state.perf_scroll,
        ScrollBar::Startup => state.startup_scroll,
    }
}

fn set_scroll(state: &mut AppState, which: ScrollBar, v: f32) {
    match which {
        ScrollBar::Processes => state.scroll = v.max(0.0),
        ScrollBar::Performance => state.perf_scroll = v.max(0.0),
        ScrollBar::Startup => state.startup_scroll = v.max(0.0),
    }
}

/// Freeze each selected process at its current list index.
fn sync_pins(state: &mut AppState) {
    state.pinned.clear();
    if state.selected.is_empty() {
        return;
    }
    for &pid in &state.selected {
        if let Some(i) = state.visible_pids.iter().position(|p| *p == pid) {
            state.pinned.insert(pid, i);
        }
    }
}

fn end_task(state: &mut AppState) -> Vec<Effect> {
    if state.selected.is_empty() {
        return vec![];
    }
    let now = Instant::now();
    if let Some(armed) = &state.armed {
        if armed.until > now && armed.pids == state.selected {
            let pids: Vec<i32> = state.selected.iter().copied().collect();
            state.armed = None;
            return vec![Effect::Kill(pids)];
        }
    }
    state.armed = Some(crate::model::Armed {
        until: now + Duration::from_secs(4),
        pids: state.selected.clone(),
    });
    vec![]
}

pub fn terminate(pids: &[i32]) -> usize {
    let me = std::process::id() as i32;
    let mut n = 0;
    for &pid in pids {
        if pid <= 1 || pid == me {
            continue;
        }
        let rc = unsafe { libc::kill(pid, libc::SIGTERM) };
        if rc == 0 {
            n += 1;
        }
    }
    n
}

pub fn selection_label(state: &AppState) -> String {
    let n = state.selected.len();
    if n == 0 {
        return "End task".into();
    }
    let armed = state
        .armed
        .as_ref()
        .is_some_and(|a| a.until > Instant::now() && a.pids == state.selected);
    if armed {
        format!("Confirm {n}")
    } else if n == 1 {
        "End task".into()
    } else {
        format!("End {n}")
    }
}
