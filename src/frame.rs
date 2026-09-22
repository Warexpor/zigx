use std::time::Instant;

use crate::format::{
    self, bytes, cpu_pct, disk_cell, duration, fit, freq_ghz, percent, rate, text_width,
};
use crate::interact::selection_label;
use crate::model::{
    theme, AppState, Col, Density, Page, Proc, ProcView, Section, Snap, StartupEntry, HIST_CAP,
};

#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }
    pub fn contains(self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }
    pub fn right(self) -> f32 {
        self.x + self.w
    }
    pub fn bottom(self) -> f32 {
        self.y + self.h
    }
    pub fn inset(self, p: f32) -> Self {
        Self::new(
            self.x + p,
            self.y + p,
            (self.w - p * 2.0).max(0.0),
            (self.h - p * 2.0).max(0.0),
        )
    }
}

#[derive(Clone, Debug)]
pub struct Slab {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub radius: f32,
    pub fill: theme::Rgba,
    pub border: theme::Rgba,
    pub border_w: f32,
}

#[derive(Clone, Debug)]
pub struct Stroke {
    pub pts: Vec<[f32; 2]>,
    pub width: f32,
    pub color: theme::Rgba,
    pub baseline: Option<f32>,
}

#[derive(Clone, Debug)]
pub struct Label {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub size: f32,
    pub color: theme::Rgba,
    pub mono: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HitKind {
    DragWindow,
    Close,
    Minimize,
    ToggleTop,
    Page(Page),
    Section(Section),
    View(ProcView),
    Density,
    Sort(Col),
    Search,
    EndTask,
    Undo,
    Group(bool),
    Proc { pid: i32 },
    Startup(usize),
    DragNav,
    DragSub,
}

#[derive(Clone, Copy, Debug)]
pub struct Hit {
    pub rect: Rect,
    pub kind: HitKind,
}

pub struct DrawList {
    pub slabs: Vec<Slab>,
    pub strokes: Vec<Stroke>,
    pub labels: Vec<Label>,
    pub hits: Vec<Hit>,
    pub list_rect: Option<Rect>,
    pub detail_rect: Option<Rect>,
    pub startup_rect: Option<Rect>,
    clip: Option<Rect>,
}

impl DrawList {
    fn new() -> Self {
        Self {
            slabs: Vec::new(),
            strokes: Vec::new(),
            labels: Vec::new(),
            hits: Vec::new(),
            list_rect: None,
            detail_rect: None,
            startup_rect: None,
            clip: None,
        }
    }

    fn visible(&self, r: Rect) -> bool {
        match self.clip {
            Some(c) => r.bottom() > c.y && r.y < c.bottom() && r.right() > c.x && r.x < c.right(),
            None => true,
        }
    }

    fn slab(
        &mut self,
        r: Rect,
        radius: f32,
        fill: theme::Rgba,
        border: theme::Rgba,
        border_w: f32,
    ) {
        if r.w < 1.0 || r.h < 1.0 || !self.visible(r) {
            return;
        }
        self.slabs.push(Slab {
            x: r.x,
            y: r.y,
            w: r.w,
            h: r.h,
            radius,
            fill,
            border,
            border_w,
        });
    }

    fn hit(&mut self, rect: Rect, kind: HitKind) {
        if self.visible(rect) {
            self.hits.push(Hit { rect, kind });
        }
    }

    fn text(
        &mut self,
        text: &str,
        r: Rect,
        size: f32,
        color: theme::Rgba,
        mono: bool,
        right: bool,
    ) {
        if !self.visible(r) {
            return;
        }
        let fitted = fit(text, r.w, size, mono);
        if fitted.is_empty() {
            return;
        }
        let tw = text_width(&fitted, size, mono);
        let lh = size * 1.35;
        let x = if right { r.right() - tw } else { r.x };
        let y = r.y + (r.h - lh) * 0.5;
        self.labels.push(Label {
            text: fitted,
            x,
            y,
            w: r.w,
            h: lh,
            size,
            color,
            mono,
        });
    }

    fn graph(&mut self, r: Rect, series: &[(&[f32], theme::Rgba)], max: f32) {
        if r.w < 4.0 || r.h < 4.0 || !self.visible(r) {
            return;
        }
        let max = max.max(0.001);
        for t in [0.25, 0.5, 0.75] {
            let y = r.bottom() - t * r.h;
            self.strokes.push(Stroke {
                pts: vec![[r.x, y], [r.right(), y]],
                width: 1.0,
                color: theme::GRID,
                baseline: None,
            });
        }
        for (values, color) in series {
            if values.len() < 2 {
                continue;
            }
            let n = values.len();
            let pts: Vec<[f32; 2]> = values
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    let x = r.x + r.w * (i as f32) / (n - 1) as f32;
                    let y = r.bottom() - (v / max).clamp(0.0, 1.0) * r.h;
                    [x, y]
                })
                .collect();
            self.strokes.push(Stroke {
                pts: pts.clone(),
                width: 1.6,
                color: *color,
                baseline: Some(r.bottom()),
            });
            let _ = HIST_CAP;
        }
    }
}

