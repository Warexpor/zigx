use std::time::Instant;

use crate::format::{
    self, bytes, cpu_pct, disk_cell, duration, fit_t, freq_ghz, percent, rate, text_width_t,
};
use crate::interact::selection_label;
use crate::model::{
    theme, AppState, Col, Density, Page, Proc, ProcView, Section, Snap, Sort, StartupEntry,
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
    pub shadow: f32,
    pub shadow_a: f32,
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
    pub weight: u16,
    /// Letter spacing in em.
    pub tracking: f32,
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

// --- Type roles --------------------------------------------------------------
//
// Two families, one weight. Hierarchy comes from size, alpha, and case, not
// from bold. Micro labels are uppercase and tracked like panel engraving.

#[derive(Clone, Copy, Debug)]
struct Type {
    size: f32,
    mono: bool,
    weight: u16,
    tracking: f32,
    upper: bool,
}

const fn ty(size: f32, mono: bool, weight: u16, tracking: f32, upper: bool) -> Type {
    Type {
        size,
        mono,
        weight,
        tracking,
        upper,
    }
}

const WORDMARK: Type = ty(11.5, false, 500, 0.18, true);
const MICRO: Type = ty(10.0, false, 400, 0.10, true);
const MICRO_NUM: Type = ty(10.0, true, 400, 0.06, true);
const NAV: Type = ty(13.0, false, 400, 0.0, false);
const BODY: Type = ty(12.5, false, 400, 0.0, false);
const PILL: Type = ty(12.0, false, 400, 0.0, false);
const PILL_ON: Type = ty(12.0, false, 500, 0.0, false);
const NUM: Type = ty(12.0, true, 400, 0.0, false);
const NUM_SMALL: Type = ty(10.5, true, 400, 0.0, false);
const TITLE: Type = ty(22.0, false, 400, -0.012, false);
const SUB: Type = ty(12.0, false, 400, 0.0, false);
const READOUT: Type = ty(44.0, true, 400, -0.02, false);
const READOUT_SUB: Type = ty(16.0, true, 400, 0.0, false);
const STAT: Type = ty(15.0, true, 400, 0.0, false);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Align {
    Left,
    Right,
    Center,
}

fn measure(s: &str, t: Type) -> f32 {
    text_width_t(s, t.size, t.mono, t.tracking)
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

const NONE: theme::Rgba = [0, 0, 0, 0];

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

    fn fill(&mut self, r: Rect, radius: f32, fill: theme::Rgba) {
        self.slab(r, radius, fill, NONE, 0.0);
    }

    fn outline(&mut self, r: Rect, radius: f32, line: theme::Rgba) {
        self.slab(r, radius, NONE, line, 1.0);
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
            shadow: 0.0,
            shadow_a: 0.0,
        });
    }

    /// 1px hairline, the only hard edge in the system.
    fn hairline(&mut self, r: Rect) {
        self.fill(r, 0.0, theme::HAIRLINE);
    }

    fn hit(&mut self, rect: Rect, kind: HitKind) {
        if self.visible(rect) {
            self.hits.push(Hit { rect, kind });
        }
    }

    fn line(&mut self, pts: &[[f32; 2]], width: f32, color: theme::Rgba) {
        self.strokes.push(Stroke {
            pts: pts.to_vec(),
            width,
            color,
            baseline: None,
        });
    }

    fn text(&mut self, s: &str, r: Rect, t: Type, color: theme::Rgba) {
        self.place(s, r, t, color, Align::Left);
    }

    fn text_r(&mut self, s: &str, r: Rect, t: Type, color: theme::Rgba) {
        self.place(s, r, t, color, Align::Right);
    }

    fn text_c(&mut self, s: &str, r: Rect, t: Type, color: theme::Rgba) {
        self.place(s, r, t, color, Align::Center);
    }

    fn place(&mut self, s: &str, r: Rect, t: Type, color: theme::Rgba, align: Align) {
        if !self.visible(r) || color[3] == 0 {
            return;
        }
        let owned;
        let s = if t.upper {
            owned = s.to_uppercase();
            owned.as_str()
        } else {
            s
        };
        let fitted = fit_t(s, r.w, t.size, t.mono, t.tracking);
        if fitted.is_empty() {
            return;
        }
        let tw = measure(&fitted, t);
        let lh = t.size * 1.3;
        let x = match align {
            Align::Left => r.x,
            Align::Right => r.right() - tw,
            Align::Center => r.x + (r.w - tw) * 0.5,
        };
        let y = r.y + (r.h - lh) * 0.5;
        self.labels.push(Label {
            text: fitted,
            x,
            y,
            w: r.w,
            h: lh,
            size: t.size,
            color,
            mono: t.mono,
            weight: t.weight,
            tracking: t.tracking,
        });
    }

    /// Telemetry graph: faint quarter grid, hairline baseline, thin traces.
    fn graph(&mut self, r: Rect, series: &[(&[f32], theme::Rgba)], max: f32, dot: bool) {
        if r.w < 4.0 || r.h < 4.0 || !self.visible(r) {
            return;
        }
        let max = max.max(0.001);
        if r.h >= 80.0 {
            for k in 1..4 {
                let gy = r.y + r.h * (k as f32) / 4.0;
                self.fill(Rect::new(r.x, gy, r.w, 1.0), 0.0, theme::GRID);
            }
        }
        self.hairline(Rect::new(r.x, r.bottom(), r.w, 1.0));
        for (si, (values, color)) in series.iter().enumerate() {
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
                width: 1.25,
                color: *color,
                baseline: Some(r.bottom()),
            });
            if dot && si == 0 {
                if let Some(p) = pts.last() {
                    self.fill(Rect::new(p[0] - 2.0, p[1] - 2.0, 4.0, 4.0), 2.0, *color);
                }
            }
        }
    }
}

pub fn hit_at(hits: &[Hit], x: f32, y: f32) -> Option<HitKind> {
    hits.iter()
        .rev()
        .find(|h| h.rect.contains(x, y))
        .map(|h| h.kind)
}

// --- Icons -----------------------------------------------------------------

#[derive(Clone, Copy)]
enum Icon {
    List,
    Pulse,
    Launch,
    Chip,
    Mem,
    Gpu,
    Disk,
    Net,
    Search,
    Pin,
    Min,
    Close,
    ChevronDown,
    ChevronRight,
}

