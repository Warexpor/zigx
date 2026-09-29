use std::time::{Duration, Instant};

use crate::model::{
    AppState, Col, ContextMenu, Density, Drag, KbFocus, KbProcRow, KbSettingCtl, KbSettingRow,
    MenuAction, Page, ProcView, ScrollBar, ScrollTarget, Section,
};
use crate::settings::{Choice, Opt, Settings, Speed};

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
    Home,
    End,
    Up,
    Down,
    Left,
    Right,
    /// Context menu key, or Shift+F10.
    Menu,
    /// F1: toggles the shortcut sheet.
    Help,
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
    Notify(String),
}

/// How long an expired notice may stay on screen while it fades out. The
/// toast normally clears itself sooner, once the fade settles.
const NOTICE_LINGER: Duration = Duration::from_millis(600);

pub fn expire(state: &mut AppState) {
    let now = Instant::now();
    if state.armed.as_ref().is_some_and(|a| a.until <= now) {
        state.armed = None;
    }
    if state
        .notice
        .as_ref()
        .is_some_and(|u| u.until + NOTICE_LINGER <= now)
    {
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
    if state.keys_open {
        if !matches!(kind, HitKind::KeysPanel) {
            state.keys_open = false;
        }
        return vec![];
    }
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
            state.kb = KbFocus::None;
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
            clear_pins(state, true);
            state.kb = KbFocus::None;
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
            state.typed_at = Instant::now();
            vec![]
        }
        HitKind::EndTask => end_task(state),
        HitKind::MenuItem(_) | HitKind::MenuPanel => vec![],
        HitKind::Setting(opt, v) => {
            if let Some((group, row, chip)) = find_setting_hit(state, opt, v) {
                state.kb = KbFocus::Setting { group, row, chip };
            }
            set_option(state, opt, v)
        }
        HitKind::Zoom(delta) => {
            if let Some(row) = state
                .kb_settings
                .iter()
                .find(|r| matches!(r.ctl, KbSettingCtl::Scale))
            {
                state.kb = KbFocus::Setting {
                    group: row.group,
                    row: row.row,
                    chip: 0,
                };
            }
            zoom(state, delta as i32)
        }
        HitKind::ResetSettings => {
            if let Some(row) = state
                .kb_settings
                .iter()
                .find(|r| matches!(r.ctl, KbSettingCtl::Reset))
            {
                state.kb = KbFocus::Setting {
                    group: row.group,
                    row: row.row,
                    chip: 0,
                };
            }
            reset_settings(state)
        }
        HitKind::Group(id) => {
            if !state.open_groups.insert(id) {
                state.open_groups.remove(&id);
            }
            state.kb = KbFocus::ProcGroup(id);
            // Visible indices shift; drop pins and re-capture after the next layout.
            clear_pins(state, true);
            vec![]
        }
        HitKind::Proc { pid } => {
            select_proc(state, pid, ctrl, shift);
            state.kb = KbFocus::Proc(pid);
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
        HitKind::Startup(index) => {
            state.kb = KbFocus::Startup(index);
            vec![Effect::FlipStartup(index)]
        }
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
        HitKind::Keys => {
            open_keys(state);
            vec![]
        }
        HitKind::KeysPanel | HitKind::KeysBackdrop => vec![],
    }
}

fn open_keys(state: &mut AppState) {
    close_menu(state);
    state.search_focused = false;
    state.keys_open = true;
    state.keys_scroll = 0.0;
}