pub fn hit_at(hits: &[Hit], x: f32, y: f32) -> Option<HitKind> {
    hits.iter()
        .rev()
        .find(|h| h.rect.contains(x, y))
        .map(|h| h.kind)
}

pub fn build(
    state: &mut AppState,
    snap: &Snap,
    startup: &[StartupEntry],
    mouse: [f32; 2],
) -> DrawList {
    let mut d = DrawList::new();
    let w = state.width.max(420.0);
    let h = state.height.max(320.0);
    let m = 12.0;
    let gap = 10.0;
    let bar = Rect::new(m, m, w - m * 2.0, 46.0);
    d.slab(bar, 16.0, theme::WELL, theme::WELL_BORDER, 1.0);
    d.hit(bar, HitKind::DragWindow);
    d.text(
        "ZIGX",
        Rect::new(bar.x + 16.0, bar.y, 70.0, bar.h),
        16.0,
        theme::INK,
        true,
        false,
    );
    let stat = format!("{}   {}", cpu_pct(snap.cpu_total), mem_label(snap));
    d.text(
        &stat,
        Rect::new(bar.x + 92.0, bar.y, bar.w - 280.0, bar.h),
        13.0,
        theme::DIM,
        true,
        false,
    );
    let btn_w = 52.0;
    let btn_y = bar.y + 9.0;
    let btn_h = 28.0;
    let labels = ["TOP", "MIN", "CLOSE"];
    let kinds = [HitKind::ToggleTop, HitKind::Minimize, HitKind::Close];
    for (i, (label, kind)) in labels.iter().zip(kinds).enumerate() {
        let r = Rect::new(
            bar.right() - 16.0 - (3 - i) as f32 * (btn_w + 6.0),
            btn_y,
            btn_w,
            btn_h,
        );
        let hot = r.contains(mouse[0], mouse[1]);
        let on = matches!(kind, HitKind::ToggleTop) && state.always_on_top;
        d.slab(
            r,
            8.0,
            if on || hot {
                theme::PILL
            } else {
                [255, 255, 255, 10]
            },
            [0, 0, 0, 0],
            0.0,
        );
        d.text(
            label,
            r,
            11.0,
            if hot { theme::INK } else { theme::MUTED },
            true,
            false,
        );
        // Center the short labels.
        if let Some(last) = d.labels.last_mut() {
            let tw = text_width(&last.text, 11.0, true);
            last.x = r.x + (r.w - tw) * 0.5;
        }
        d.hit(r, kind);
    }

    let body_y = bar.bottom() + gap;
    let body_h = (h - m - body_y).max(80.0);
    let nav = Rect::new(m, body_y, state.nav_w, body_h);
    d.slab(nav, 16.0, theme::WELL, theme::WELL_BORDER, 1.0);
    nav_items(&mut d, state, nav, mouse, snap.sample_ms);

    let gap_nav = Rect::new(nav.right(), body_y, gap, body_h);
    d.hit(gap_nav, HitKind::DragNav);

    let main_x = nav.right() + gap;
    let main_w = (w - m - main_x).max(80.0);
    let main = Rect::new(main_x, body_y, main_w, body_h);

    match state.page {
        Page::Performance => performance(&mut d, state, snap, main, mouse),
        Page::Startup => startup_page(&mut d, state, startup, main, mouse),
        Page::Processes => processes(&mut d, state, snap, main, mouse),
    }

    if let Some(undo) = &state.undo {
        if undo.until > Instant::now() {
            let r = Rect::new(
                main.x + 16.0,
                main.bottom() - 48.0,
                (main.w - 32.0).max(40.0),
                34.0,
            );
            d.slab(r, 12.0, [16, 16, 16, 210], theme::WELL_BORDER, 1.0);
            d.text(
                &undo.label,
                Rect::new(r.x + 12.0, r.y, r.w - 90.0, r.h),
                13.0,
                theme::INK,
                false,
                false,
            );
            if undo.revert.is_some() {
                let u = Rect::new(r.right() - 72.0, r.y + 5.0, 60.0, r.h - 10.0);
                d.slab(u, 8.0, theme::PILL, [0, 0, 0, 0], 0.0);
                d.text("Undo", u, 12.0, theme::INK, false, false);
                if let Some(last) = d.labels.last_mut() {
                    let tw = text_width(&last.text, 12.0, false);
                    last.x = u.x + (u.w - tw) * 0.5;
                }
                d.hit(u, HitKind::Undo);
            }
        }
    }
    d
}

fn mem_label(snap: &Snap) -> String {
    if snap.mem_total == 0 {
        "MEM —".into()
    } else {
        format!(
            "MEM {}",
            percent(snap.mem_used as f32 / snap.mem_total as f32 * 100.0)
        )
    }
}

