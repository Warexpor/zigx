use std::time::Instant;

use crate::format::{
    self, bytes, cpu_pct, disk_cell, duration, fit, freq_ghz, percent, rate, text_width,
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
        self.slab_ex(r, radius, fill, border, border_w, 0.0, 0.0);
    }

    #[allow(clippy::too_many_arguments)]
    fn slab_ex(
        &mut self,
        r: Rect,
        radius: f32,
        fill: theme::Rgba,
        border: theme::Rgba,
        border_w: f32,
        shadow: f32,
        shadow_a: f32,
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
            shadow,
            shadow_a,
        });
    }

    /// 1px hairline, the only hard edge in the system.
    fn hairline(&mut self, r: Rect) {
        self.slab(r, 0.0, theme::DIVIDER, [0, 0, 0, 0], 0.0);
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

    fn text(&mut self, text: &str, r: Rect, size: f32, color: theme::Rgba, mono: bool, right: bool) {
        self.textw(text, r, size, color, mono, right, 400);
    }

    #[allow(clippy::too_many_arguments)]
    fn textw(
        &mut self,
        text: &str,
        r: Rect,
        size: f32,
        color: theme::Rgba,
        mono: bool,
        right: bool,
        weight: u16,
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
            weight,
        });
    }

    /// Center the most recent label inside `r`.
    fn center_last(&mut self, r: Rect, size: f32, mono: bool) {
        if let Some(last) = self.labels.last_mut() {
            let tw = text_width(&last.text, size, mono);
            last.x = r.x + (r.w - tw) * 0.5;
        }
    }

    fn graph(&mut self, r: Rect, series: &[(&[f32], theme::Rgba)], max: f32, dot: bool) {
        if r.w < 4.0 || r.h < 4.0 || !self.visible(r) {
            return;
        }
        let max = max.max(0.001);
        self.slab(
            Rect::new(r.x, r.bottom(), r.w, 1.0),
            0.0,
            theme::DIVIDER,
            [0, 0, 0, 0],
            0.0,
        );
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
                width: 1.5,
                color: *color,
                baseline: Some(r.bottom()),
            });
            if dot && si == 0 {
                if let Some(p) = pts.last() {
                    self.slab(
                        Rect::new(p[0] - 2.0, p[1] - 2.0, 4.0, 4.0),
                        2.0,
                        *color,
                        [0, 0, 0, 0],
                        0.0,
                    );
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

/// Draw a 14x14 line icon at (x, y).
fn icon(d: &mut DrawList, kind: Icon, x: f32, y: f32, c: theme::Rgba) {
    match kind {
        Icon::List => {
            for i in 0..3 {
                let yy = y + 2.5 + i as f32 * 4.0;
                d.line(&[[x + 1.0, yy], [x + 3.0, yy]], 1.4, c);
                d.line(&[[x + 5.5, yy], [x + 13.0, yy]], 1.4, c);
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
            1.4,
            c,
        ),
        Icon::Launch => {
            d.line(&[[x + 3.0, y + 11.0], [x + 11.0, y + 3.0]], 1.4, c);
            d.line(
                &[[x + 5.5, y + 3.0], [x + 11.0, y + 3.0], [x + 11.0, y + 8.5]],
                1.4,
                c,
            );
        }
        Icon::Chip => {
            d.slab(Rect::new(x + 2.0, y + 2.0, 10.0, 10.0), 2.5, [0, 0, 0, 0], c, 1.2);
            d.slab(
                Rect::new(x + 5.75, y + 5.75, 2.5, 2.5),
                0.8,
                c,
                [0, 0, 0, 0],
                0.0,
            );
        }
        Icon::Mem => {
            d.slab(Rect::new(x + 2.0, y + 7.0, 2.2, 5.0), 1.1, c, [0, 0, 0, 0], 0.0);
            d.slab(Rect::new(x + 5.9, y + 4.5, 2.2, 7.5), 1.1, c, [0, 0, 0, 0], 0.0);
            d.slab(Rect::new(x + 9.8, y + 2.0, 2.2, 10.0), 1.1, c, [0, 0, 0, 0], 0.0);
        }
        Icon::Gpu => {
            d.slab(Rect::new(x + 1.0, y + 3.5, 12.0, 8.0), 2.5, [0, 0, 0, 0], c, 1.2);
            d.slab(Rect::new(x + 6.0, y + 6.5, 2.0, 2.0), 1.0, c, [0, 0, 0, 0], 0.0);
        }
        Icon::Disk => {
            d.slab(Rect::new(x + 2.0, y + 2.0, 10.0, 10.0), 5.0, [0, 0, 0, 0], c, 1.2);
            d.slab(Rect::new(x + 6.0, y + 6.0, 2.0, 2.0), 1.0, c, [0, 0, 0, 0], 0.0);
        }
        Icon::Net => {
            d.line(&[[x + 4.0, y + 2.5], [x + 4.0, y + 9.5]], 1.3, c);
            d.line(
                &[[x + 1.8, y + 7.0], [x + 4.0, y + 9.5], [x + 6.2, y + 7.0]],
                1.3,
                c,
            );
            d.line(&[[x + 10.0, y + 4.5], [x + 10.0, y + 11.5]], 1.3, c);
            d.line(
                &[[x + 7.8, y + 7.0], [x + 10.0, y + 4.5], [x + 12.2, y + 7.0]],
                1.3,
                c,
            );
        }
        Icon::Search => {
            d.slab(Rect::new(x + 2.0, y + 2.0, 8.0, 8.0), 4.0, [0, 0, 0, 0], c, 1.3);
            d.line(&[[x + 8.8, y + 8.8], [x + 12.5, y + 12.5]], 1.4, c);
        }
        Icon::Pin => {
            d.slab(Rect::new(x + 4.5, y + 1.5, 5.0, 5.0), 2.5, [0, 0, 0, 0], c, 1.3);
            d.line(&[[x + 7.0, y + 6.5], [x + 7.0, y + 12.5]], 1.3, c);
        }
        Icon::Min => d.line(&[[x + 3.0, y + 7.0], [x + 11.0, y + 7.0]], 1.4, c),
        Icon::Close => {
            d.line(&[[x + 3.5, y + 3.5], [x + 10.5, y + 10.5]], 1.4, c);
            d.line(&[[x + 10.5, y + 3.5], [x + 3.5, y + 10.5]], 1.4, c);
        }
        Icon::ChevronDown => d.line(
            &[[x + 3.0, y + 5.0], [x + 7.0, y + 9.0], [x + 11.0, y + 5.0]],
            1.4,
            c,
        ),
        Icon::ChevronRight => d.line(
            &[[x + 5.0, y + 3.0], [x + 9.0, y + 7.0], [x + 5.0, y + 11.0]],
            1.4,
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

/// Soft pill button: rounded-full, whisper fill, 12px label.
fn pill_button(
    d: &mut DrawList,
    r: Rect,
    label: &str,
    mouse: [f32; 2],
    enabled: bool,
    danger: bool,
) {
    let hot = r.contains(mouse[0], mouse[1]) && enabled;
    let (fill, ink) = if danger {
        (theme::DANGER_SOFT, theme::DANGER_INK)
    } else {
        (
            if hot { theme::HOVER } else { theme::SOFT },
            if !enabled {
                theme::FAINT
            } else if hot {
                theme::INK
            } else {
                theme::DIM
            },
        )
    };
    d.slab(r, r.h * 0.5, fill, [0, 0, 0, 0], 0.0);
    d.textw(label, r, 12.0, ink, false, false, 500);
    d.center_last(r, 12.0, false);
}

/// x.ai stat pattern: mono value on top, quiet label beneath. No box.
fn stat(d: &mut DrawList, x: f32, y: f32, w: f32, value: &str, label: &str) {
    d.textw(value, Rect::new(x, y, w, 20.0), 14.0, theme::INK, true, false, 500);
    d.text(label, Rect::new(x, y + 22.0, w, 14.0), 10.5, theme::FAINT, false, false);
}

// --- Frame -----------------------------------------------------------------

pub fn build(
    state: &mut AppState,
    snap: &Snap,
    startup: &[StartupEntry],
    mouse: [f32; 2],
) -> DrawList {
    let mut d = DrawList::new();
    let w = state.width.max(420.0);
    let h = state.height.max(320.0);

    // One window, one jet-glass surface.
    let root = Rect::new(0.0, 0.0, w, h);
    d.slab(root, 12.0, theme::WELL, theme::WELL_BORDER, 1.0);

    let bar = Rect::new(0.0, 0.0, w, 54.0);
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

    d.hit(Rect::new(nav.right() - 2.0, body.y, 5.0, body.h), HitKind::DragNav);

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

    d.textw(
        "zigx",
        Rect::new(bar.x + 20.0, bar.y, 60.0, bar.h),
        14.0,
        theme::INK,
        false,
        false,
        600,
    );

    // Live readout ahead of the window controls.
    let stat_text = format!("cpu {} · mem {}", cpu_pct(snap.cpu_total), mem_short(snap));
    let sw = text_width(&stat_text, 10.5, true);
    d.text(
        &stat_text,
        Rect::new(
            bar.right() - 14.0 - 3.0 * 34.0 - 20.0 - sw,
            bar.y,
            sw + 4.0,
            bar.h,
        ),
        10.5,
        theme::MUTED,
        true,
        false,
    );

    let specs = [
        (Icon::Pin, HitKind::ToggleTop),
        (Icon::Min, HitKind::Minimize),
        (Icon::Close, HitKind::Close),
    ];
    let cy = bar.y + bar.h * 0.5;
    for (i, (ic, kind)) in specs.iter().enumerate() {
        let r = Rect::new(
            bar.right() - 14.0 - (3 - i) as f32 * 34.0,
            cy - 14.0,
            28.0,
            28.0,
        );
        let hot = r.contains(mouse[0], mouse[1]);
        let on = matches!(kind, HitKind::ToggleTop) && state.always_on_top;
        let is_close = matches!(kind, HitKind::Close);
        if on {
            d.slab(r, 14.0, theme::SELECTED, [0, 0, 0, 0], 0.0);
        } else if hot {
            d.slab(
                r,
                14.0,
                if is_close {
                    theme::DANGER_SOFT
                } else {
                    theme::HOVER
                },
                [0, 0, 0, 0],
                0.0,
            );
        }
        let color = if on {
            theme::INK
        } else if hot {
            if is_close {
                theme::DANGER_INK
            } else {
                theme::INK
            }
        } else {
            theme::MUTED
        };
        icon(d, *ic, r.x + 7.0, r.y + 7.0, color);
        d.hit(r, *kind);
    }
}

fn nav_items(d: &mut DrawList, state: &AppState, nav: Rect, mouse: [f32; 2], sample_ms: f32) {
    d.textw(
        "Monitor",
        Rect::new(nav.x + 20.0, nav.y + 16.0, nav.w - 40.0, 16.0),
        11.0,
        theme::FAINT,
        false,
        false,
        500,
    );
    let items = [
        (Icon::List, "Processes", Page::Processes),
        (Icon::Pulse, "Performance", Page::Performance),
        (Icon::Launch, "Startup", Page::Startup),
    ];
    let mut y = nav.y + 44.0;
    for (ic, label, page) in items {
        let r = Rect::new(nav.x + 8.0, y, nav.w - 16.0, 36.0);
        let on = state.page == page;
        let hot = r.contains(mouse[0], mouse[1]);
        if on {
            d.slab(r, 9.0, theme::SELECTED, [0, 0, 0, 0], 0.0);
        } else if hot {
            d.slab(r, 9.0, theme::HOVER, [0, 0, 0, 0], 0.0);
        }
        let color = if on {
            theme::INK
        } else if hot {
            theme::DIM
        } else {
            theme::MUTED
        };
        icon(d, ic, r.x + 12.0, r.y + 11.0, color);
        d.textw(
            label,
            Rect::new(r.x + 38.0, r.y, r.w - 48.0, r.h),
            13.0,
            color,
            false,
            false,
            if on { 500 } else { 400 },
        );
        d.hit(r, HitKind::Page(page));
        y += 40.0;
    }
    d.text(
        &format!("v0.2 · {sample_ms:.1} ms"),
        Rect::new(nav.x + 20.0, nav.bottom() - 26.0, nav.w - 40.0, 14.0),
        10.0,
        theme::FAINT,
        true,
        false,
    );
}

fn toast(d: &mut DrawList, state: &AppState, main: Rect) {
    let Some(undo) = &state.undo else { return };
    if undo.until <= Instant::now() {
        return;
    }
    let has_revert = undo.revert.is_some();
    let label_w = text_width(&undo.label, 12.5, false);
    let w = label_w + 32.0 + if has_revert { 88.0 } else { 0.0 };
    let r = Rect::new(
        main.x + (main.w - w) * 0.5,
        main.bottom() - 56.0,
        w.max(120.0),
        40.0,
    );
    d.slab_ex(r, 20.0, theme::TOAST, theme::SOFT_BORDER, 1.0, 14.0, 0.45);
    d.text(
        &undo.label,
        Rect::new(r.x + 16.0, r.y, label_w + 4.0, r.h),
        12.5,
        theme::INK,
        false,
        false,
    );
    if has_revert {
        // Inverted primary: solid white pill, near-black text.
        let u = Rect::new(r.right() - 72.0, r.y + 7.0, 60.0, 26.0);
        d.slab(u, 13.0, theme::ACCENT, [0, 0, 0, 0], 0.0);
        d.textw("Undo", u, 11.5, theme::ON_ACCENT, false, false, 600);
        d.center_last(u, 11.5, false);
        d.hit(u, HitKind::Undo);
    }
}

// --- Processes --------------------------------------------------------------

fn processes(d: &mut DrawList, state: &mut AppState, snap: &Snap, main: Rect, mouse: [f32; 2]) {
    let inner = main.inset(24.0);
    let mut y = inner.y + 4.0;

    // Segmented control: charcoal pill, active segment inverted to solid white.
    let views = [
        ("Grouped", ProcView::Grouped),
        ("Flat", ProcView::Flat),
        ("User", ProcView::User),
        ("System", ProcView::System),
    ];
    let seg_h = 32.0;
    let mut seg_w = 8.0;
    for (label, _) in views {
        seg_w += text_width(label, 12.0, false) + 24.0 + 2.0;
    }
    let seg = Rect::new(inner.x, y, seg_w, seg_h);
    d.slab(seg, seg_h * 0.5, theme::CARD, [0, 0, 0, 0], 0.0);
    let mut x = seg.x + 4.0;
    for (label, view) in views {
        let w = text_width(label, 12.0, false) + 24.0;
        let r = Rect::new(x, y + 4.0, w, seg_h - 8.0);
        let on = state.view == view;
        let hot = r.contains(mouse[0], mouse[1]);
        if on {
            d.slab(r, r.h * 0.5, theme::ACCENT, [0, 0, 0, 0], 0.0);
        }
        d.textw(
            label,
            r,
            12.0,
            if on {
                theme::ON_ACCENT
            } else if hot {
                theme::INK
            } else {
                theme::MUTED
            },
            false,
            false,
            500,
        );
        d.center_last(r, 12.0, false);
        d.hit(r, HitKind::View(view));
        x += w + 2.0;
    }

    // End task, soft pill on the right.
    let end = selection_label(state);
    let ew = text_width(&end, 12.0, false) + 30.0;
    let er = Rect::new(inner.right() - ew, y, ew, 32.0);
    let armed = state
        .armed
        .as_ref()
        .is_some_and(|a| a.until > Instant::now() && a.pids == state.selected);
    pill_button(d, er, &end, mouse, !state.selected.is_empty(), armed);
    d.hit(er, HitKind::EndTask);

    // Density toggle.
    let dense = if state.density == Density::Compact {
        "Compact"
    } else {
        "Comfortable"
    };
    let dw = text_width(dense, 12.0, false) + 30.0;
    let dr = Rect::new(er.x - 8.0 - dw, y, dw, 32.0);
    pill_button(d, dr, dense, mouse, true, false);
    d.hit(dr, HitKind::Density);

    // Search: charcoal pill, border brightens on focus.
    let search_w = 200.0_f32.min((dr.x - seg.right() - 16.0).max(90.0));
    let sr = Rect::new(dr.x - 8.0 - search_w, y, search_w, 32.0);
    let focus = state.search_focused;
    d.slab(
        sr,
        16.0,
        theme::CARD,
        if focus {
            theme::ACCENT_LINE
        } else {
            [0, 0, 0, 0]
        },
        1.0,
    );
    icon(
        d,
        Icon::Search,
        sr.x + 11.0,
        sr.y + 9.0,
        if focus { theme::INK } else { theme::FAINT },
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
        Rect::new(sr.x + 33.0, sr.y, sr.w - 41.0, sr.h),
        12.0,
        if state.query.is_empty() && !focus {
            theme::FAINT
        } else {
            theme::INK
        },
        state.search_focused || !state.query.is_empty(),
        false,
    );
    d.hit(sr, HitKind::Search);

    y += 46.0;
    let cols = columns(state.density, inner.w);
    let header = Rect::new(inner.x, y, inner.w, 18.0);
    draw_header(d, &cols, header, state.sort);
    d.hairline(Rect::new(inner.x, y + 24.0, inner.w, 1.0));
    y += 29.0;
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
        28.0
    } else {
        34.0
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
                    list.x + 4.0,
                    ry + row_h * 0.5 - 7.0,
                    theme::MUTED,
                );
                d.textw(
                    title,
                    Rect::new(list.x + 26.0, ry, 120.0, row_h),
                    11.5,
                    theme::DIM,
                    false,
                    false,
                    600,
                );
                let tw = text_width(title, 11.5, false);
                d.text(
                    &format!("{count}"),
                    Rect::new(list.x + 32.0 + tw, ry, 60.0, row_h),
                    10.0,
                    theme::FAINT,
                    true,
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

fn draw_header(d: &mut DrawList, cols: &[ColSpec], row: Rect, sort: Sort) {
    for c in cols {
        let r = Rect::new(row.x + c.x, row.y, c.w - 8.0, row.h);
        let active = c.col == sort.col;
        let color = if active { theme::DIM } else { theme::FAINT };
        d.textw(col_title(c.col), r, 11.0, color, false, c.right, 500);
        if active {
            let tw = text_width(col_title(c.col), 11.0, false);
            let cx = if c.right {
                r.right() - tw - 11.0
            } else {
                r.x + tw + 5.0
            };
            let cy = row.y + row.h * 0.5 - 2.0;
            if sort.desc {
                d.line(&[[cx, cy], [cx + 3.5, cy + 4.0], [cx + 7.0, cy]], 1.2, theme::DIM);
            } else {
                d.line(&[[cx, cy + 4.0], [cx + 3.5, cy], [cx + 7.0, cy + 4.0]], 1.2, theme::DIM);
            }
        }
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
        let color = match c.col {
            Col::Name => theme::INK,
            Col::Cpu => heat(p.cpu),
            _ => theme::DIM,
        };
        d.text(
            &text,
            r,
            12.5,
            color,
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
        Rect::new(viewport.right() - 4.0, y, 2.0, thumb_h),
        1.0,
        [238, 240, 246, 46],
        [0, 0, 0, 0],
        0.0,
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
    d.textw(
        "Resources",
        Rect::new(sub.x + 20.0, sub.y + 16.0, sub.w - 40.0, 16.0),
        11.0,
        theme::FAINT,
        false,
        false,
        500,
    );
    let items = [
        (Icon::Chip, "CPU", Section::Cpu, Some(percent(snap.cpu_total))),
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
        let r = Rect::new(sub.x + 8.0, y, sub.w - 16.0, 36.0);
        let on = state.section == section;
        let hot = r.contains(mouse[0], mouse[1]);
        if on {
            d.slab(r, 9.0, theme::SELECTED, [0, 0, 0, 0], 0.0);
        } else if hot {
            d.slab(r, 9.0, theme::HOVER, [0, 0, 0, 0], 0.0);
        }
        let color = if on {
            theme::INK
        } else if hot {
            theme::DIM
        } else {
            theme::MUTED
        };
        icon(d, ic, r.x + 12.0, r.y + 11.0, color);
        d.textw(
            label,
            Rect::new(r.x + 37.0, r.y, r.w * 0.5, r.h),
            13.0,
            color,
            false,
            false,
            if on { 500 } else { 400 },
        );
        if let Some(extra) = extra {
            d.text(
                &extra,
                Rect::new(r.x + r.w * 0.55, r.y, r.w * 0.45 - 12.0, r.h),
                10.5,
                if on { theme::DIM } else { theme::FAINT },
                true,
                true,
            );
        }
        d.hit(r, HitKind::Section(section));
        y += 40.0;
    }

    let view = detail.inset(28.0);
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

fn page_title(d: &mut DrawList, title: &str, sub: &str, view: Rect, y: f32) -> f32 {
    d.textw(
        title,
        Rect::new(view.x, y, view.w, 26.0),
        20.0,
        theme::INK,
        false,
        false,
        500,
    );
    d.text(
        sub,
        Rect::new(view.x, y + 27.0, view.w, 16.0),
        12.0,
        theme::MUTED,
        false,
        false,
    );
    y + 52.0
}

fn cpu_page(d: &mut DrawList, snap: &Snap, view: Rect, mut y: f32) -> f32 {
    y = page_title(d, "Processor", &snap.cpu_model, view, y);
    d.text(
        &percent(snap.cpu_total),
        Rect::new(view.x, y, 260.0, 56.0),
        48.0,
        theme::INK,
        true,
        false,
    );
    d.text(
        "total utilization",
        Rect::new(view.x, y + 58.0, 200.0, 14.0),
        10.5,
        theme::FAINT,
        false,
        false,
    );
    y += 84.0;

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
        (format!("{:.2}", snap.load[0]), "Load avg"),
    ];
    let slot = view.w / 5.0;
    for (i, (v, k)) in stats.iter().enumerate() {
        stat(d, view.x + i as f32 * slot, y, slot - 16.0, v, k);
    }
    y += 56.0;

    d.graph(
        Rect::new(view.x, y, view.w, 140.0),
        &[(&snap.cpu_hist, theme::TRACE)],
        100.0,
        true,
    );
    y += 162.0;

    let n = snap.cpu_per.len();
    if n == 0 {
        return y;
    }
    let peak = snap.cpu_per.iter().copied().fold(0.0_f32, f32::max);
    d.textw(
        "Cores",
        Rect::new(view.x, y, 100.0, 18.0),
        13.0,
        theme::DIM,
        false,
        false,
        500,
    );
    d.text(
        &format!("{n} logical · peak {}", cpu_pct(peak)),
        Rect::new(view.x + 100.0, y + 1.0, view.w - 100.0, 16.0),
        10.5,
        theme::FAINT,
        true,
        false,
    );
    y += 26.0;
    equalizer(d, Rect::new(view.x, y, view.w, 92.0), &snap.cpu_per);
    y + 108.0
}

/// Per-core usage as an equalizer: slim round-topped bars on a baseline.
fn equalizer(d: &mut DrawList, r: Rect, values: &[f32]) {
    let n = values.len();
    if n == 0 {
        return;
    }
    let slot = r.w / n as f32;
    let bw = (slot * 0.5).clamp(2.0, 12.0);
    for (i, v) in values.iter().enumerate() {
        let h = (v / 100.0).clamp(0.0, 1.0) * r.h;
        let h = h.max(2.0);
        let x = r.x + i as f32 * slot + (slot - bw) * 0.5;
        let color = if *v >= 90.0 {
            theme::HOT
        } else if *v >= 70.0 {
            theme::WARN
        } else {
            [238, 240, 246, 170]
        };
        d.slab(
            Rect::new(x, r.bottom() - h, bw, h),
            bw * 0.5,
            color,
            [0, 0, 0, 0],
            0.0,
        );
    }
    d.slab(
        Rect::new(r.x, r.bottom(), r.w, 1.0),
        0.0,
        theme::DIVIDER,
        [0, 0, 0, 0],
        0.0,
    );
}

fn memory_page(d: &mut DrawList, snap: &Snap, view: Rect, mut y: f32) -> f32 {
    y = page_title(d, "Memory", "Physical RAM", view, y);
    let used = bytes(snap.mem_used);
    d.text(
        &used,
        Rect::new(view.x, y, view.w, 56.0),
        48.0,
        theme::INK,
        true,
        false,
    );
    let uw = text_width(&used, 48.0, true);
    d.text(
        &format!("/ {}", bytes(snap.mem_total)),
        Rect::new(view.x + uw + 10.0, y + 26.0, view.w - uw - 10.0, 26.0),
        16.0,
        theme::MUTED,
        true,
        false,
    );
    y += 76.0;
    d.graph(
        Rect::new(view.x, y, view.w, 140.0),
        &[(&snap.mem_hist, theme::TRACE)],
        1.0,
        true,
    );
    y += 158.0;

    let stats = [
        (bytes(snap.mem_available), "Available"),
        (bytes(snap.mem_cached), "Cached"),
        (bytes(snap.mem_buffers), "Buffers"),
    ];
    let slot = view.w / 3.0;
    for (i, (v, k)) in stats.iter().enumerate() {
        stat(d, view.x + i as f32 * slot, y, slot - 16.0, v, k);
    }
    y += 58.0;

    if snap.swap_total > 0 {
        d.textw(
            "Swap",
            Rect::new(view.x, y, 100.0, 18.0),
            13.0,
            theme::DIM,
            false,
            false,
            500,
        );
        d.text(
            &format!("{} / {}", bytes(snap.swap_used), bytes(snap.swap_total)),
            Rect::new(view.x + 100.0, y + 1.0, view.w - 100.0, 16.0),
            10.5,
            theme::FAINT,
            true,
            false,
        );
        y += 24.0;
        d.graph(
            Rect::new(view.x, y, view.w, 80.0),
            &[(&snap.swap_hist, theme::TRACE_DIM)],
            1.0,
            false,
        );
        y += 92.0;
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
        y = page_title(d, "Graphics", &g.name, view, y);
        let util = g.util.map(percent).unwrap_or_else(|| "—".into());
        d.text(
            &util,
            Rect::new(view.x, y, 220.0, 56.0),
            48.0,
            theme::INK,
            true,
            false,
        );
        d.text(
            "utilization",
            Rect::new(view.x, y + 58.0, 200.0, 14.0),
            10.5,
            theme::FAINT,
            false,
            false,
        );
        y += 82.0;
        let gh = 110.0;
        let hist = g.util_hist.as_slice();
        d.graph(
            Rect::new(view.x, y, view.w, gh),
            &[(hist, theme::TRACE)],
            100.0,
            true,
        );
        y += gh + 20.0;
        if g.mem_total > 0 {
            let frac = (g.mem_used as f32 / g.mem_total as f32).clamp(0.0, 1.0);
            d.textw(
                "VRAM",
                Rect::new(view.x, y, 80.0, 16.0),
                11.0,
                theme::DIM,
                false,
                false,
                500,
            );
            d.text(
                &format!("{} / {}", bytes(g.mem_used), bytes(g.mem_total)),
                Rect::new(view.x + 80.0, y, view.w - 80.0, 16.0),
                11.0,
                theme::MUTED,
                true,
                true,
            );
            y += 20.0;
            d.slab(
                Rect::new(view.x, y, view.w, 3.0),
                1.5,
                [238, 240, 246, 14],
                [0, 0, 0, 0],
                0.0,
            );
            d.slab(
                Rect::new(view.x, y, (view.w * frac).max(3.0), 3.0),
                1.5,
                theme::ACCENT_DIM,
                [0, 0, 0, 0],
                0.0,
            );
            y += 22.0;
        }
        // Telemetry as one quiet readout line.
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
            &bits.join("   ·   "),
            Rect::new(view.x, y, view.w, 16.0),
            10.5,
            theme::MUTED,
            true,
            false,
        );
        y += 40.0;
    }
    y
}

fn io_page(d: &mut DrawList, view: Rect, mut y: f32, disk: bool, snap: &Snap) -> f32 {
    let (names_empty, a_legend, b_legend) = if disk {
        ("No disks", "read", "write")
    } else {
        ("No interfaces", "rx", "tx")
    };
    let devs: Vec<(&str, f64, f64, &[f32], &[f32])> = if disk {
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
        d.text(
            names_empty,
            Rect::new(view.x, y, view.w, 24.0),
            14.0,
            theme::MUTED,
            false,
            false,
        );
        return y + 28.0;
    }
    for (name, a_bps, b_bps, a_hist, b_hist) in devs {
        d.textw(
            name,
            Rect::new(view.x, y, view.w * 0.4, 20.0),
            13.0,
            theme::INK,
            true,
            false,
            500,
        );
        let (la, lb) = if disk { ("R", "W") } else { ("RX", "TX") };
        d.text(
            &format!("{la} {}    {lb} {}", rate(a_bps), rate(b_bps)),
            Rect::new(view.x + view.w * 0.35, y, view.w * 0.65, 20.0),
            11.0,
            theme::MUTED,
            true,
            true,
        );
        y += 26.0;
        let max = nice_pair(a_hist, b_hist);
        let gr = Rect::new(view.x, y, view.w, 100.0);
        d.graph(
            gr,
            &[(a_hist, theme::TRACE), (b_hist, theme::TRACE_DIM)],
            max,
            false,
        );
        // Legend: short line swatches, top right.
        let lw = text_width(a_legend, 10.0, true) + text_width(b_legend, 10.0, true) + 52.0;
        let lx = gr.right() - lw - 2.0;
        let ly = gr.y + 4.0;
        d.line(&[[lx, ly + 5.0], [lx + 12.0, ly + 5.0]], 1.5, theme::TRACE);
        d.text(
            a_legend,
            Rect::new(lx + 17.0, ly, 34.0, 12.0),
            10.0,
            theme::MUTED,
            true,
            false,
        );
        let lx2 = lx + 17.0 + text_width(a_legend, 10.0, true) + 14.0;
        d.line(&[[lx2, ly + 5.0], [lx2 + 12.0, ly + 5.0]], 1.5, theme::TRACE_DIM);
        d.text(
            b_legend,
            Rect::new(lx2 + 17.0, ly, 34.0, 12.0),
            10.0,
            theme::MUTED,
            true,
            false,
        );
        y += 118.0;
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
    let inner = main.inset(28.0);
    d.textw(
        "Startup apps",
        Rect::new(inner.x, inner.y + 4.0, inner.w, 28.0),
        20.0,
        theme::INK,
        false,
        false,
        500,
    );
    d.text(
        "User autostart entries. Turning one off writes Hidden=true and keeps a .bak.",
        Rect::new(inner.x, inner.y + 34.0, inner.w, 16.0),
        12.0,
        theme::MUTED,
        false,
        false,
    );
    let list = Rect::new(
        inner.x,
        inner.y + 66.0,
        inner.w,
        (inner.h - 66.0).max(20.0),
    );
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
    let row_h = 54.0;
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
                Rect::new(list.x - 8.0, ry, list.w + 16.0, row_h),
                8.0,
                theme::HOVER,
                [0, 0, 0, 0],
                0.0,
            );
        }
        d.textw(
            &entry.name,
            Rect::new(list.x, ry + 8.0, list.w - 110.0, 20.0),
            13.5,
            theme::INK,
            false,
            false,
            500,
        );
        d.text(
            &entry.exec,
            Rect::new(list.x, ry + 29.0, list.w - 110.0, 16.0),
            10.5,
            theme::MUTED,
            true,
            false,
        );
        d.hairline(Rect::new(list.x, ry + row_h - 1.0, list.w, 1.0));
        switch(
            d,
            Rect::new(list.right() - 40.0, ry + 17.0, 34.0, 20.0),
            entry.enabled,
        );
        d.hit(Rect::new(list.x, ry, list.w, row_h), HitKind::Startup(i));
    }
    scrollbar(d, list, content_h, state.startup_scroll);
}

fn switch(d: &mut DrawList, r: Rect, on: bool) {
    d.slab(
        r,
        r.h * 0.5,
        if on {
            theme::ACCENT
        } else {
            [238, 240, 246, 14]
        },
        [0, 0, 0, 0],
        0.0,
    );
    let kx = if on { r.right() - 16.5 } else { r.x + 2.5 };
    d.slab(
        Rect::new(kx, r.y + 2.5, 15.0, 15.0),
        7.5,
        if on {
            theme::ON_ACCENT
        } else {
            [140, 142, 150, 255]
        },
        [0, 0, 0, 0],
        0.0,
    );
}