const ICON_W: f32 = 1.25;

/// Draw a 14x14 line icon at (x, y).
fn icon(d: &mut DrawList, kind: Icon, x: f32, y: f32, c: theme::Rgba) {
    let w = ICON_W;
    match kind {
        Icon::List => {
            for i in 0..3 {
                let yy = y + 2.5 + i as f32 * 4.0;
                d.line(&[[x + 1.0, yy], [x + 3.0, yy]], w, c);
                d.line(&[[x + 5.5, yy], [x + 13.0, yy]], w, c);
            }
        }
        Icon::Pulse => d.line(
            &[
                [x + 1.0, y + 8.5],
                [x + 4.0, y + 8.5],
                [x + 6.0, y + 3.0],
                [x + 8.5, y + 11.5],
                [x + 10.5, y + 8.5],
                [x + 13.0, y + 8.5],
            ],
            w,
            c,
        ),
        Icon::Launch => {
            d.line(&[[x + 3.0, y + 11.0], [x + 11.0, y + 3.0]], w, c);
            d.line(
                &[[x + 5.5, y + 3.0], [x + 11.0, y + 3.0], [x + 11.0, y + 8.5]],
                w,
                c,
            );
        }
        Icon::Chip => {
            d.slab(Rect::new(x + 2.0, y + 2.0, 10.0, 10.0), 2.0, NONE, c, w);
            d.fill(Rect::new(x + 5.75, y + 5.75, 2.5, 2.5), 0.5, c);
        }
        Icon::Mem => {
            d.fill(Rect::new(x + 2.0, y + 7.0, 2.2, 5.0), 0.6, c);
            d.fill(Rect::new(x + 5.9, y + 4.5, 2.2, 7.5), 0.6, c);
            d.fill(Rect::new(x + 9.8, y + 2.0, 2.2, 10.0), 0.6, c);
        }
        Icon::Gpu => {
            d.slab(Rect::new(x + 1.0, y + 3.5, 12.0, 8.0), 2.0, NONE, c, w);
            d.fill(Rect::new(x + 6.0, y + 6.5, 2.0, 2.0), 0.5, c);
        }
        Icon::Disk => {
            d.slab(Rect::new(x + 2.0, y + 2.0, 10.0, 10.0), 5.0, NONE, c, w);
            d.fill(Rect::new(x + 6.0, y + 6.0, 2.0, 2.0), 1.0, c);
        }
        Icon::Net => {
            d.line(&[[x + 4.0, y + 2.5], [x + 4.0, y + 9.5]], w, c);
            d.line(
                &[[x + 1.8, y + 7.0], [x + 4.0, y + 9.5], [x + 6.2, y + 7.0]],
                w,
                c,
            );
            d.line(&[[x + 10.0, y + 4.5], [x + 10.0, y + 11.5]], w, c);
            d.line(
                &[[x + 7.8, y + 7.0], [x + 10.0, y + 4.5], [x + 12.2, y + 7.0]],
                w,
                c,
            );
        }
        Icon::Search => {
            d.slab(Rect::new(x + 2.0, y + 2.0, 8.0, 8.0), 4.0, NONE, c, w);
            d.line(&[[x + 8.8, y + 8.8], [x + 12.5, y + 12.5]], w, c);
        }
        Icon::Pin => {
            d.slab(Rect::new(x + 4.5, y + 1.5, 5.0, 5.0), 2.5, NONE, c, w);
            d.line(&[[x + 7.0, y + 6.5], [x + 7.0, y + 12.5]], w, c);
        }
        Icon::Min => d.line(&[[x + 3.0, y + 7.0], [x + 11.0, y + 7.0]], w, c),
        Icon::Close => {
            d.line(&[[x + 3.5, y + 3.5], [x + 10.5, y + 10.5]], w, c);
            d.line(&[[x + 10.5, y + 3.5], [x + 3.5, y + 10.5]], w, c);
        }
        Icon::ChevronDown => d.line(
            &[[x + 3.0, y + 5.0], [x + 7.0, y + 9.0], [x + 11.0, y + 5.0]],
            w,
            c,
        ),
        Icon::ChevronRight => d.line(
            &[[x + 5.0, y + 3.0], [x + 9.0, y + 7.0], [x + 5.0, y + 11.0]],
            w,
            c,
        ),
    }
}

fn heat(v: f32) -> theme::Rgba {
    if v >= 90.0 {
        theme::HOT
    } else if v >= 70.0 {
        theme::WARN
    } else {
        theme::INK
    }
}

/// Ghost pill: transparent, 1px outline, label centered. The only button.
fn ghost_pill(
    d: &mut DrawList,
    r: Rect,
    label: &str,
    mouse: [f32; 2],
    enabled: bool,
    danger: bool,
) {
    let hot = enabled && r.contains(mouse[0], mouse[1]);
    let (line, ink) = if danger {
        (theme::DANGER_LINE, theme::DANGER_INK)
    } else if !enabled {
        (theme::HAIRLINE, theme::INK_4)
    } else if hot {
        (theme::ACCENT_LINE, theme::INK)
    } else {
        (theme::GHOST_LINE, theme::INK_2)
    };
    d.slab(
        r,
        r.h * 0.5,
        if hot { theme::HOVER } else { NONE },
        line,
        1.0,
    );
    d.text_c(label, r, PILL, ink);
}

/// Stat: mono value over a tracked micro label. No box.
fn stat(d: &mut DrawList, x: f32, y: f32, w: f32, value: &str, label: &str) {
    d.text(value, Rect::new(x, y, w, 20.0), STAT, theme::INK);
    d.text(label, Rect::new(x, y + 23.0, w, 14.0), MICRO, theme::INK_3);
}

/// Section eyebrow: micro label, optional mono detail to its right.
fn eyebrow(d: &mut DrawList, x: f32, y: f32, w: f32, label: &str, detail: Option<&str>) {
    d.text(label, Rect::new(x, y, w, 14.0), MICRO, theme::INK_3);
    if let Some(detail) = detail {
        let lw = measure(&label.to_uppercase(), MICRO);
        d.text(
            detail,
            Rect::new(x + lw + 14.0, y, (w - lw - 14.0).max(0.0), 14.0),
            MICRO_NUM,
            theme::INK_4,
        );
    }
}