fn nav_items(d: &mut DrawList, state: &AppState, nav: Rect, mouse: [f32; 2], sample_ms: f32) {
    let items = [
        ("Processes", Page::Processes),
        ("Performance", Page::Performance),
        ("Startup", Page::Startup),
    ];
    let mut y = nav.y + 14.0;
    for (label, page) in items {
        let r = Rect::new(nav.x + 10.0, y, nav.w - 20.0, 36.0);
        let on = state.page == page;
        let hot = r.contains(mouse[0], mouse[1]);
        if on || hot {
            d.slab(
                r,
                10.0,
                if on { theme::SELECTED } else { theme::HOVER },
                [0, 0, 0, 0],
                0.0,
            );
        }
        d.text(
            label,
            Rect::new(r.x + 12.0, r.y, r.w - 16.0, r.h),
            14.0,
            if on { theme::INK } else { theme::DIM },
            false,
            false,
        );
        d.hit(r, HitKind::Page(page));
        y += 40.0;
    }
    d.text(
        &format!("v0.1   {sample_ms:.1} ms"),
        Rect::new(nav.x + 16.0, nav.bottom() - 28.0, nav.w - 32.0, 18.0),
        11.0,
        theme::FAINT,
        true,
        false,
    );
}

fn processes(d: &mut DrawList, state: &mut AppState, snap: &Snap, main: Rect, mouse: [f32; 2]) {
    d.slab(main, 16.0, theme::WELL, theme::WELL_BORDER, 1.0);
    let inner = main.inset(14.0);
    let mut y = inner.y;
    let views = [
        ("Grouped", ProcView::Grouped),
        ("Flat", ProcView::Flat),
        ("User", ProcView::User),
        ("System", ProcView::System),
    ];
    let mut x = inner.x;
    for (label, view) in views {
        let tw = text_width(label, 12.0, false) + 22.0;
        let r = Rect::new(x, y, tw, 28.0);
        let on = state.view == view;
        d.slab(
            r,
            9.0,
            if on { theme::PILL } else { [255, 255, 255, 8] },
            [0, 0, 0, 0],
            0.0,
        );
        d.text(
            label,
            r,
            12.0,
            if on { theme::INK } else { theme::MUTED },
            false,
            false,
        );
        if let Some(last) = d.labels.last_mut() {
            let w = text_width(&last.text, 12.0, false);
            last.x = r.x + (r.w - w) * 0.5;
        }
        d.hit(r, HitKind::View(view));
        x += tw + 6.0;
    }
    let dense = if state.density == Density::Compact {
        "Compact"
    } else {
        "Comfortable"
    };
    let dw = text_width(dense, 12.0, false) + 22.0;
    let dr = Rect::new(x, y, dw, 28.0);
    d.slab(dr, 9.0, [255, 255, 255, 8], [0, 0, 0, 0], 0.0);
    d.text(dense, dr, 12.0, theme::MUTED, false, false);
    if let Some(last) = d.labels.last_mut() {
        let w = text_width(&last.text, 12.0, false);
        last.x = dr.x + (dr.w - w) * 0.5;
    }
    d.hit(dr, HitKind::Density);

    let end = selection_label(state);
    let ew = text_width(&end, 12.0, false) + 26.0;
    let er = Rect::new(inner.right() - ew, y, ew, 28.0);
    let armed = state
        .armed
        .as_ref()
        .is_some_and(|a| a.until > Instant::now() && a.pids == state.selected);
    d.slab(
        er,
        9.0,
        if armed { theme::SELECTED } else { theme::PILL },
        [0, 0, 0, 0],
        0.0,
    );
    d.text(
        &end,
        er,
        12.0,
        if state.selected.is_empty() {
            theme::FAINT
        } else {
            theme::INK
        },
        false,
        false,
    );
    if let Some(last) = d.labels.last_mut() {
        let w = text_width(&last.text, 12.0, false);
        last.x = er.x + (er.w - w) * 0.5;
    }
    d.hit(er, HitKind::EndTask);

    let search_w = 200.0_f32.min((er.x - dr.right() - 16.0).max(80.0));
    let sr = Rect::new(er.x - 8.0 - search_w, y, search_w, 28.0);
    let focus = state.search_focused;
    d.slab(
        sr,
        9.0,
        [0, 0, 0, 70],
        if focus {
            theme::WELL_BORDER
        } else {
            [255, 255, 255, 28]
        },
        1.0,
    );
    let q = if state.query.is_empty() && !focus {
        "Search".to_string()
    } else if focus {
        format!("{}|", state.query)
    } else {
        state.query.clone()
    };
    d.text(
        &q,
        Rect::new(sr.x + 10.0, sr.y, sr.w - 16.0, sr.h),
        12.0,
        if state.query.is_empty() && !focus {
            theme::FAINT
        } else {
            theme::INK
        },
        true,
        false,
    );
    d.hit(sr, HitKind::Search);

    y += 38.0;
    let cols = columns(state.density, inner.w);
    let header = Rect::new(inner.x, y, inner.w, 22.0);
    draw_header(d, &cols, header, state.sort.col);
    y += 24.0;
    let list = Rect::new(inner.x, y, inner.w, (inner.bottom() - y).max(20.0));
    d.list_rect = Some(list);

    let rows = visible_rows(state, snap);
    state.visible_pids = rows
        .iter()
        .filter_map(|r| match r {
            Row::Proc(p) => Some(p.pid),
            Row::Header { .. } => None,
        })
        .collect();
    let row_h = if state.density == Density::Compact {
        26.0
    } else {
        32.0
    };
    let content_h = rows.len() as f32 * row_h;
    let max_scroll = (content_h - list.h).max(0.0);
    if state.scroll > max_scroll {
        state.scroll = max_scroll;
    }
    let first = (state.scroll / row_h).floor() as usize;
    let nvis = ((list.h / row_h).ceil() as usize) + 2;
    d.clip = Some(list);
    for (i, row) in rows.iter().enumerate().skip(first).take(nvis) {
        let ry = list.y + i as f32 * row_h - state.scroll;
        if ry + row_h < list.y || ry > list.bottom() {
            continue;
        }
        let rr = Rect::new(
            list.x,
            ry.max(list.y),
            list.w,
            row_h.min(list.bottom() - ry.max(list.y)),
        );
        match row {
            Row::Header {
                title,
                count,
                open,
                user,
            } => {
                d.slab(
                    Rect::new(list.x, ry, list.w, row_h),
                    8.0,
                    theme::CELL,
                    [0, 0, 0, 0],
                    0.0,
                );
                let mark = if *open { "–" } else { "+" };
                d.text(
                    &format!("{mark}  {title}  {count}"),
                    Rect::new(list.x + 8.0, ry, list.w - 16.0, row_h),
                    12.0,
                    theme::DIM,
                    false,
                    false,
                );
                d.hit(Rect::new(list.x, ry, list.w, row_h), HitKind::Group(*user));
            }
            Row::Proc(p) => {
                let on = state.selected.contains(&p.pid);
                let hot = Rect::new(list.x, ry, list.w, row_h).contains(mouse[0], mouse[1]);
                if on || hot {
                    d.slab(
                        Rect::new(list.x, ry, list.w, row_h),
                        8.0,
                        if on { theme::SELECTED } else { theme::HOVER },
                        [0, 0, 0, 0],
                        0.0,
                    );
                }
                draw_proc(d, &cols, Rect::new(list.x, ry, list.w, row_h), p);
                let _ = rr;
                d.hit(
                    Rect::new(list.x, ry, list.w, row_h),
                    HitKind::Proc { pid: p.pid },
                );
            }
        }
    }
    d.clip = None;
    scrollbar(d, list, content_h, state.scroll);
    if rows.is_empty() {
        d.text(
            "No matching processes",
            Rect::new(list.x, list.y + 12.0, list.w, 24.0),
            13.0,
            theme::MUTED,
            false,
            false,
        );
    }
}

