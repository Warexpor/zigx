use std::time::{Duration, Instant};

use crate::model::{AppState, Col, ContextMenu, Density, Drag, MenuAction, Page, ScrollBar};

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
    Up,
    Down,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sig {
    Term,
    Kill,
    Stop,
    Cont,
}

impl Sig {
    fn raw(self) -> i32 {
        match self {
            Sig::Term => libc::SIGTERM,
            Sig::Kill => libc::SIGKILL,
            Sig::Stop => libc::SIGSTOP,
            Sig::Cont => libc::SIGCONT,
        }
    }
}

pub enum Effect {
    Exit,
    Minimize,
    DragWindow,
    Signal(Vec<i32>, Sig),
    OpenLocation(i32),
    Copy(String),
    CopyCommand(i32),
    FlipStartup(usize),
    Persist,
}

pub fn expire(state: &mut AppState) {
    let now = Instant::now();
    if state.armed.as_ref().is_some_and(|a| a.until <= now) {
        state.armed = None;
    }
    if state.notice.as_ref().is_some_and(|u| u.until <= now) {
        state.notice = None;
    }
}

pub fn on_press(
    state: &mut AppState,
    kind: HitKind,
    ctrl: bool,
    shift: bool,
    mouse: [f32; 2],
) -> Vec<Effect> {
    // An open menu owns the next click: its items act, anything else only
    // dismisses it.
    if state.menu.is_some() {
        return match kind {
            HitKind::MenuItem(action) => menu_action(state, action),
            HitKind::MenuPanel => vec![],
            _ => {
                close_menu(state);
                vec![]
            }
        };
    }
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
        HitKind::MenuItem(_) | HitKind::MenuPanel => vec![],
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
    if state.menu.is_some() {
        close_menu(state);
        return;
    }
    if over_list {
        state.scroll = (state.scroll + dy).max(0.0);
    } else if over_detail {
        state.perf_scroll = (state.perf_scroll + dy).max(0.0);
    } else if over_startup {
        state.startup_scroll = (state.startup_scroll + dy).max(0.0);
    }
}

pub fn on_key(state: &mut AppState, key: KeyIn, ctrl: bool) -> Vec<Effect> {
    if state.menu.is_some() {
        return menu_key(state, key);
    }
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
        KeyIn::Up | KeyIn::Down => vec![],
    }
}

/// Open the process menu at the pointer. A row outside the selection becomes
/// the selection first, so the menu always acts on what is highlighted.
pub fn open_menu(state: &mut AppState, pid: i32, mouse: [f32; 2]) {
    if !state.selected.contains(&pid) {
        select_proc(state, pid, false, false);
    }
    state.armed = None;
    state.search_focused = false;
    state.menu = Some(ContextMenu {
        x: mouse[0],
        y: mouse[1],
        pids: state.selected.iter().copied().collect(),
        opened: Instant::now(),
        items: Vec::new(),
        focus: None,
        confirm_kill: false,
    });
}

pub fn close_menu(state: &mut AppState) {
    state.menu = None;
}

fn menu_key(state: &mut AppState, key: KeyIn) -> Vec<Effect> {
    let Some(menu) = state.menu.as_mut() else {
        return vec![];
    };
    let n = menu.items.len();
    match key {
        KeyIn::Escape => close_menu(state),
        KeyIn::Up | KeyIn::Down if n > 0 => {
            let step = if matches!(key, KeyIn::Down) { 1 } else { n - 1 };
            menu.focus = Some(match menu.focus {
                Some(i) => (i + step) % n,
                None if matches!(key, KeyIn::Down) => 0,
                None => n - 1,
            });
        }
        KeyIn::Enter => {
            if let Some(action) = menu.focus.and_then(|i| menu.items.get(i).copied()) {
                return menu_action(state, action);
            }
        }
        _ => {}
    }
    vec![]
}