// --- Frame -----------------------------------------------------------------

const BAR_H: f32 = 48.0;

pub fn build(
    state: &mut AppState,
    snap: &Snap,
    startup: &[StartupEntry],
    mouse: [f32; 2],
) -> DrawList {
    let mut d = DrawList::new();
    let w = state.width.max(420.0);
    let h = state.height.max(320.0);

    // One window, one sheet of black glass.
    let root = Rect::new(0.0, 0.0, w, h);
    d.slab(root, 10.0, theme::CANVAS, theme::CANVAS_LINE, 1.0);

    let bar = Rect::new(0.0, 0.0, w, BAR_H);
    let body = Rect::new(0.0, bar.bottom(), w, (h - bar.bottom()).max(80.0));
    let nav = Rect::new(0.0, body.y, state.nav_w, body.h);
    let main = Rect::new(nav.right(), body.y, (w - nav.right()).max(80.0), body.h);

    let perf = if state.page == Page::Performance {
        let sub = Rect::new(main.x, main.y, state.sub_w.min(main.w * 0.4), main.h);
        let detail = Rect::new(
            sub.right() + 1.0,
            main.y,
            (main.right() - sub.right() - 1.0).max(40.0),
            main.h,
        );
        Some((sub, detail))
    } else {
        None
    };

    d.hairline(Rect::new(0.0, bar.bottom(), w, 1.0));
    d.hairline(Rect::new(nav.right(), body.y, 1.0, body.h));
    if let Some((sub, _)) = perf {
        d.hairline(Rect::new(sub.right(), body.y, 1.0, body.h));
    }

    title_bar(&mut d, state, snap, bar, mouse);
    nav_items(&mut d, state, nav, mouse, snap.sample_ms);

    d.hit(
        Rect::new(nav.right() - 2.0, body.y, 5.0, body.h),
        HitKind::DragNav,
    );

    match (state.page, perf) {
        (Page::Performance, Some((sub, detail))) => {
            d.hit(
                Rect::new(sub.right() - 2.0, body.y, 5.0, body.h),
                HitKind::DragSub,
            );
            performance(&mut d, state, snap, sub, detail, mouse)
        }
        (Page::Startup, _) => startup_page(&mut d, state, startup, main, mouse),
        (Page::Processes, _) => processes(&mut d, state, snap, main, mouse),
        _ => {}
    }

    toast(&mut d, state, main);
    d
}

fn title_bar(d: &mut DrawList, state: &AppState, snap: &Snap, bar: Rect, mouse: [f32; 2]) {
    d.hit(bar, HitKind::DragWindow);

    d.text(
        "zigx",
        Rect::new(bar.x + 20.0, bar.y, 80.0, bar.h),
        WORDMARK,
        theme::INK,
    );

    // Window controls: bare glyphs, ghost disc only on hover.
    let specs = [
        (Icon::Pin, HitKind::ToggleTop),
        (Icon::Min, HitKind::Minimize),
        (Icon::Close, HitKind::Close),
    ];
    let cy = bar.y + bar.h * 0.5;
    let mut left_edge = bar.right();
    for (i, (ic, kind)) in specs.iter().enumerate() {
        let r = Rect::new(
            bar.right() - 12.0 - (3 - i) as f32 * 32.0,
            cy - 14.0,
            28.0,
            28.0,
        );
        left_edge = left_edge.min(r.x);
        let hot = r.contains(mouse[0], mouse[1]);
        let on = matches!(kind, HitKind::ToggleTop) && state.always_on_top;
        let is_close = matches!(kind, HitKind::Close);
        if on {
            d.fill(r, 14.0, theme::SELECTED);
        } else if hot {
            d.fill(r, 14.0, theme::HOVER);
        }
        let color = if hot && is_close {
            theme::DANGER_INK
        } else if on || hot {
            theme::INK
        } else {
            theme::INK_3
        };
        icon(d, *ic, r.x + 7.0, r.y + 7.0, color);
        d.hit(r, *kind);
    }

    // Live readout, instrument style, ahead of the controls.
    let readout = format!("cpu {}   mem {}", cpu_pct(snap.cpu_total), mem_short(snap));
    let rw = measure(&readout.to_uppercase(), MICRO_NUM);
    d.text_r(
        &readout,
        Rect::new(left_edge - 24.0 - rw, bar.y, rw + 2.0, bar.h),
        MICRO_NUM,
        theme::INK_3,
    );
}

fn side_item(
    d: &mut DrawList,
    r: Rect,
    ic: Icon,
    label: &str,
    detail: Option<&str>,
    on: bool,
    mouse: [f32; 2],
) {
    let hot = r.contains(mouse[0], mouse[1]);
    if on {
        d.fill(r, 6.0, theme::GHOST);
    } else if hot {
        d.fill(r, 6.0, theme::HOVER);
    }
    let ink = if on || hot { theme::INK } else { theme::INK_2 };
    let glyph = if on { theme::INK } else { theme::INK_3 };
    icon(d, ic, r.x + 12.0, r.y + (r.h - 14.0) * 0.5, glyph);
    d.text(label, Rect::new(r.x + 36.0, r.y, r.w * 0.55, r.h), NAV, ink);
    if let Some(detail) = detail {
        d.text_r(
            detail,
            Rect::new(r.x + r.w * 0.5, r.y, r.w * 0.5 - 12.0, r.h),
            MICRO_NUM,
            if on { theme::INK_2 } else { theme::INK_4 },
        );
    }
}

fn nav_items(d: &mut DrawList, state: &AppState, nav: Rect, mouse: [f32; 2], sample_ms: f32) {
    eyebrow(d, nav.x + 20.0, nav.y + 18.0, nav.w - 40.0, "Monitor", None);
    let items = [
        (Icon::List, "Processes", Page::Processes),
        (Icon::Pulse, "Performance", Page::Performance),
        (Icon::Launch, "Startup", Page::Startup),
    ];
    let mut y = nav.y + 44.0;
    for (ic, label, page) in items {
        let r = Rect::new(nav.x + 10.0, y, nav.w - 20.0, 34.0);
        side_item(d, r, ic, label, None, state.page == page, mouse);
        d.hit(r, HitKind::Page(page));
        y += 38.0;
    }
    d.text(
        &format!("v{}  ·  {sample_ms:.1} ms", env!("CARGO_PKG_VERSION")),
        Rect::new(nav.x + 20.0, nav.bottom() - 30.0, nav.w - 40.0, 14.0),
        MICRO_NUM,
        theme::INK_4,
    );
}