enum Row<'a> {
    Header {
        title: &'static str,
        count: usize,
        open: bool,
        user: bool,
    },
    Proc(&'a Proc),
}

fn visible_rows<'a>(state: &AppState, snap: &'a Snap) -> Vec<Row<'a>> {
    let q = state.query.to_ascii_lowercase();
    let mut procs: Vec<&Proc> = snap
        .procs
        .iter()
        .filter(|p| match state.view {
            ProcView::User => p.is_user,
            ProcView::System => !p.is_user,
            _ => true,
        })
        .filter(|p| {
            if q.is_empty() {
                return true;
            }
            p.name.to_ascii_lowercase().contains(&q)
                || p.user.to_ascii_lowercase().contains(&q)
                || p.pid.to_string().contains(q.trim())
        })
        .collect();
    sort_procs(&mut procs, state.sort.col, state.sort.desc);
    if state.view != ProcView::Grouped {
        return procs.into_iter().map(Row::Proc).collect();
    }
    let mut user: Vec<&Proc> = Vec::new();
    let mut system: Vec<&Proc> = Vec::new();
    for p in procs {
        if p.is_user {
            user.push(p);
        } else {
            system.push(p);
        }
    }
    let mut rows = Vec::new();
    rows.push(Row::Header {
        title: "User",
        count: user.len(),
        open: state.user_open,
        user: true,
    });
    if state.user_open {
        rows.extend(user.into_iter().map(Row::Proc));
    }
    rows.push(Row::Header {
        title: "System",
        count: system.len(),
        open: state.system_open,
        user: false,
    });
    if state.system_open {
        rows.extend(system.into_iter().map(Row::Proc));
    }
    rows
}

fn sort_procs(procs: &mut [&Proc], col: Col, desc: bool) {
    procs.sort_by(|a, b| {
        let ord = match col {
            Col::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            Col::Cpu => a
                .cpu
                .partial_cmp(&b.cpu)
                .unwrap_or(std::cmp::Ordering::Equal),
            Col::Memory => a.rss.cmp(&b.rss),
            Col::Disk => disk_sum(a)
                .partial_cmp(&disk_sum(b))
                .unwrap_or(std::cmp::Ordering::Equal),
            Col::Pid => a.pid.cmp(&b.pid),
            Col::User => a.user.to_lowercase().cmp(&b.user.to_lowercase()),
            Col::Threads => a.threads.cmp(&b.threads),
        };
        if desc {
            ord.reverse()
        } else {
            ord
        }
    });
}

fn disk_sum(p: &Proc) -> f64 {
    p.read_bps.unwrap_or(0.0) + p.write_bps.unwrap_or(0.0)
}