fn menu_action(state: &mut AppState, action: MenuAction) -> Vec<Effect> {
    let Some(menu) = state.menu.as_mut() else {
        return vec![];
    };
    let pids = menu.pids.clone();
    if action == MenuAction::ForceKill && !menu.confirm_kill {
        menu.confirm_kill = true;
        return vec![];
    }
    close_menu(state);
    let first = pids.first().copied();
    match action {
        MenuAction::EndTask => vec![Effect::Signal(pids, Sig::Term)],
        MenuAction::ForceKill => vec![Effect::Signal(pids, Sig::Kill)],
        MenuAction::Suspend => vec![Effect::Signal(pids, Sig::Stop)],
        MenuAction::Resume => vec![Effect::Signal(pids, Sig::Cont)],
        MenuAction::OpenLocation => first.map(Effect::OpenLocation).into_iter().collect(),
        MenuAction::CopyCommand => first.map(Effect::CopyCommand).into_iter().collect(),
        MenuAction::CopyPid => {
            let text = pids
                .iter()
                .map(|p| p.to_string())
                .collect::<Vec<_>>()
                .join(" ");
            vec![Effect::Copy(text)]
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
            return vec![Effect::Signal(pids, Sig::Term)];
        }
    }
    state.armed = Some(crate::model::Armed {
        until: now + Duration::from_secs(4),
        pids: state.selected.clone(),
    });
    vec![]
}

/// Signal each PID, skipping init and ZIGX itself. Returns how many succeeded.
pub fn send_signal(pids: &[i32], sig: Sig) -> usize {
    let me = std::process::id() as i32;
    let mut n = 0;
    for &pid in pids {
        if pid <= 1 || pid == me {
            continue;
        }
        let rc = unsafe { libc::kill(pid, sig.raw()) };
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

#[cfg(test)]
mod tests {
    use super::*;

    fn menu_state() -> AppState {
        let mut state = AppState::new(800.0, 600.0);
        state.visible_pids = vec![10, 20, 30];
        state
    }

    #[test]
    fn right_click_outside_selection_selects_that_row() {
        let mut state = menu_state();
        state.selected.insert(10);
        open_menu(&mut state, 20, [5.0, 5.0]);
        assert_eq!(state.menu.as_ref().unwrap().pids, vec![20]);
        assert!(state.selected.contains(&20) && !state.selected.contains(&10));
    }

    #[test]
    fn right_click_inside_selection_keeps_it() {
        let mut state = menu_state();
        state.selected.extend([10, 30]);
        open_menu(&mut state, 30, [5.0, 5.0]);
        assert_eq!(state.menu.as_ref().unwrap().pids, vec![10, 30]);
    }

    #[test]
    fn force_kill_needs_a_second_click() {
        let mut state = menu_state();
        open_menu(&mut state, 10, [0.0, 0.0]);
        let kill = HitKind::MenuItem(MenuAction::ForceKill);
        let first = on_press(&mut state, kill, false, false, [0.0, 0.0]);
        assert!(first.is_empty());
        assert!(state.menu.as_ref().unwrap().confirm_kill);
        let second = on_press(&mut state, kill, false, false, [0.0, 0.0]);
        assert!(matches!(second.as_slice(), [Effect::Signal(p, Sig::Kill)] if p == &vec![10]));
        assert!(state.menu.is_none());
    }

    #[test]
    fn click_elsewhere_only_dismisses() {
        let mut state = menu_state();
        open_menu(&mut state, 10, [0.0, 0.0]);
        let fx = on_press(&mut state, HitKind::EndTask, false, false, [0.0, 0.0]);
        assert!(fx.is_empty());
        assert!(state.menu.is_none());
        assert!(state.armed.is_none(), "dismiss click must not arm End task");
    }

    #[test]
    fn keyboard_walks_items_and_activates() {
        let mut state = menu_state();
        open_menu(&mut state, 10, [0.0, 0.0]);
        state.menu.as_mut().unwrap().items = vec![MenuAction::EndTask, MenuAction::CopyPid];
        on_key(&mut state, KeyIn::Down, false);
        on_key(&mut state, KeyIn::Down, false);
        let fx = on_key(&mut state, KeyIn::Enter, false);
        assert!(matches!(fx.as_slice(), [Effect::Copy(t)] if t == "10"));
    }
}