fn toast(d: &mut DrawList, state: &AppState, main: Rect) {
    let Some(undo) = &state.undo else { return };
    if undo.until <= Instant::now() {
        return;
    }
    let has_revert = undo.revert.is_some();
    let label_w = measure(&undo.label, BODY);
    let w = (label_w + 36.0 + if has_revert { 86.0 } else { 0.0 }).max(120.0);
    let r = Rect::new(main.x + (main.w - w) * 0.5, main.bottom() - 60.0, w, 38.0);
    d.slab(r, 19.0, theme::TOAST, theme::GHOST_LINE, 1.0);
    d.text(
        &undo.label,
        Rect::new(r.x + 18.0, r.y, label_w + 4.0, r.h),
        BODY,
        theme::INK,
    );
    if has_revert {
        let u = Rect::new(r.right() - 74.0, r.y + 7.0, 62.0, 24.0);
        d.fill(u, 12.0, theme::ACCENT);
        d.text_c("Undo", u, PILL_ON, theme::ON_ACCENT);
        d.hit(u, HitKind::Undo);
    }
}

// --- Processes --------------------------------------------------------------

fn processes(d: &mut DrawList, state: &mut AppState, snap: &Snap, main: Rect, mouse: [f32; 2]) {
    let inner = Rect::new(main.x + 24.0, main.y + 20.0, main.w - 48.0, main.h - 44.0);
    let y = inner.y;
    let ctl_h = 30.0;

    // Segmented control: one ghost outline, the active segment is the white pill.
    let views = [
        ("Grouped", ProcView::Grouped),
        ("Flat", ProcView::Flat),
        ("User", ProcView::User),
        ("System", ProcView::System),
    ];
    let pad = 3.0;
    let seg_w: f32 = views
        .iter()
        .map(|(l, _)| measure(l, PILL) + 26.0)
        .sum::<f32>()
        + pad * 2.0;
    let seg = Rect::new(inner.x, y, seg_w, ctl_h);
    d.outline(seg, ctl_h * 0.5, theme::GHOST_LINE);
    let mut x = seg.x + pad;
    for (label, view) in views {
        let w = measure(label, PILL) + 26.0;
        let r = Rect::new(x, y + pad, w, ctl_h - pad * 2.0);
        let on = state.view == view;
        let hot = r.contains(mouse[0], mouse[1]);
        if on {
            d.fill(r, r.h * 0.5, theme::ACCENT);
            d.text_c(label, r, PILL_ON, theme::ON_ACCENT);
        } else {
            d.text_c(label, r, PILL, if hot { theme::INK } else { theme::INK_2 });
        }
        d.hit(r, HitKind::View(view));
        x += w;
    }

    // Right cluster: End task, density, search.
    let end = selection_label(state);
    let ew = measure(&end, PILL) + 32.0;
    let er = Rect::new(inner.right() - ew, y, ew, ctl_h);
    let armed = state
        .armed
        .as_ref()
        .is_some_and(|a| a.until > Instant::now() && a.pids == state.selected);
    ghost_pill(d, er, &end, mouse, !state.selected.is_empty(), armed);
    d.hit(er, HitKind::EndTask);

    let dense = if state.density == Density::Compact {
        "Compact"
    } else {
        "Comfortable"
    };
    let dw = measure(dense, PILL) + 32.0;
    let dr = Rect::new(er.x - 8.0 - dw, y, dw, ctl_h);
    ghost_pill(d, dr, dense, mouse, true, false);
    d.hit(dr, HitKind::Density);

    let search_w = 200.0_f32.min((dr.x - seg.right() - 16.0).max(90.0));
    let sr = Rect::new(dr.x - 8.0 - search_w, y, search_w, ctl_h);
    let focus = state.search_focused;
    let hot = sr.contains(mouse[0], mouse[1]);
    d.slab(
        sr,
        ctl_h * 0.5,
        if focus { theme::GHOST } else { NONE },
        if focus {
            theme::ACCENT_LINE
        } else if hot {
            theme::GHOST_LINE
        } else {
            theme::HAIRLINE
        },
        1.0,
    );
    icon(
        d,
        Icon::Search,
        sr.x + 11.0,
        sr.y + 8.0,
        if focus { theme::INK } else { theme::INK_4 },
    );
    let q = if state.query.is_empty() && !focus {
        "Search".to_string()
    } else if focus {
        format!("{}|", state.query)
    } else {
        state.query.clone()
    };
    let typing = focus || !state.query.is_empty();
    d.text(
        &q,
        Rect::new(sr.x + 32.0, sr.y, sr.w - 42.0, sr.h),
        if typing { NUM } else { PILL },
        if typing { theme::INK } else { theme::INK_4 },
    );
    d.hit(sr, HitKind::Search);

    // Column header.
    let y = y + ctl_h + 20.0;
    let cols = columns(state.density, inner.w);
    let header = Rect::new(inner.x, y, inner.w, 16.0);
    draw_header(d, &cols, header, state.sort);
    d.hairline(Rect::new(inner.x, y + 24.0, inner.w, 1.0));
    let y = y + 25.0;
    let list = Rect::new(inner.x, y, inner.w, (inner.bottom() - y).max(20.0));
    d.list_rect = Some(list);

    // Drop selections whose process has exited so End task cannot hit a recycled PID.
    if !state.selected.is_empty() {
        state
            .selected
            .retain(|pid| snap.procs.iter().any(|p| p.pid == *pid));
    }
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
        let rr = Rect::new(list.x, ry, list.w, row_h);
        match row {
            Row::Header {
                title,
                count,
                open,
                user,
            } => {
                icon(
                    d,
                    if *open {
                        Icon::ChevronDown
                    } else {
                        Icon::ChevronRight
                    },
                    list.x + 2.0,
                    ry + row_h * 0.5 - 7.0,
                    theme::INK_4,
                );
                eyebrow(
                    d,
                    list.x + 24.0,
                    ry + (row_h - 14.0) * 0.5,
                    200.0,
                    title,
                    Some(&count.to_string()),
                );
                d.hit(rr, HitKind::Group(*user));
            }
            Row::Proc(p) => {
                let on = state.selected.contains(&p.pid);
                let hot = rr.contains(mouse[0], mouse[1]);
                if on {
                    d.fill(rr, 6.0, theme::SELECTED);
                } else if hot {
                    d.fill(rr, 6.0, theme::HOVER);
                }
                draw_proc(d, &cols, rr, p);
                d.hit(rr, HitKind::Proc { pid: p.pid });
            }
        }
    }
    d.clip = None;
    scrollbar(d, list, content_h, state.scroll);
    if rows.is_empty() {
        d.text(
            "No matching processes",
            Rect::new(list.x, list.y + 12.0, list.w, 24.0),
            BODY,
            theme::INK_3,
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
            (Col::Pid, 68.0, true, true),
            (Col::Memory, 88.0, true, true),
            (Col::Cpu, 68.0, true, true),
        ]
    } else {
        &[
            (Col::Threads, 76.0, true, true),
            (Col::User, 96.0, false, false),
            (Col::Pid, 72.0, true, true),
            (Col::Disk, 96.0, true, true),
            (Col::Memory, 88.0, true, true),
            (Col::Cpu, 72.0, true, true),
        ]
    };
    let fixed: f32 = spec.iter().map(|(_, w, _, _)| *w).sum();
    let x = width - fixed;
    let mut cols = vec![ColSpec {
        col: Col::Name,
        x: 10.0,
        w: (x - 14.0).max(40.0),
        right: false,
        mono: false,
    }];
    let mut cursor = x;
    for (col, w, right, mono) in spec.iter().rev() {
        cols.push(ColSpec {
            col: *col,
            x: cursor,
            w: *w,
            right: *right,
            mono: *mono,
        });
        cursor += *w;
    }
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