struct ColSpec {
    col: Col,
    x: f32,
    w: f32,
    right: bool,
    mono: bool,
}

fn columns(density: Density, width: f32) -> Vec<ColSpec> {
    let spec: &[(Col, f32, bool, bool)] = if density == Density::Compact {
        &[
            (Col::Pid, 64.0, true, true),
            (Col::Memory, 84.0, true, true),
            (Col::Cpu, 64.0, true, true),
        ]
    } else {
        &[
            (Col::Threads, 72.0, true, true),
            (Col::User, 88.0, false, false),
            (Col::Pid, 64.0, true, true),
            (Col::Disk, 88.0, true, true),
            (Col::Memory, 84.0, true, true),
            (Col::Cpu, 68.0, true, true),
        ]
    };
    let fixed: f32 = spec.iter().map(|(_, w, _, _)| *w).sum();
    let x = width - fixed;
    let mut cols = vec![ColSpec {
        col: Col::Name,
        x: 8.0,
        w: (x - 12.0).max(40.0),
        right: false,
        mono: false,
    }];
    // Place fixed columns from the left of the remaining region, in reverse of the spec
    // so CPU sits just right of the name.
    let mut cursor = x;
    let mut placed = Vec::new();
    for (col, w, right, mono) in spec.iter().rev() {
        placed.push(ColSpec {
            col: *col,
            x: cursor,
            w: *w,
            right: *right,
            mono: *mono,
        });
        cursor += *w;
    }
    cols.extend(placed);
    let _ = cursor;
    cols
}

fn col_title(col: Col) -> &'static str {
    match col {
        Col::Name => "Name",
        Col::Cpu => "CPU",
        Col::Memory => "Memory",
        Col::Disk => "Disk",
        Col::Pid => "PID",
        Col::User => "User",
        Col::Threads => "Threads",
    }
}

fn draw_header(d: &mut DrawList, cols: &[ColSpec], row: Rect, active: Col) {
    for c in cols {
        let r = Rect::new(row.x + c.x, row.y, c.w - 8.0, row.h);
        let color = if c.col == active {
            theme::INK
        } else {
            theme::MUTED
        };
        d.text(col_title(c.col), r, 11.0, color, true, c.right);
        d.hit(
            Rect::new(row.x + c.x, row.y, c.w, row.h),
            HitKind::Sort(c.col),
        );
    }
}

fn draw_proc(d: &mut DrawList, cols: &[ColSpec], row: Rect, p: &Proc) {
    for c in cols {
        let r = Rect::new(row.x + c.x, row.y, c.w - 8.0, row.h);
        let text = match c.col {
            Col::Name => p.name.clone(),
            Col::Cpu => cpu_pct(p.cpu),
            Col::Memory => bytes(p.rss),
            Col::Disk => disk_cell(p.read_bps, p.write_bps),
            Col::Pid => p.pid.to_string(),
            Col::User => p.user.clone(),
            Col::Threads => p.threads.to_string(),
        };
        d.text(
            &text,
            r,
            12.5,
            theme::INK,
            c.mono || c.col != Col::Name && c.col != Col::User,
            c.right,
        );
    }
}

fn scrollbar(d: &mut DrawList, viewport: Rect, content_h: f32, scroll: f32) {
    if content_h <= viewport.h + 1.0 {
        return;
    }
    let thumb_h = (viewport.h * viewport.h / content_h).clamp(24.0, viewport.h);
    let max_scroll = content_h - viewport.h;
    let t = (scroll / max_scroll).clamp(0.0, 1.0);
    let y = viewport.y + (viewport.h - thumb_h) * t;
    d.slab(
        Rect::new(viewport.right() - 3.0, y, 2.0, thumb_h),
        1.0,
        [255, 255, 255, 90],
        [0, 0, 0, 0],
        0.0,
    );
}