/// The sheet is modal: it scrolls and closes, and swallows everything else.
fn keys_key(state: &mut AppState, key: KeyIn) -> Vec<Effect> {
    match key {
        KeyIn::Escape | KeyIn::Help | KeyIn::Char('?') => state.keys_open = false,
        KeyIn::Up => state.keys_scroll -= 40.0,
        KeyIn::Down => state.keys_scroll += 40.0,
        KeyIn::PageUp => state.keys_scroll -= 240.0,
        KeyIn::PageDown => state.keys_scroll += 240.0,
        KeyIn::Home => state.keys_scroll = 0.0,
        KeyIn::End => state.keys_scroll = f32::MAX,
        _ => {}
    }
    state.keys_scroll = state.keys_scroll.max(0.0);
    vec![]
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

/// Scroll whichever pane the pointer is over.
pub fn on_wheel(state: &mut AppState, over: Option<ScrollBar>, dy: f32) {
    if state.keys_open {
        state.keys_scroll = (state.keys_scroll + dy).max(0.0);
        return;
    }
    if state.menu.is_some() {
        close_menu(state);
        return;
    }
    if let Some(which) = over {
        let v = get_scroll(state, which) + dy;
        set_scroll(state, which, v);
    }
}

/// `repeat` is true for auto-repeat while a key is held. List focus wraps
/// around only on a fresh press, so holding an arrow stops at the end.
pub fn on_key(
    state: &mut AppState,
    key: KeyIn,
    ctrl: bool,
    shift: bool,
    repeat: bool,
) -> Vec<Effect> {
    if state.keys_open {
        return keys_key(state, key);
    }
    if state.menu.is_some() {
        return menu_key(state, key);
    }
    let wrap = !repeat && !shift;
    if matches!(key, KeyIn::Help) || (matches!(key, KeyIn::Char('?')) && !state.search_focused) {
        open_keys(state);
        return vec![];
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
                state.page = Page::Processes;
                state.search_focused = true;
                state.kb = KbFocus::None;
                return vec![Effect::Persist];
            }
            if ctrl && c == ',' {
                state.page = Page::Settings;
                state.search_focused = false;
                state.kb = KbFocus::None;
                return vec![Effect::Persist];
            }
            if ctrl && (c == 'a' || c == 'A') && !state.search_focused {
                return select_all_visible(state);
            }
            if ctrl && c == ' ' && state.page == Page::Processes && !state.search_focused {
                toggle_focused_proc(state);
                return vec![];
            }
            if ctrl {
                return vec![];
            }
            if !state.search_focused {
                match c {
                    '1' => return go_page(state, Page::Processes),
                    '2' => return go_page(state, Page::Performance),
                    '3' => return go_page(state, Page::Startup),
                    '4' => return go_page(state, Page::Settings),
                    ' ' if state.page == Page::Performance => {
                        state.paused = !state.paused;
                        return vec![Effect::Persist];
                    }
                    ' ' if state.page == Page::Startup => {
                        return activate_startup(state);
                    }
                    ' ' if state.page == Page::Settings => {
                        return activate_setting(state);
                    }
                    ' ' if state.page == Page::Processes => {
                        if matches!(state.kb, KbFocus::ProcGroup(_)) {
                            toggle_focused_group(state);
                            return vec![];
                        }
                        // Hold freezes the list. Repeats must not recapture, or a
                        // process that exited mid-hold would lock in the gap.
                        if state.held.is_none() && !state.visible_pids.is_empty() {
                            state.held = Some(state.visible_pids.clone());
                        }
                        return vec![];
                    }
                    'c' | 'C' if state.page == Page::Performance => {
                        return go_section(state, Section::Cpu);
                    }
                    'm' | 'M' if state.page == Page::Performance => {
                        return go_section(state, Section::Memory);
                    }
                    'g' | 'G' if state.page == Page::Performance => {
                        return go_section(state, Section::Gpu);
                    }
                    'd' | 'D' if state.page == Page::Performance => {
                        return go_section(state, Section::Disk);
                    }
                    'n' | 'N' if state.page == Page::Performance => {
                        return go_section(state, Section::Net);
                    }
                    'v' | 'V' if state.page == Page::Processes => {
                        return cycle_view(state);
                    }
                    _ => return vec![],
                }
            }
            if !c.is_control() {
                state.query.push(c);
                state.scroll = 0.0;
                state.typed_at = Instant::now();
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
                state.typed_at = Instant::now();
            }
            vec![]
        }
        KeyIn::Escape => {
            if state.search_focused {
                state.query.clear();
                state.search_focused = false;
                state.scroll = 0.0;
            } else if state.page == Page::Processes {
                clear_selection(state);
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
                return end_task(state);
            }
            match state.page {
                Page::Processes if !state.search_focused => {
                    if matches!(state.kb, KbFocus::ProcGroup(_)) {
                        toggle_focused_group(state);
                    }
                    // Process rows: selection is already on the focused row.
                    // Context menu is Shift+F10 / Menu — Enter does not open it.
                    vec![]
                }
                Page::Startup => activate_startup(state),
                Page::Settings => activate_setting(state),
                _ => {
                    state.search_focused = false;
                    vec![]
                }
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
        KeyIn::Home | KeyIn::End | KeyIn::Up | KeyIn::Down => {
            if state.search_focused {
                state.search_focused = false;
            }
            let dir = match key {
                KeyIn::Up => -1,
                KeyIn::Down => 1,
                KeyIn::Home => i32::MIN,
                KeyIn::End => i32::MAX,
                _ => 0,
            };
            match state.page {
                Page::Processes => {
                    move_proc_focus(state, dir, ctrl, shift, wrap);
                    vec![]
                }
                Page::Startup => {
                    move_startup_focus(state, dir, wrap);
                    vec![]
                }
                Page::Settings => {
                    move_setting_focus(state, dir, wrap);
                    vec![]
                }
                Page::Performance if matches!(key, KeyIn::Home | KeyIn::End) => {
                    if matches!(key, KeyIn::Home) {
                        state.perf_scroll = 0.0;
                    } else {
                        nudge_scroll(state, 1.0e6);
                    }
                    vec![]
                }
                _ => vec![],
            }
        }
        KeyIn::Left | KeyIn::Right => {
            if state.search_focused {
                return vec![];
            }
            let right = matches!(key, KeyIn::Right);
            match state.page {
                Page::Processes => {
                    set_group_open(state, right);
                    vec![]
                }
                Page::Performance => cycle_section(state, right),
                Page::Settings => cycle_setting(state, right),
                _ => vec![],
            }
        }
        KeyIn::Menu => {
            if state.page == Page::Processes && !state.search_focused {
                open_menu_keyboard(state)
            } else {
                vec![]
            }
        }
        KeyIn::Help => vec![],
    }
}

/// Next list index for a focus step. `dir` is -1 / +1, or `i32::MIN` /
/// `i32::MAX` for Home / End. With `wrap`, stepping past either end lands on
/// the other.
fn step_index(cur: usize, last: usize, dir: i32, wrap: bool) -> usize {
    match dir {
        i32::MIN => 0,
        i32::MAX => last,
        d => {
            let next = cur as i32 + d;
            if next < 0 {
                if wrap {
                    last
                } else {
                    0
                }
            } else if next > last as i32 {
                if wrap {
                    0
                } else {
                    last
                }
            } else {
                next as usize
            }
        }
    }
}

fn go_page(state: &mut AppState, page: Page) -> Vec<Effect> {
    state.page = page;
    state.search_focused = false;
    state.kb = KbFocus::None;
    vec![Effect::Persist]
}

fn go_section(state: &mut AppState, section: Section) -> Vec<Effect> {
    state.section = section;
    state.perf_scroll = 0.0;
    vec![Effect::Persist]
}

fn cycle_section(state: &mut AppState, right: bool) -> Vec<Effect> {
    let i = Section::ALL
        .iter()
        .position(|s| *s == state.section)
        .unwrap_or(0);
    let n = Section::ALL.len();
    let next = if right { (i + 1) % n } else { (i + n - 1) % n };
    go_section(state, Section::ALL[next])
}

fn cycle_view(state: &mut AppState) -> Vec<Effect> {
    let i = ProcView::ALL
        .iter()
        .position(|v| *v == state.view)
        .unwrap_or(0);
    let next = ProcView::ALL[(i + 1) % ProcView::ALL.len()];
    state.view = next;
    state.scroll = 0.0;
    clear_pins(state, true);
    state.kb = KbFocus::None;
    vec![Effect::Persist]
}

fn select_all_visible(state: &mut AppState) -> Vec<Effect> {
    if state.page != Page::Processes || state.visible_pids.is_empty() {
        return vec![];
    }
    state.selected.clear();
    state.selected.extend(state.visible_pids.iter().copied());
    state.anchor = state.visible_pids.first().copied();
    // Keep the sorted order stable under the keyboard selection.
    clear_pins(state, false);
    if let Some(&pid) = state.visible_pids.first() {
        state.kb = KbFocus::Proc(pid);
    }
    vec![]
}

fn proc_focus_index(state: &AppState) -> Option<usize> {
    match state.kb {
        KbFocus::ProcGroup(id) => state
            .kb_proc_rows
            .iter()
            .position(|r| matches!(r, KbProcRow::Group { id: gid } if *gid == id)),
        KbFocus::Proc(pid) => state
            .kb_proc_rows
            .iter()
            .position(|r| matches!(r, KbProcRow::Pid { pid: p, .. } if *p == pid)),
        _ => None,
    }
}

fn move_proc_focus(state: &mut AppState, dir: i32, ctrl: bool, shift: bool, wrap: bool) {
    let n = state.kb_proc_rows.len();
    if n == 0 {
        return;
    }
    let next = match proc_focus_index(state) {
        Some(i) => step_index(i, n - 1, dir, wrap),
        None if dir == i32::MIN || dir == i32::MAX => step_index(0, n - 1, dir, false),
        None => preferred_entry_index(&state.kb_proc_rows, dir),
    };
    apply_proc_focus(state, next, ctrl, shift);
}

/// First arrow from no focus prefers a process row so Grouped view does not
/// land on a collapsed header with nothing selected.
fn preferred_entry_index(rows: &[KbProcRow], dir: i32) -> usize {
    if dir > 0 {
        rows.iter()
            .position(|r| matches!(r, KbProcRow::Pid { .. }))
            .unwrap_or(0)
    } else {
        rows.iter()
            .rposition(|r| matches!(r, KbProcRow::Pid { .. }))
            .unwrap_or(rows.len().saturating_sub(1))
    }
}

fn apply_proc_focus(state: &mut AppState, idx: usize, ctrl: bool, shift: bool) {
    let Some(row) = state.kb_proc_rows.get(idx).copied() else {
        return;
    };
    match row {
        KbProcRow::Group { id } => {
            state.kb = KbFocus::ProcGroup(id);
            state.scroll_into_view = Some(ScrollTarget::ProcGroup(id));
            if !ctrl && !shift {
                state.selected.clear();
                clear_pins(state, false);
                state.anchor = None;
                state.armed = None;
            }
        }
        KbProcRow::Pid { pid, .. } => {
            state.kb = KbFocus::Proc(pid);
            state.scroll_into_view = Some(ScrollTarget::Proc(pid));
            if !ctrl {
                // Keyboard selection must not pin: pinning every arrow step
                // makes the list leap under the cursor as the old row unpins.
                select_proc_keyboard(state, pid, shift);
            }
        }
    }
}

fn toggle_focused_proc(state: &mut AppState) {
    let KbFocus::Proc(pid) = state.kb else {
        return;
    };
    select_proc(state, pid, true, false);
    clear_pins(state, false);
}

fn focused_group_id(state: &AppState) -> Option<u64> {
    match state.kb {
        KbFocus::ProcGroup(id) => Some(id),
        KbFocus::Proc(pid) => state.kb_proc_rows.iter().find_map(|r| match r {
            KbProcRow::Pid {
                pid: p,
                group: Some(g),
            } if *p == pid => Some(*g),
            _ => None,
        }),
        _ => None,
    }
}

fn toggle_focused_group(state: &mut AppState) {
    let Some(id) = focused_group_id(state) else {
        return;
    };
    if !state.open_groups.insert(id) {
        state.open_groups.remove(&id);
    }
    state.kb = KbFocus::ProcGroup(id);
    clear_pins(state, true);
    state.scroll_into_view = Some(ScrollTarget::ProcGroup(id));
}

fn set_group_open(state: &mut AppState, open: bool) {
    let Some(id) = focused_group_id(state) else {
        return;
    };
    if open {
        state.open_groups.insert(id);
    } else {
        state.open_groups.remove(&id);
        state.kb = KbFocus::ProcGroup(id);
    }
    clear_pins(state, true);
    state.scroll_into_view = Some(ScrollTarget::ProcGroup(id));
}

fn open_menu_keyboard(state: &mut AppState) -> Vec<Effect> {
    let pid = match state.kb {
        KbFocus::Proc(pid) => pid,
        _ => match state.selected.iter().next().copied() {
            Some(pid) => pid,
            None => return vec![],
        },
    };
    let pos = state
        .menu_anchor
        .unwrap_or([state.nav_w + 80.0, state.height * 0.4]);
    open_menu(state, pid, pos);
    vec![]
}

fn move_startup_focus(state: &mut AppState, dir: i32, wrap: bool) {
    let n = state.kb_startup_len;
    if n == 0 {
        return;
    }
    let last = n - 1;
    let next = match state.kb {
        KbFocus::Startup(i) => step_index(i.min(last), last, dir, wrap),
        _ if dir > 0 && dir != i32::MAX => 0,
        _ if dir == i32::MIN => 0,
        _ => last,
    };
    state.kb = KbFocus::Startup(next);
    state.scroll_into_view = Some(ScrollTarget::Startup(next));
}

fn activate_startup(state: &mut AppState) -> Vec<Effect> {
    let idx = match state.kb {
        KbFocus::Startup(i) => i,
        _ => {
            if state.kb_startup_len == 0 {
                return vec![];
            }
            state.kb = KbFocus::Startup(0);
            state.scroll_into_view = Some(ScrollTarget::Startup(0));
            0
        }
    };
    if idx >= state.kb_startup_len {
        return vec![];
    }
    vec![Effect::FlipStartup(idx)]
}

fn setting_flat_index(state: &AppState, group: usize, row: usize) -> Option<usize> {
    state
        .kb_settings
        .iter()
        .position(|r| r.group == group && r.row == row)
}

fn move_setting_focus(state: &mut AppState, dir: i32, wrap: bool) {
    let n = state.kb_settings.len();
    if n == 0 {
        return;
    }
    let last = n - 1;
    let cur = match state.kb {
        KbFocus::Setting { group, row, .. } => setting_flat_index(state, group, row),
        _ => None,
    };
    let next = match cur {
        Some(i) => step_index(i, last, dir, wrap),
        None if dir > 0 && dir != i32::MAX => 0,
        None if dir == i32::MIN => 0,
        None => last,
    };
    let row = &state.kb_settings[next];
    state.kb = KbFocus::Setting {
        group: row.group,
        row: row.row,
        chip: 0,
    };
    state.scroll_into_view = Some(ScrollTarget::Setting {
        group: row.group,
        row: row.row,
    });
}

fn focused_setting(state: &AppState) -> Option<&KbSettingRow> {
    match state.kb {
        KbFocus::Setting { group, row, .. } => state
            .kb_settings
            .iter()
            .find(|r| r.group == group && r.row == row),
        _ => None,
    }
}

fn cycle_setting(state: &mut AppState, right: bool) -> Vec<Effect> {
    if focused_setting(state).is_none() {
        move_setting_focus(state, 1, false);
    }
    let Some(focus) = focused_setting(state).cloned() else {
        return vec![];
    };
    match focus.ctl {
        KbSettingCtl::Choice {
            opt,
            count,
            current,
        } => {
            if count == 0 {
                return vec![];
            }
            let next = if right {
                (current + 1) % count
            } else {
                (current + count - 1) % count
            };
            set_option(state, opt, next)
        }
        KbSettingCtl::Switch { opt, on } => set_option(state, opt, (!on) as u8),
        KbSettingCtl::Chips(chips) => {
            if chips.is_empty() {
                return vec![];
            }
            let KbFocus::Setting { group, row, chip } = state.kb else {
                return vec![];
            };
            let n = chips.len();
            let next = if right {
                (chip + 1) % n
            } else {
                (chip + n - 1) % n
            };
            state.kb = KbFocus::Setting {
                group,
                row,
                chip: next,
            };
            vec![]
        }
        KbSettingCtl::Scale => zoom(state, if right { 1 } else { -1 }),
        KbSettingCtl::Reset => vec![],
    }
}

fn activate_setting(state: &mut AppState) -> Vec<Effect> {
    if focused_setting(state).is_none() {
        move_setting_focus(state, 1, false);
    }
    let Some(focus) = focused_setting(state).cloned() else {
        return vec![];
    };
    match focus.ctl {
        KbSettingCtl::Choice {
            opt,
            count,
            current,
        } => {
            if count == 0 {
                return vec![];
            }
            set_option(state, opt, (current + 1) % count)
        }
        KbSettingCtl::Switch { opt, on } => set_option(state, opt, (!on) as u8),
        KbSettingCtl::Chips(chips) => {
            let chip = match state.kb {
                KbFocus::Setting { chip, .. } => chip,
                _ => 0,
            };
            let Some((opt, on)) = chips.get(chip).copied() else {
                return vec![];
            };
            set_option(state, opt, (!on) as u8)
        }
        KbSettingCtl::Scale => vec![],
        KbSettingCtl::Reset => reset_settings(state),
    }
}

fn find_setting_hit(state: &AppState, opt: Opt, v: u8) -> Option<(usize, usize, usize)> {
    for row in &state.kb_settings {
        match &row.ctl {
            KbSettingCtl::Choice {
                opt: o, current, ..
            } if *o == opt && *current == v => {
                return Some((row.group, row.row, 0));
            }
            KbSettingCtl::Choice { opt: o, .. } if *o == opt => {
                return Some((row.group, row.row, 0));
            }
            KbSettingCtl::Switch { opt: o, .. } if *o == opt => {
                return Some((row.group, row.row, 0));
            }
            KbSettingCtl::Chips(chips) => {
                if let Some(i) = chips.iter().position(|(o, _)| *o == opt) {
                    return Some((row.group, row.row, i));
                }
            }
            _ => {}
        }
    }
    let _ = v;
    None
}

/// Space went up, or the window lost focus. The list sorts again.
pub fn end_hold(state: &mut AppState) -> bool {
    state.held.take().is_some()
}

/// Open the process menu at the pointer. A row outside the selection becomes
/// the selection first, so the menu always acts on what is highlighted.
pub fn open_menu(state: &mut AppState, pid: i32, mouse: [f32; 2]) {
    if !state.selected.contains(&pid) {
        select_proc(state, pid, false, false);
    }
    state.armed = None;
    state.search_focused = false;
    // A menu already open fades out where it was while the new one appears.
    close_menu(state);
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
    let menu = state.menu.take();
    if state.settings.animations && menu.is_some() {
        state.menu_ghost = menu;
    }
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
    if action == MenuAction::ForceKill && !menu.confirm_kill && state.settings.confirm {
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

/// Apply a Settings page control. `v` is a segment index, or 0 / 1 for a switch.
fn set_option(state: &mut AppState, opt: Opt, v: u8) -> Vec<Effect> {
    fn pick<T: Choice>(slot: &mut T, v: u8) {
        if let Some(x) = T::from_index(v as usize) {
            *slot = x;
        }
    }
    let on = v != 0;
    let s = &mut state.settings;
    match opt {
        Opt::Glass => pick(&mut s.glass, v),
        Opt::Animations => s.animations = on,
        Opt::Motion => pick(&mut s.motion, v),
        Opt::Density => {
            state.density = if on {
                Density::Compact
            } else {
                Density::Comfortable
            }
        }
        Opt::Heat => s.heat = on,
        Opt::Readout => s.readout = on,
        Opt::History => pick(&mut s.history, v),
        Opt::Curve => pick(&mut s.curve, v),
        Opt::Fill => s.fill = on,
        Opt::Grid => s.grid = on,
        Opt::Speed => {
            // The trailing segment is Pause; picking a speed also resumes.
            if (v as usize) < Speed::ALL.len() {
                pick(&mut s.speed, v);
                state.paused = false;
            } else {
                state.paused = !state.paused;
            }
        }
        Opt::ProcCpu => pick(&mut s.proc_cpu, v),
        Opt::Units => pick(&mut s.units, v),
        Opt::Temp => pick(&mut s.temp, v),
        Opt::Column(col) => {
            if let Some(slot) = s.show_mut(col) {
                *slot = on;
            }
        }
        Opt::ListAnimations => s.list_animations = on,
        Opt::Confirm => {
            s.confirm = on;
            state.armed = None;
        }
        Opt::OpenOn => pick(&mut s.open_on, v),
    }
    if !state.settings.animations {
        state.menu_ghost = None;
    }
    state.reset_armed = None;
    vec![Effect::Persist]
}

/// First click arms for a few seconds; the second restores every default.
fn reset_settings(state: &mut AppState) -> Vec<Effect> {
    let now = Instant::now();
    if !state.reset_armed.is_some_and(|t| t > now) {
        state.reset_armed = Some(now + Duration::from_secs(4));
        return vec![];
    }
    state.reset_armed = None;
    state.settings = Settings::default();
    state.density = Density::Comfortable;
    state.ui_scale = 1.0;
    state.paused = false;
    vec![
        Effect::Persist,
        Effect::Notify("Settings reset to defaults".into()),
    ]
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
        Page::Settings => state.settings_scroll = (state.settings_scroll + dy).max(0.0),
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

/// Keyboard selection: same as [`select_proc`] but never pins rows, so arrowing
/// through the list does not make each previous row leap back to sort order.
fn select_proc_keyboard(state: &mut AppState, pid: i32, shift: bool) {
    if shift {
        if let Some(anchor) = state.anchor {
            if let (Some(a), Some(b)) = (
                state.visible_pids.iter().position(|p| *p == anchor),
                state.visible_pids.iter().position(|p| *p == pid),
            ) {
                let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
                state.selected.clear();
                for p in &state.visible_pids[lo..=hi] {
                    state.selected.insert(*p);
                }
                clear_pins(state, false);
                return;
            }
        }
    }
    state.selected.clear();
    state.selected.insert(pid);
    state.anchor = Some(pid);
    clear_pins(state, false);
}

pub fn clear_selection(state: &mut AppState) {
    state.selected.clear();
    clear_pins(state, false);
    state.anchor = None;
    state.armed = None;
    if matches!(state.kb, KbFocus::Proc(_) | KbFocus::ProcGroup(_)) {
        state.kb = KbFocus::None;
    }
}

/// Drop row pins. When `repin`, the next frame freezes the current selection
/// at its new indices (after a view or group change). Keyboard navigation
/// clears pins without repinning so the list does not jump under the cursor.
fn clear_pins(state: &mut AppState, repin: bool) {
    state.pinned.clear();
    state.needs_repin = repin && !state.selected.is_empty();
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
        ScrollBar::Settings => state.settings_scroll,
    }
}

fn set_scroll(state: &mut AppState, which: ScrollBar, v: f32) {
    match which {
        ScrollBar::Processes => state.scroll = v.max(0.0),
        ScrollBar::Performance => state.perf_scroll = v.max(0.0),
        ScrollBar::Startup => state.startup_scroll = v.max(0.0),
        ScrollBar::Settings => state.settings_scroll = v.max(0.0),
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
    if !state.settings.confirm {
        state.armed = None;
        return vec![Effect::Signal(
            state.selected.iter().copied().collect(),
            Sig::Term,
        )];
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
        assert!(state.menu_ghost.is_some(), "a closed menu fades out");
    }

    #[test]
    fn closed_menu_leaves_no_ghost_with_animations_off() {
        let mut state = menu_state();
        state.settings.animations = false;
        open_menu(&mut state, 10, [0.0, 0.0]);
        close_menu(&mut state);
        assert!(state.menu.is_none() && state.menu_ghost.is_none());
    }

    #[test]
    fn expired_notice_lingers_for_its_fade() {
        let mut state = menu_state();
        let now = Instant::now();
        state.notice = Some(crate::model::Notice {
            until: now - Duration::from_millis(100),
            label: "x".into(),
        });
        expire(&mut state);
        assert!(state.notice.is_some());
        state.notice.as_mut().unwrap().until = now - Duration::from_secs(1);
        expire(&mut state);
        assert!(state.notice.is_none());
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
    fn confirm_off_ends_on_the_first_click() {
        let mut state = menu_state();
        state.settings.confirm = false;
        state.selected.insert(20);
        let fx = on_press(&mut state, HitKind::EndTask, false, false, [0.0, 0.0]);
        assert!(matches!(fx.as_slice(), [Effect::Signal(p, Sig::Term)] if p == &vec![20]));
        open_menu(&mut state, 10, [0.0, 0.0]);
        let kill = HitKind::MenuItem(MenuAction::ForceKill);
        let fx = on_press(&mut state, kill, false, false, [0.0, 0.0]);
        assert!(matches!(fx.as_slice(), [Effect::Signal(_, Sig::Kill)]));
    }

    #[test]
    fn typing_does_not_focus_search_on_its_own() {
        let mut state = menu_state();
        state.page = Page::Processes;
        state.visible_pids = vec![10];
        on_key(&mut state, KeyIn::Char('a'), false, false, false);
        on_key(&mut state, KeyIn::Char(' '), false, false, false);
        assert!(!state.search_focused);
        assert!(state.query.is_empty());
        assert!(state.held.is_some(), "space still freezes the list");
    }

    #[test]
    fn space_in_search_types_instead_of_holding() {
        let mut state = menu_state();
        state.page = Page::Processes;
        state.visible_pids = vec![10, 20];
        state.search_focused = true;
        on_key(&mut state, KeyIn::Char(' '), false, false, false);
        assert!(state.held.is_none());
        assert_eq!(state.query, " ");
    }

    #[test]
    fn pause_segment_toggles_and_a_speed_resumes() {
        let mut state = menu_state();
        let pause = HitKind::Setting(Opt::Speed, Speed::ALL.len() as u8);
        on_press(&mut state, pause, false, false, [0.0, 0.0]);
        assert!(state.paused);
        on_press(
            &mut state,
            HitKind::Setting(Opt::Speed, 2),
            false,
            false,
            [0.0, 0.0],
        );
        assert!(!state.paused);
        assert_eq!(state.settings.speed, Speed::Slow);
    }

    #[test]
    fn reset_needs_a_second_click() {
        let mut state = menu_state();
        state.settings.fill = false;
        state.ui_scale = 1.4;
        assert!(on_press(&mut state, HitKind::ResetSettings, false, false, [0.0, 0.0]).is_empty());
        assert!(!state.settings.fill);
        on_press(&mut state, HitKind::ResetSettings, false, false, [0.0, 0.0]);
        assert_eq!(state.settings, Settings::default());
        assert!((state.ui_scale - 1.0).abs() < 0.001);
    }

    #[test]
    fn keyboard_walks_items_and_activates() {
        let mut state = menu_state();
        open_menu(&mut state, 10, [0.0, 0.0]);
        state.menu.as_mut().unwrap().items = vec![MenuAction::EndTask, MenuAction::CopyPid];
        on_key(&mut state, KeyIn::Down, false, false, false);
        on_key(&mut state, KeyIn::Down, false, false, false);
        let fx = on_key(&mut state, KeyIn::Enter, false, false, false);
        assert!(matches!(fx.as_slice(), [Effect::Copy(t)] if t == "10"));
    }

    #[test]
    fn arrows_select_processes_and_shift_extends() {
        let mut state = menu_state();
        state.page = Page::Processes;
        state.kb_proc_rows = vec![
            KbProcRow::Pid {
                pid: 10,
                group: None,
            },
            KbProcRow::Pid {
                pid: 20,
                group: None,
            },
            KbProcRow::Pid {
                pid: 30,
                group: None,
            },
        ];
        on_key(&mut state, KeyIn::Down, false, false, false);
        assert_eq!(state.kb, KbFocus::Proc(10));
        assert!(state.selected.contains(&10) && state.selected.len() == 1);
        assert!(state.pinned.is_empty(), "keyboard select must not pin rows");
        on_key(&mut state, KeyIn::Down, false, true, false);
        assert_eq!(state.kb, KbFocus::Proc(20));
        assert!(state.selected.contains(&10) && state.selected.contains(&20));
        assert!(state.pinned.is_empty());
        on_key(&mut state, KeyIn::Down, true, false, false);
        assert_eq!(state.kb, KbFocus::Proc(30));
        assert!(!state.selected.contains(&30), "ctrl moves focus only");
    }

    fn flat_rows(pids: &[i32]) -> Vec<KbProcRow> {
        pids.iter()
            .map(|&pid| KbProcRow::Pid { pid, group: None })
            .collect()
    }

    #[test]
    fn arrows_wrap_at_the_ends_of_the_list() {
        let mut state = menu_state();
        state.page = Page::Processes;
        state.kb_proc_rows = flat_rows(&[10, 20, 30]);
        state.kb = KbFocus::Proc(10);
        on_key(&mut state, KeyIn::Up, false, false, false);
        assert_eq!(state.kb, KbFocus::Proc(30));
        assert!(state.selected.contains(&30) && state.selected.len() == 1);
        on_key(&mut state, KeyIn::Down, false, false, false);
        assert_eq!(state.kb, KbFocus::Proc(10));
    }

    #[test]
    fn held_arrow_stops_at_the_end_and_shift_never_wraps() {
        let mut state = menu_state();
        state.page = Page::Processes;
        state.kb_proc_rows = flat_rows(&[10, 20, 30]);
        state.kb = KbFocus::Proc(30);
        on_key(&mut state, KeyIn::Down, false, false, true);
        assert_eq!(state.kb, KbFocus::Proc(30), "auto-repeat stops at the edge");
        state.kb = KbFocus::Proc(10);
        on_key(&mut state, KeyIn::Up, false, true, false);
        assert_eq!(state.kb, KbFocus::Proc(10), "range select does not wrap");
    }

    #[test]
    fn startup_and_settings_wrap_too() {
        let mut state = menu_state();
        state.page = Page::Startup;
        state.kb_startup_len = 3;
        state.kb = KbFocus::Startup(2);
        on_key(&mut state, KeyIn::Down, false, false, false);
        assert_eq!(state.kb, KbFocus::Startup(0));

        state.page = Page::Settings;
        state.kb_settings = (0..2)
            .map(|row| KbSettingRow {
                group: 0,
                row,
                ctl: KbSettingCtl::Reset,
            })
            .collect();
        state.kb = KbFocus::Setting {
            group: 0,
            row: 0,
            chip: 0,
        };
        on_key(&mut state, KeyIn::Up, false, false, false);
        assert_eq!(
            state.kb,
            KbFocus::Setting {
                group: 0,
                row: 1,
                chip: 0
            }
        );
    }

    #[test]
    fn question_mark_opens_the_sheet_and_it_is_modal() {
        let mut state = menu_state();
        state.page = Page::Processes;
        on_key(&mut state, KeyIn::Char('?'), false, true, false);
        assert!(state.keys_open);
        on_key(&mut state, KeyIn::Char('2'), false, false, false);
        assert_eq!(
            state.page,
            Page::Processes,
            "keys under the sheet are swallowed"
        );
        on_key(&mut state, KeyIn::Escape, false, false, false);
        assert!(!state.keys_open);
        on_key(&mut state, KeyIn::Help, false, false, false);
        assert!(state.keys_open);
        on_press(&mut state, HitKind::KeysPanel, false, false, [0.0, 0.0]);
        assert!(state.keys_open, "clicks on the sheet keep it open");
        on_press(&mut state, HitKind::KeysBackdrop, false, false, [0.0, 0.0]);
        assert!(!state.keys_open);
    }

    #[test]
    fn question_mark_types_into_search() {
        let mut state = menu_state();
        state.page = Page::Processes;
        state.search_focused = true;
        on_key(&mut state, KeyIn::Char('?'), false, true, false);
        assert!(!state.keys_open);
        assert_eq!(state.query, "?");
    }

    #[test]
    fn first_arrow_prefers_a_process_over_a_group_header() {
        let mut state = menu_state();
        state.page = Page::Processes;
        state.kb_proc_rows = vec![
            KbProcRow::Group { id: 1 },
            KbProcRow::Group { id: 2 },
            KbProcRow::Pid {
                pid: 10,
                group: None,
            },
        ];
        on_key(&mut state, KeyIn::Down, false, false, false);
        assert_eq!(state.kb, KbFocus::Proc(10));
        assert!(state.selected.contains(&10));
    }

    #[test]
    fn settings_space_activates_on_first_press() {
        let mut state = menu_state();
        state.page = Page::Settings;
        state.kb_settings = vec![KbSettingRow {
            group: 0,
            row: 0,
            ctl: KbSettingCtl::Switch {
                opt: Opt::Heat,
                on: true,
            },
        }];
        assert!(state.settings.heat);
        on_key(&mut state, KeyIn::Char(' '), false, false, false);
        assert!(!state.settings.heat);
        assert_eq!(
            state.kb,
            KbFocus::Setting {
                group: 0,
                row: 0,
                chip: 0
            }
        );
    }

    #[test]
    fn left_right_toggle_process_groups() {
        let mut state = menu_state();
        state.page = Page::Processes;
        state.kb_proc_rows = vec![
            KbProcRow::Group { id: 7 },
            KbProcRow::Pid {
                pid: 10,
                group: Some(7),
            },
        ];
        state.open_groups.insert(7);
        state.kb = KbFocus::Proc(10);
        on_key(&mut state, KeyIn::Left, false, false, false);
        assert!(!state.open_groups.contains(&7));
        assert_eq!(state.kb, KbFocus::ProcGroup(7));
        on_key(&mut state, KeyIn::Right, false, false, false);
        assert!(state.open_groups.contains(&7));
    }

    #[test]
    fn ctrl_a_selects_every_visible_process() {
        let mut state = menu_state();
        state.page = Page::Processes;
        on_key(&mut state, KeyIn::Char('a'), true, false, false);
        assert_eq!(state.selected.len(), 3);
        assert!(state.selected.contains(&10));
        assert!(state.selected.contains(&30));
    }

    #[test]
    fn performance_arrows_cycle_sections() {
        let mut state = menu_state();
        state.page = Page::Performance;
        state.section = Section::Cpu;
        on_key(&mut state, KeyIn::Right, false, false, false);
        assert_eq!(state.section, Section::Memory);
        on_key(&mut state, KeyIn::Left, false, false, false);
        assert_eq!(state.section, Section::Cpu);
    }

    #[test]
    fn startup_space_flips_focused_entry() {
        let mut state = menu_state();
        state.page = Page::Startup;
        state.kb_startup_len = 3;
        state.kb = KbFocus::Startup(1);
        let fx = on_key(&mut state, KeyIn::Char(' '), false, false, false);
        assert!(matches!(fx.as_slice(), [Effect::FlipStartup(1)]));
    }

    #[test]
    fn settings_arrows_cycle_a_choice() {
        let mut state = menu_state();
        state.page = Page::Settings;
        state.kb_settings = vec![KbSettingRow {
            group: 0,
            row: 0,
            ctl: KbSettingCtl::Choice {
                opt: Opt::Glass,
                count: 3,
                current: 1,
            },
        }];
        state.kb = KbFocus::Setting {
            group: 0,
            row: 0,
            chip: 0,
        };
        let fx = on_key(&mut state, KeyIn::Right, false, false, false);
        assert!(fx.iter().any(|e| matches!(e, Effect::Persist)));
        assert_eq!(state.settings.glass, crate::settings::Glass::Solid);
    }
}