fn draw_header(d: &mut DrawList, cols: &[ColSpec], row: Rect, sort: Sort) {
    for c in cols {
        let r = Rect::new(row.x + c.x, row.y, c.w - 10.0, row.h);
        let active = c.col == sort.col;
        let color = if active { theme::INK } else { theme::INK_3 };
        let title = col_title(c.col);
        if c.right {
            d.text_r(title, r, MICRO, color);
        } else {
            d.text(title, r, MICRO, color);
        }
        if active {
            let tw = measure(&title.to_uppercase(), MICRO);
            let cx = if c.right {
                r.right() - tw - 12.0
            } else {
                r.x + tw + 6.0
            };
            let cy = row.y + row.h * 0.5 - 2.0;
            if sort.desc {
                d.line(
                    &[[cx, cy], [cx + 3.0, cy + 3.5], [cx + 6.0, cy]],
                    1.1,
                    theme::INK_2,
                );
            } else {
                d.line(
                    &[[cx, cy + 3.5], [cx + 3.0, cy], [cx + 6.0, cy + 3.5]],
                    1.1,
                    theme::INK_2,
                );
            }
        }
        d.hit(
            Rect::new(row.x + c.x, row.y - 4.0, c.w, row.h + 8.0),
            HitKind::Sort(c.col),
        );
    }
}