fn performance(d: &mut DrawList, state: &mut AppState, snap: &Snap, main: Rect, mouse: [f32; 2]) {
    let sub = Rect::new(main.x, main.y, state.sub_w.min(main.w * 0.4), main.h);
    d.slab(sub, 16.0, theme::WELL, theme::WELL_BORDER, 1.0);
    let items = [
        ("CPU", Section::Cpu, Some(percent(snap.cpu_total))),
        ("Memory", Section::Memory, Some(mem_short(snap))),
        (
            "GPU",
            Section::Gpu,
            snap.gpus.first().and_then(|g| g.util).map(percent),
        ),
        ("Disk", Section::Disk, None),
        ("Network", Section::Net, None),
    ];
    let mut y = sub.y + 14.0;
    for (label, section, extra) in items {
        let r = Rect::new(sub.x + 10.0, y, sub.w - 20.0, 36.0);
        let on = state.section == section;
        let hot = r.contains(mouse[0], mouse[1]);
        if on || hot {
            d.slab(
                r,
                10.0,
                if on { theme::SELECTED } else { theme::HOVER },
                [0, 0, 0, 0],
                0.0,
            );
        }
        d.text(
            label,
            Rect::new(r.x + 10.0, r.y, r.w * 0.55, r.h),
            13.0,
            if on { theme::INK } else { theme::DIM },
            false,
            false,
        );
        if let Some(extra) = extra {
            d.text(
                &extra,
                Rect::new(r.x + r.w * 0.45, r.y, r.w * 0.5, r.h),
                12.0,
                theme::MUTED,
                true,
                true,
            );
        }
        d.hit(r, HitKind::Section(section));
        y += 40.0;
    }
    let gap = Rect::new(sub.right(), main.y, 10.0, main.h);
    d.hit(gap, HitKind::DragSub);
    let detail = Rect::new(
        sub.right() + 10.0,
        main.y,
        (main.right() - sub.right() - 10.0).max(40.0),
        main.h,
    );
    d.slab(detail, 16.0, theme::WELL, theme::WELL_BORDER, 1.0);
    let view = detail.inset(16.0);
    d.detail_rect = Some(view);
    let y0 = view.y - state.perf_scroll;
    d.clip = Some(view);
    let content_bottom = match state.section {
        Section::Cpu => cpu_page(d, snap, view, y0),
        Section::Memory => memory_page(d, snap, view, y0),
        Section::Gpu => gpu_page(d, snap, view, y0),
        Section::Disk => io_page(d, view, y0, true, snap),
        Section::Net => io_page(d, view, y0, false, snap),
    };
    d.clip = None;
    let content_h = (content_bottom - y0).max(0.0);
    let max_scroll = (content_h - view.h).max(0.0);
    if state.perf_scroll > max_scroll {
        state.perf_scroll = max_scroll;
    }
    scrollbar(d, view, content_h, state.perf_scroll);
}

fn mem_short(snap: &Snap) -> String {
    if snap.mem_total == 0 {
        "—".into()
    } else {
        percent(snap.mem_used as f32 / snap.mem_total as f32 * 100.0)
    }
}

fn cpu_page(d: &mut DrawList, snap: &Snap, view: Rect, mut y: f32) -> f32 {
    d.text(
        &snap.cpu_model,
        Rect::new(view.x, y, view.w, 22.0),
        13.0,
        theme::MUTED,
        false,
        false,
    );
    y += 26.0;
    let hero_h = 168.0;
    if y + hero_h > view.y - 40.0 && y < view.bottom() + 40.0 {
        let graph = Rect::new(view.x, y, (view.w - 200.0).max(40.0), hero_h);
        d.graph(graph, &[(&snap.cpu_hist, theme::TRACE)], 100.0);
        d.text(
            &percent(snap.cpu_total),
            Rect::new(graph.x, graph.y, 120.0, 36.0),
            28.0,
            theme::INK,
            true,
            false,
        );
        let sx = graph.right() + 16.0;
        let freq: f32 = if snap.cpu_freq_mhz.is_empty() {
            0.0
        } else {
            snap.cpu_freq_mhz.iter().sum::<f32>() / snap.cpu_freq_mhz.len() as f32
        };
        let lines = [
            freq_ghz(freq),
            format!("{} proc", snap.proc_count),
            format!("{} threads", snap.thread_count),
            duration(snap.uptime_secs),
            format!("load {:.2}", snap.load[0]),
        ];
        for (i, line) in lines.iter().enumerate() {
            d.text(
                line,
                Rect::new(sx, y + i as f32 * 28.0, 180.0, 24.0),
                13.0,
                theme::DIM,
                true,
                false,
            );
        }
    }
    y += hero_h + 18.0;
    let n = snap.cpu_per.len();
    if n == 0 {
        return y;
    }
    let (cols, rows, cell) = format::square_grid(n, view.w);
    for i in 0..n {
        let col = i % cols;
        let row = i / cols;
        let r = Rect::new(
            view.x + col as f32 * cell,
            y + row as f32 * cell,
            cell - 8.0,
            cell - 8.0,
        );
        if r.bottom() < view.y || r.y > view.bottom() {
            continue;
        }
        d.slab(r, 12.0, theme::CELL, [255, 255, 255, 28], 1.0);
        d.text(
            &format!("CPU {i}"),
            Rect::new(r.x + 10.0, r.y + 6.0, r.w * 0.5, 20.0),
            12.0,
            theme::MUTED,
            true,
            false,
        );
        d.text(
            &cpu_pct(snap.cpu_per[i]),
            Rect::new(r.x + r.w * 0.4, r.y + 6.0, r.w * 0.55, 20.0),
            12.0,
            theme::INK,
            true,
            true,
        );
        let hist = snap.cpu_per_hist.get(i).map(Vec::as_slice).unwrap_or(&[]);
        d.graph(
            Rect::new(r.x + 10.0, r.y + 32.0, r.w - 20.0, (r.h - 44.0).max(8.0)),
            &[(hist, theme::TRACE)],
            100.0,
        );
    }
    y + rows as f32 * cell
}