fn draw_proc(d: &mut DrawList, cols: &[ColSpec], row: Rect, p: &Proc) {
    for c in cols {
        let r = Rect::new(row.x + c.x, row.y, c.w - 10.0, row.h);
        let text = match c.col {
            Col::Name => p.name.clone(),
            Col::Cpu => cpu_pct(p.cpu),
            Col::Memory => bytes(p.rss),
            Col::Disk => disk_cell(p.read_bps, p.write_bps),
            Col::Pid => p.pid.to_string(),
            Col::User => p.user.clone(),
            Col::Threads => p.threads.to_string(),
        };
        // Idle cells step back so the rows doing something read first.
        let idle = match c.col {
            Col::Cpu => p.cpu < 0.05,
            Col::Disk => disk_sum(p) < 1.0,
            _ => false,
        };
        let color = match c.col {
            Col::Name => theme::INK,
            Col::Cpu if idle => theme::INK_4,
            Col::Cpu => heat(p.cpu),
            _ if idle => theme::INK_4,
            _ => theme::INK_2,
        };
        let t = if c.mono { NUM } else { BODY };
        if c.right {
            d.text_r(&text, r, t, color);
        } else {
            d.text(&text, r, t, color);
        }
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
    d.fill(
        Rect::new(viewport.right() - 3.0, y, 2.0, thumb_h),
        1.0,
        theme::INK_4,
    );
}

// --- Performance ------------------------------------------------------------

fn performance(
    d: &mut DrawList,
    state: &mut AppState,
    snap: &Snap,
    sub: Rect,
    detail: Rect,
    mouse: [f32; 2],
) {
    eyebrow(
        d,
        sub.x + 20.0,
        sub.y + 18.0,
        sub.w - 40.0,
        "Resources",
        None,
    );
    let items = [
        (
            Icon::Chip,
            "CPU",
            Section::Cpu,
            Some(percent(snap.cpu_total)),
        ),
        (Icon::Mem, "Memory", Section::Memory, Some(mem_short(snap))),
        (
            Icon::Gpu,
            "GPU",
            Section::Gpu,
            snap.gpus.first().and_then(|g| g.util).map(percent),
        ),
        (Icon::Disk, "Disk", Section::Disk, None),
        (Icon::Net, "Network", Section::Net, None),
    ];
    let mut y = sub.y + 44.0;
    for (ic, label, section, extra) in items {
        let r = Rect::new(sub.x + 10.0, y, sub.w - 20.0, 34.0);
        side_item(
            d,
            r,
            ic,
            label,
            extra.as_deref(),
            state.section == section,
            mouse,
        );
        d.hit(r, HitKind::Section(section));
        y += 38.0;
    }

    let view = Rect::new(
        detail.x + 28.0,
        detail.y + 22.0,
        detail.w - 56.0,
        detail.h - 46.0,
    );
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

fn window_label() -> String {
    format!("{} s window", crate::model::HIST_SECS)
}

fn mem_short(snap: &Snap) -> String {
    if snap.mem_total == 0 {
        "—".into()
    } else {
        percent(snap.mem_used as f32 / snap.mem_total as f32 * 100.0)
    }
}

fn page_title(d: &mut DrawList, title: &str, sub: &str, view: Rect, y: f32) -> f32 {
    d.text(title, Rect::new(view.x, y, view.w, 28.0), TITLE, theme::INK);
    d.text(
        sub,
        Rect::new(view.x, y + 30.0, view.w, 16.0),
        SUB,
        theme::INK_3,
    );
    y + 62.0
}

/// Big mono readout with a micro caption. Returns the y after it.
fn readout(d: &mut DrawList, view: Rect, y: f32, value: &str, caption: &str) -> f32 {
    d.text(
        value,
        Rect::new(view.x, y, view.w, 52.0),
        READOUT,
        theme::INK,
    );
    d.text(
        caption,
        Rect::new(view.x, y + 56.0, view.w, 14.0),
        MICRO,
        theme::INK_3,
    );
    y + 88.0
}

/// Graph with a scale note in the top-right corner.
fn scaled_graph(
    d: &mut DrawList,
    r: Rect,
    series: &[(&[f32], theme::Rgba)],
    max: f32,
    max_label: &str,
    dot: bool,
) {
    d.graph(r, series, max, dot);
    d.text_r(
        max_label,
        Rect::new(r.x, r.y - 16.0, r.w, 14.0),
        MICRO_NUM,
        theme::INK_4,
    );
}

fn cpu_page(d: &mut DrawList, snap: &Snap, view: Rect, mut y: f32) -> f32 {
    let top = y;
    y = page_title(d, "Processor", &snap.cpu_model, view, y);
    y = readout(d, view, y, &percent(snap.cpu_total), "Total utilization");

    let freq: f32 = if snap.cpu_freq_mhz.is_empty() {
        0.0
    } else {
        snap.cpu_freq_mhz.iter().sum::<f32>() / snap.cpu_freq_mhz.len() as f32
    };
    let stats = [
        (freq_ghz(freq), "Clock"),
        (format!("{}", snap.proc_count), "Processes"),
        (format!("{}", snap.thread_count), "Threads"),
        (duration(snap.uptime_secs), "Uptime"),
        (format!("{:.2}", snap.load[0]), "Load"),
    ];
    let slot = view.w / 5.0;
    for (i, (v, k)) in stats.iter().enumerate() {
        stat(d, view.x + i as f32 * slot, y, slot - 16.0, v, k);
    }
    y += 64.0;

    // Telemetry grows with the sheet: graph takes the larger share, cores the rest.
    let n = snap.cpu_per.len();
    let spare = spare_height(
        view,
        y - top,
        24.0 + 28.0 + if n > 0 { 24.0 + 16.0 } else { 0.0 },
    );
    let graph_h = (spare * if n > 0 { 0.62 } else { 1.0 }).clamp(110.0, 320.0);
    let cores_h = (spare * 0.38).clamp(60.0, 150.0);

    eyebrow(d, view.x, y, view.w, "Utilization", Some(&window_label()));
    y += 24.0;
    scaled_graph(
        d,
        Rect::new(view.x, y, view.w, graph_h),
        &[(&snap.cpu_hist, theme::TRACE)],
        100.0,
        "100%",
        true,
    );
    y += graph_h + 28.0;

    if n == 0 {
        return y;
    }
    let peak = snap.cpu_per.iter().copied().fold(0.0_f32, f32::max);
    eyebrow(
        d,
        view.x,
        y,
        view.w,
        "Cores",
        Some(&format!("{n} logical   peak {}", cpu_pct(peak))),
    );
    y += 24.0;
    equalizer(d, Rect::new(view.x, y, view.w, cores_h), &snap.cpu_per);
    y + cores_h + 16.0
}

/// Vertical room left in the viewport once `used` content height and
/// `reserved` fixed chrome are laid out. Uses content height, not the
/// scrolled y, so graph sizes stay stable while the user scrolls.
fn spare_height(view: Rect, used: f32, reserved: f32) -> f32 {
    (view.h - used - reserved).max(0.0)
}

/// Per-core usage as thin bars on a hairline.
fn equalizer(d: &mut DrawList, r: Rect, values: &[f32]) {
    let n = values.len();
    if n == 0 {
        return;
    }
    let slot = r.w / n as f32;
    let bw = (slot * 0.42).clamp(2.0, 10.0);
    for (i, v) in values.iter().enumerate() {
        let h = ((v / 100.0).clamp(0.0, 1.0) * r.h).max(2.0);
        let x = r.x + i as f32 * slot + (slot - bw) * 0.5;
        let color = if *v >= 90.0 {
            theme::HOT
        } else if *v >= 70.0 {
            theme::WARN
        } else {
            theme::INK_2
        };
        d.fill(Rect::new(x, r.bottom() - h, bw, h), 1.0, color);
    }
    d.hairline(Rect::new(r.x, r.bottom(), r.w, 1.0));
}

fn memory_page(d: &mut DrawList, snap: &Snap, view: Rect, mut y: f32) -> f32 {
    let top = y;
    y = page_title(d, "Memory", "Physical RAM", view, y);
    let used = bytes(snap.mem_used);
    d.text(
        &used,
        Rect::new(view.x, y, view.w, 52.0),
        READOUT,
        theme::INK,
    );
    let uw = measure(&used, READOUT);
    d.text(
        &format!("/ {}", bytes(snap.mem_total)),
        Rect::new(view.x + uw + 12.0, y + 24.0, view.w - uw - 12.0, 24.0),
        READOUT_SUB,
        theme::INK_3,
    );
    d.text(
        "In use",
        Rect::new(view.x, y + 56.0, view.w, 14.0),
        MICRO,
        theme::INK_3,
    );
    y += 88.0;

    let stats = [
        (bytes(snap.mem_available), "Available"),
        (bytes(snap.mem_cached), "Cached"),
        (bytes(snap.mem_buffers), "Buffers"),
        (mem_short(snap), "Used"),
    ];
    let slot = view.w / 4.0;
    for (i, (v, k)) in stats.iter().enumerate() {
        stat(d, view.x + i as f32 * slot, y, slot - 16.0, v, k);
    }
    y += 64.0;

    let has_swap = snap.swap_total > 0;
    let spare = spare_height(
        view,
        y - top,
        24.0 + 28.0 + if has_swap { 24.0 + 20.0 } else { 0.0 },
    );
    let graph_h = (spare * if has_swap { 0.68 } else { 1.0 }).clamp(110.0, 320.0);
    let swap_h = (spare * 0.32).clamp(56.0, 120.0);

    eyebrow(d, view.x, y, view.w, "Usage", Some(&window_label()));
    y += 24.0;
    scaled_graph(
        d,
        Rect::new(view.x, y, view.w, graph_h),
        &[(&snap.mem_hist, theme::TRACE)],
        1.0,
        &bytes(snap.mem_total),
        true,
    );
    y += graph_h + 28.0;

    if has_swap {
        eyebrow(
            d,
            view.x,
            y,
            view.w,
            "Swap",
            Some(&format!(
                "{} / {}",
                bytes(snap.swap_used),
                bytes(snap.swap_total)
            )),
        );
        y += 24.0;
        d.graph(
            Rect::new(view.x, y, view.w, swap_h),
            &[(&snap.swap_hist, theme::TRACE_2)],
            1.0,
            false,
        );
        y += swap_h + 20.0;
    }
    y
}

fn gpu_page(d: &mut DrawList, snap: &Snap, view: Rect, mut y: f32) -> f32 {
    let top = y;
    if snap.gpus.is_empty() {
        y = page_title(d, "Graphics", "No GPU reported", view, y);
        return y;
    }
    for g in &snap.gpus {
        y = page_title(d, "Graphics", &g.name, view, y);
        let util = g.util.map(percent).unwrap_or_else(|| "—".into());
        y = readout(d, view, y, &util, "Utilization");

        // Telemetry as a stat row.
        let mut stats: Vec<(String, &str)> = Vec::new();
        if let Some(t) = g.temp_c {
            stats.push((format!("{t} °C"), "Temp"));
        }
        if let Some(p) = g.power_w {
            stats.push((format!("{p:.0} W"), "Power"));
        }
        if let Some(c) = g.clk_core {
            stats.push((format!("{c} MHz"), "Core"));
        }
        if let Some(c) = g.clk_mem {
            stats.push((format!("{c} MHz"), "Mem clock"));
        }
        if let Some(e) = g.enc {
            stats.push((format!("{e}%"), "Encode"));
        }
        if let Some(e) = g.dec {
            stats.push((format!("{e}%"), "Decode"));
        }
        if g.integrated {
            stats.push(("Integrated".into(), "Type"));
        }
        if !stats.is_empty() {
            let cols = stats.len().min(5);
            let slot = view.w / cols as f32;
            for (i, (v, k)) in stats.iter().take(cols).enumerate() {
                stat(d, view.x + i as f32 * slot, y, slot - 16.0, v, k);
            }
            y += 64.0;
        }

        // A single GPU gets the whole sheet; several share fixed panels.
        let has_vram = g.mem_total > 0;
        let graph_h = if snap.gpus.len() == 1 {
            spare_height(
                view,
                y - top,
                24.0 + 28.0 + if has_vram { 52.0 } else { 0.0 },
            )
            .clamp(100.0, 320.0)
        } else {
            120.0
        };
        eyebrow(d, view.x, y, view.w, "Utilization", Some(&window_label()));
        y += 24.0;
        scaled_graph(
            d,
            Rect::new(view.x, y, view.w, graph_h),
            &[(g.util_hist.as_slice(), theme::TRACE)],
            100.0,
            "100%",
            true,
        );
        y += graph_h + 28.0;

        if g.mem_total > 0 {
            let frac = (g.mem_used as f32 / g.mem_total as f32).clamp(0.0, 1.0);
            eyebrow(
                d,
                view.x,
                y,
                view.w,
                "VRAM",
                Some(&format!("{} / {}", bytes(g.mem_used), bytes(g.mem_total))),
            );
            y += 22.0;
            d.fill(Rect::new(view.x, y, view.w, 2.0), 1.0, theme::GRID);
            d.fill(
                Rect::new(view.x, y, (view.w * frac).max(2.0), 2.0),
                1.0,
                theme::INK,
            );
            y += 30.0;
        }
    }
    y
}

fn io_page(d: &mut DrawList, view: Rect, mut y: f32, disk: bool, snap: &Snap) -> f32 {
    type Dev<'a> = (&'a str, f64, f64, &'a [f32], &'a [f32]);
    let top = y;
    let (title, sub, a_legend, b_legend, la, lb) = if disk {
        ("Disk", "Block devices", "read", "write", "R", "W")
    } else {
        ("Network", "Interfaces", "receive", "transmit", "RX", "TX")
    };
    let devs: Vec<Dev> = if disk {
        snap.disks
            .iter()
            .map(|d| {
                (
                    d.name.as_str(),
                    d.read_bps,
                    d.write_bps,
                    d.read_hist.as_slice(),
                    d.write_hist.as_slice(),
                )
            })
            .collect()
    } else {
        snap.nets
            .iter()
            .map(|n| {
                (
                    n.name.as_str(),
                    n.rx_bps,
                    n.tx_bps,
                    n.rx_hist.as_slice(),
                    n.tx_hist.as_slice(),
                )
            })
            .collect()
    };
    if devs.is_empty() {
        return page_title(
            d,
            title,
            if disk { "No disks" } else { "No interfaces" },
            view,
            y,
        );
    }

    // Headline readout: totals across devices.
    let (sum_a, sum_b) = devs
        .iter()
        .fold((0.0, 0.0), |acc, dv| (acc.0 + dv.1, acc.1 + dv.2));
    y = page_title(d, title, sub, view, y);
    let slot = view.w / 4.0;
    stat(d, view.x, y, slot - 16.0, &rate(sum_a), a_legend);
    stat(d, view.x + slot, y, slot - 16.0, &rate(sum_b), b_legend);
    stat(
        d,
        view.x + slot * 2.0,
        y,
        slot - 16.0,
        &devs.len().to_string(),
        if disk { "Devices" } else { "Interfaces" },
    );
    y += 64.0;

    // Split the remaining sheet evenly between devices.
    let n = devs.len() as f32;
    let per_dev = spare_height(view, y - top, 0.0) / n;
    let graph_h = (per_dev - 26.0 - 24.0).clamp(80.0, 320.0);

    for (name, a_bps, b_bps, a_hist, b_hist) in devs {
        d.text(
            name,
            Rect::new(view.x, y, view.w * 0.5, 18.0),
            NUM,
            theme::INK,
        );
        d.text_r(
            &format!("{la} {}     {lb} {}", rate(a_bps), rate(b_bps)),
            Rect::new(view.x + view.w * 0.4, y, view.w * 0.6, 18.0),
            NUM_SMALL,
            theme::INK_3,
        );
        y += 26.0;
        let max = nice_pair(a_hist, b_hist);
        let gr = Rect::new(view.x, y, view.w, graph_h);
        d.graph(
            gr,
            &[(a_hist, theme::TRACE), (b_hist, theme::TRACE_2)],
            max,
            false,
        );
        // Legend, top left; scale, top right.
        let ly = gr.y + 6.0;
        let mut lx = gr.x;
        d.line(&[[lx, ly + 6.0], [lx + 12.0, ly + 6.0]], 1.25, theme::TRACE);
        lx += 17.0;
        d.text(a_legend, Rect::new(lx, ly, 90.0, 12.0), MICRO, theme::INK_3);
        lx += measure(&a_legend.to_uppercase(), MICRO) + 16.0;
        d.line(
            &[[lx, ly + 6.0], [lx + 12.0, ly + 6.0]],
            1.25,
            theme::TRACE_2,
        );
        lx += 17.0;
        d.text(b_legend, Rect::new(lx, ly, 90.0, 12.0), MICRO, theme::INK_3);
        d.text_r(
            &rate(max as f64),
            Rect::new(gr.x, ly, gr.w, 12.0),
            MICRO_NUM,
            theme::INK_4,
        );
        y += graph_h + 24.0;
    }
    y
}

fn nice_pair(a: &[f32], b: &[f32]) -> f32 {
    let m = a.iter().chain(b.iter()).copied().fold(0.0_f32, f32::max);
    format::nice_ceil(m)
}

// --- Startup ----------------------------------------------------------------

fn startup_page(
    d: &mut DrawList,
    state: &mut AppState,
    startup: &[StartupEntry],
    main: Rect,
    mouse: [f32; 2],
) {
    let inner = Rect::new(main.x + 28.0, main.y + 22.0, main.w - 56.0, main.h - 46.0);
    let enabled = startup.iter().filter(|e| e.enabled).count();
    let y = page_title(
        d,
        "Startup",
        "Session autostart. Off writes Hidden=true to ~/.config/autostart and keeps a .bak.",
        inner,
        inner.y,
    );
    eyebrow(
        d,
        inner.x,
        y,
        inner.w,
        "Entries",
        Some(&format!("{enabled} of {} on", startup.len())),
    );
    let y = y + 22.0;
    d.hairline(Rect::new(inner.x, y, inner.w, 1.0));
    let list = Rect::new(
        inner.x,
        y + 1.0,
        inner.w,
        (inner.bottom() - y - 1.0).max(20.0),
    );
    d.startup_rect = Some(list);
    if startup.is_empty() {
        d.text(
            "No autostart entries found",
            Rect::new(list.x, list.y + 12.0, list.w, 24.0),
            BODY,
            theme::INK_3,
        );
        return;
    }
    let row_h = 52.0;
    let content_h = startup.len() as f32 * row_h;
    let max_scroll = (content_h - list.h).max(0.0);
    if state.startup_scroll > max_scroll {
        state.startup_scroll = max_scroll;
    }
    let first = (state.startup_scroll / row_h).floor() as usize;
    let nvis = ((list.h / row_h).ceil() as usize) + 2;
    d.clip = Some(list);
    for (i, entry) in startup.iter().enumerate().skip(first).take(nvis) {
        let ry = list.y + i as f32 * row_h - state.startup_scroll;
        if ry + row_h < list.y || ry > list.bottom() {
            continue;
        }
        let rr = Rect::new(list.x, ry, list.w, row_h);
        if rr.contains(mouse[0], mouse[1]) {
            d.fill(
                Rect::new(list.x - 10.0, ry, list.w + 20.0, row_h),
                6.0,
                theme::HOVER,
            );
        }
        let name_ink = if entry.enabled {
            theme::INK
        } else {
            theme::INK_2
        };
        let name_w = measure(&entry.name, BODY).min(list.w - 120.0);
        d.text(
            &entry.name,
            Rect::new(list.x, ry + 9.0, list.w - 120.0, 18.0),
            BODY,
            name_ink,
        );
        if entry.system_path.is_some() {
            d.text(
                "system",
                Rect::new(list.x + name_w + 10.0, ry + 10.0, 60.0, 16.0),
                MICRO,
                theme::INK_4,
            );
        }
        d.text(
            if entry.exec.is_empty() {
                "no Exec line"
            } else {
                &entry.exec
            },
            Rect::new(list.x, ry + 28.0, list.w - 120.0, 16.0),
            NUM_SMALL,
            if entry.enabled {
                theme::INK_3
            } else {
                theme::INK_4
            },
        );
        d.hairline(Rect::new(list.x, ry + row_h - 1.0, list.w, 1.0));
        switch(
            d,
            Rect::new(list.right() - 36.0, ry + 17.0, 32.0, 18.0),
            entry.enabled,
        );
        d.hit(rr, HitKind::Startup(i));
    }
    d.clip = None;
    scrollbar(d, list, content_h, state.startup_scroll);
}

fn switch(d: &mut DrawList, r: Rect, on: bool) {
    if on {
        d.fill(r, r.h * 0.5, theme::ACCENT);
        d.fill(
            Rect::new(r.right() - 15.0, r.y + 3.0, 12.0, 12.0),
            6.0,
            theme::ON_ACCENT,
        );
    } else {
        d.outline(r, r.h * 0.5, theme::GHOST_LINE);
        d.fill(
            Rect::new(r.x + 3.0, r.y + 3.0, 12.0, 12.0),
            6.0,
            theme::INK_3,
        );
    }
}