fn memory_page(d: &mut DrawList, snap: &Snap, view: Rect, mut y: f32) -> f32 {
    d.text(
        "Memory",
        Rect::new(view.x, y, view.w, 24.0),
        16.0,
        theme::INK,
        false,
        false,
    );
    y += 30.0;
    let gh = 160.0;
    d.graph(
        Rect::new(view.x, y, view.w, gh),
        &[(&snap.mem_hist, theme::TRACE)],
        1.0,
    );
    d.text(
        &format!("{} / {}", bytes(snap.mem_used), bytes(snap.mem_total)),
        Rect::new(view.x, y, view.w * 0.7, 28.0),
        18.0,
        theme::INK,
        true,
        false,
    );
    y += gh + 16.0;
    let stats = [
        ("Available", bytes(snap.mem_available)),
        ("Cached", bytes(snap.mem_cached)),
        ("Buffers", bytes(snap.mem_buffers)),
    ];
    for (i, (k, v)) in stats.iter().enumerate() {
        let x = view.x + (i as f32) * (view.w / 3.0);
        d.text(
            k,
            Rect::new(x, y, view.w / 3.0 - 8.0, 16.0),
            11.0,
            theme::MUTED,
            false,
            false,
        );
        d.text(
            v,
            Rect::new(x, y + 16.0, view.w / 3.0 - 8.0, 20.0),
            14.0,
            theme::INK,
            true,
            false,
        );
    }
    y += 52.0;
    if snap.swap_total > 0 {
        d.text(
            &format!(
                "Swap  {} / {}",
                bytes(snap.swap_used),
                bytes(snap.swap_total)
            ),
            Rect::new(view.x, y, view.w, 20.0),
            13.0,
            theme::DIM,
            true,
            false,
        );
        y += 24.0;
        d.graph(
            Rect::new(view.x, y, view.w, 100.0),
            &[(&snap.swap_hist, theme::TRACE_DIM)],
            1.0,
        );
        y += 110.0;
    }
    y
}

fn gpu_page(d: &mut DrawList, snap: &Snap, view: Rect, mut y: f32) -> f32 {
    if snap.gpus.is_empty() {
        d.text(
            "No GPU reported",
            Rect::new(view.x, y, view.w, 24.0),
            14.0,
            theme::MUTED,
            false,
            false,
        );
        return y + 30.0;
    }
    for g in &snap.gpus {
        d.text(
            &g.name,
            Rect::new(view.x, y, view.w, 24.0),
            16.0,
            theme::INK,
            false,
            false,
        );
        y += 28.0;
        let gh = 120.0;
        let hist = g.util_hist.as_slice();
        d.graph(
            Rect::new(view.x, y, view.w, gh),
            &[(hist, theme::TRACE)],
            100.0,
        );
        let util = g.util.map(percent).unwrap_or_else(|| "—".into());
        d.text(
            &util,
            Rect::new(view.x, y, 100.0, 28.0),
            22.0,
            theme::INK,
            true,
            false,
        );
        y += gh + 10.0;
        if g.mem_total > 0 {
            let frac = (g.mem_used as f32 / g.mem_total as f32).clamp(0.0, 1.0);
            d.slab(
                Rect::new(view.x, y, view.w, 6.0),
                3.0,
                [255, 255, 255, 24],
                [0, 0, 0, 0],
                0.0,
            );
            d.slab(
                Rect::new(view.x, y, view.w * frac, 6.0),
                3.0,
                theme::TRACE,
                [0, 0, 0, 0],
                0.0,
            );
            y += 14.0;
            d.text(
                &format!("{} / {}", bytes(g.mem_used), bytes(g.mem_total)),
                Rect::new(view.x, y, view.w, 18.0),
                12.0,
                theme::DIM,
                true,
                false,
            );
            y += 22.0;
        }
        let mut bits = Vec::new();
        if let Some(t) = g.temp_c {
            bits.push(format!("{t} °C"));
        }
        if let Some(p) = g.power_w {
            bits.push(format!("{p:.0} W"));
        }
        if let Some(c) = g.clk_core {
            bits.push(format!("{c} MHz"));
        }
        if let Some(c) = g.clk_mem {
            bits.push(format!("mem {c} MHz"));
        }
        if let Some(e) = g.enc {
            bits.push(format!("enc {e}%"));
        }
        if let Some(e) = g.dec {
            bits.push(format!("dec {e}%"));
        }
        if g.integrated {
            bits.push("integrated".into());
        }
        d.text(
            &bits.join("    "),
            Rect::new(view.x, y, view.w, 18.0),
            12.0,
            theme::MUTED,
            true,
            false,
        );
        y += 36.0;
    }
    y
}

fn io_page(d: &mut DrawList, view: Rect, mut y: f32, disk: bool, snap: &Snap) -> f32 {
    if disk {
        if snap.disks.is_empty() {
            d.text(
                "No disks",
                Rect::new(view.x, y, view.w, 24.0),
                14.0,
                theme::MUTED,
                false,
                false,
            );
            return y + 28.0;
        }
        for dev in &snap.disks {
            d.text(
                &dev.name,
                Rect::new(view.x, y, view.w * 0.4, 22.0),
                15.0,
                theme::INK,
                true,
                false,
            );
            d.text(
                &format!("R {}    W {}", rate(dev.read_bps), rate(dev.write_bps)),
                Rect::new(view.x + view.w * 0.35, y, view.w * 0.65, 22.0),
                13.0,
                theme::DIM,
                true,
                true,
            );
            y += 26.0;
            let max = nice_pair(&dev.read_hist, &dev.write_hist);
            d.graph(
                Rect::new(view.x, y, view.w, 110.0),
                &[
                    (&dev.read_hist, theme::TRACE),
                    (&dev.write_hist, theme::TRACE_DIM),
                ],
                max,
            );
            y += 126.0;
        }
    } else if snap.nets.is_empty() {
        d.text(
            "No interfaces",
            Rect::new(view.x, y, view.w, 24.0),
            14.0,
            theme::MUTED,
            false,
            false,
        );
        return y + 28.0;
    } else {
        for dev in &snap.nets {
            d.text(
                &dev.name,
                Rect::new(view.x, y, view.w * 0.4, 22.0),
                15.0,
                theme::INK,
                true,
                false,
            );
            d.text(
                &format!("RX {}    TX {}", rate(dev.rx_bps), rate(dev.tx_bps)),
                Rect::new(view.x + view.w * 0.3, y, view.w * 0.7, 22.0),
                13.0,
                theme::DIM,
                true,
                true,
            );
            y += 26.0;
            let max = nice_pair(&dev.rx_hist, &dev.tx_hist);
            d.graph(
                Rect::new(view.x, y, view.w, 110.0),
                &[
                    (&dev.rx_hist, theme::TRACE),
                    (&dev.tx_hist, theme::TRACE_DIM),
                ],
                max,
            );
            y += 126.0;
        }
    }
    y
}

fn nice_pair(a: &[f32], b: &[f32]) -> f32 {
    let m = a.iter().chain(b.iter()).copied().fold(0.0_f32, f32::max);
    format::nice_ceil(m)
}

fn startup_page(
    d: &mut DrawList,
    state: &mut AppState,
    startup: &[StartupEntry],
    main: Rect,
    mouse: [f32; 2],
) {
    d.slab(main, 16.0, theme::WELL, theme::WELL_BORDER, 1.0);
    let inner = main.inset(16.0);
    d.text(
        "Startup apps",
        Rect::new(inner.x, inner.y, inner.w, 24.0),
        16.0,
        theme::INK,
        false,
        false,
    );
    d.text(
        "User autostart. Turning one off writes Hidden=true and keeps a .bak.",
        Rect::new(inner.x, inner.y + 24.0, inner.w, 18.0),
        12.0,
        theme::MUTED,
        false,
        false,
    );
    let list = Rect::new(inner.x, inner.y + 52.0, inner.w, (inner.h - 52.0).max(20.0));
    d.startup_rect = Some(list);
    if startup.is_empty() {
        d.text(
            "Nothing in ~/.config/autostart",
            Rect::new(list.x, list.y, list.w, 24.0),
            13.0,
            theme::MUTED,
            false,
            false,
        );
        return;
    }
    let row_h = 48.0;
    let content_h = startup.len() as f32 * row_h;
    let max_scroll = (content_h - list.h).max(0.0);
    if state.startup_scroll > max_scroll {
        state.startup_scroll = max_scroll;
    }
    let first = (state.startup_scroll / row_h).floor() as usize;
    let nvis = ((list.h / row_h).ceil() as usize) + 2;
    for (i, entry) in startup.iter().enumerate().skip(first).take(nvis) {
        let ry = list.y + i as f32 * row_h - state.startup_scroll;
        if ry + row_h < list.y || ry > list.bottom() {
            continue;
        }
        let hot = Rect::new(list.x, ry, list.w, row_h).contains(mouse[0], mouse[1]);
        if hot {
            d.slab(
                Rect::new(list.x, ry, list.w, row_h),
                10.0,
                theme::HOVER,
                [0, 0, 0, 0],
                0.0,
            );
        }
        d.text(
            &entry.name,
            Rect::new(list.x + 8.0, ry + 4.0, list.w - 90.0, 22.0),
            14.0,
            theme::INK,
            false,
            false,
        );
        d.text(
            &entry.exec,
            Rect::new(list.x + 8.0, ry + 24.0, list.w - 90.0, 18.0),
            11.0,
            theme::MUTED,
            true,
            false,
        );
        let pill = Rect::new(list.right() - 64.0, ry + 12.0, 52.0, 24.0);
        d.slab(
            pill,
            12.0,
            if entry.enabled {
                theme::PILL
            } else {
                [255, 255, 255, 8]
            },
            [0, 0, 0, 0],
            0.0,
        );
        d.text(
            if entry.enabled { "on" } else { "off" },
            pill,
            12.0,
            if entry.enabled {
                theme::INK
            } else {
                theme::FAINT
            },
            true,
            false,
        );
        if let Some(last) = d.labels.last_mut() {
            let tw = text_width(&last.text, 12.0, true);
            last.x = pill.x + (pill.w - tw) * 0.5;
        }
        d.hit(Rect::new(list.x, ry, list.w, row_h), HitKind::Startup(i));
    }
    scrollbar(d, list, content_h, state.startup_scroll);
}
