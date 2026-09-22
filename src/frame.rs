use std::time::{Duration, Instant};

use crate::anim::{self, cascade, key, lerp, mix_rgba, Anim, Key};
use crate::format::{
    bytes, cpu_pct, disk_cell, duration, fit_t, freq_ghz, percent, rate, text_width_t,
};
use crate::interact::selection_label;
use crate::model::{
    io_scale, theme, AppState, Col, ContextMenu, Density, Drag, MenuAction, Page, Proc, ProcView,
    ScrollBar, Section, Snap, Sort, StartupEntry,
};
use crate::settings::{Choice, Curve, Opt, ProcCpu, Settings, Speed, Units, OPTIONAL_COLS};

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
    /// Round capsule ends. Off for graph traces, which run edge to edge.
    pub round: bool,
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
    /// Optional scissor in design pixels (e.g. process list viewport).
    pub clip: Option<Rect>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HitKind {
    DragWindow,
    Close,
    Minimize,
    Page(Page),
    Section(Section),
    View(ProcView),
    Density,
    Sort(Col),
    Search,
    EndTask,
    Group(bool),
    Proc {
        pid: i32,
    },
    /// Empty process-list chrome: clears the selection (unfreezes pinned rows).
    Deselect,
    Scroll(crate::model::ScrollBar),
    Startup(usize),
    DragNav,
    DragSub,
    MenuItem(crate::model::MenuAction),
    /// Menu body outside any item: swallows the click.
    MenuPanel,
    /// Settings control: a segment index, or 0 / 1 for a switch.
    Setting(Opt, u8),
    /// Interface scale stepper, -1 or +1.
    Zoom(i8),
    ResetSettings,
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

/// One paint layer. The renderer draws each layer's shapes, strokes, and
/// text before the next layer, so overlays fully cover what is under them.
#[derive(Default)]
pub struct Layer {
    pub slabs: Vec<Slab>,
    pub strokes: Vec<Stroke>,
    pub labels: Vec<Label>,
}

const BASE: usize = 0;
const OVERLAY: usize = 1;

pub struct DrawList {
    pub layers: [Layer; 2],
    pub hits: Vec<Hit>,
    pub list_rect: Option<Rect>,
    pub detail_rect: Option<Rect>,
    pub startup_rect: Option<Rect>,
    pub settings_rect: Option<Rect>,
    clip: Option<Rect>,
    layer: usize,
    /// Alpha multiplier for everything drawn: entrances, exits, overlays.
    fade: f32,
    /// Entrance progress of the current page, 0 to 1.
    enter: f32,
    prefs: Settings,
    anim: Anim,
}

const NONE: theme::Rgba = [0, 0, 0, 0];

impl DrawList {
    fn new(prefs: Settings, anim: Anim) -> Self {
        Self {
            layers: [Layer::default(), Layer::default()],
            hits: Vec::new(),
            list_rect: None,
            detail_rect: None,
            startup_rect: None,
            settings_rect: None,
            clip: None,
            layer: BASE,
            fade: 1.0,
            enter: 1.0,
            prefs,
            anim,
        }
    }

    /// Run `f` with everything it draws faded by `a` on top of the current fade.
    fn faded<R>(&mut self, a: f32, f: impl FnOnce(&mut Self) -> R) -> R {
        let saved = self.fade;
        self.fade *= a.clamp(0.0, 1.0);
        let out = f(self);
        self.fade = saved;
        out
    }

    fn ink(&self, c: theme::Rgba) -> theme::Rgba {
        [c[0], c[1], c[2], (c[3] as f32 * self.fade).round() as u8]
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
        let Some(r) = self.clip_rect(r) else {
            return;
        };
        if r.w < 1.0 || r.h < 1.0 {
            return;
        }
        let (fill, border) = (self.ink(fill), self.ink(border));
        if fill[3] == 0 && border[3] == 0 {
            return;
        }
        self.layers[self.layer].slabs.push(Slab {
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

    /// Intersect with the active clip, if any. `None` means fully outside.
    fn clip_rect(&self, r: Rect) -> Option<Rect> {
        let Some(c) = self.clip else {
            return Some(r);
        };
        let x0 = r.x.max(c.x);
        let y0 = r.y.max(c.y);
        let x1 = r.right().min(c.right());
        let y1 = r.bottom().min(c.bottom());
        if x1 <= x0 || y1 <= y0 {
            return None;
        }
        Some(Rect::new(x0, y0, x1 - x0, y1 - y0))
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

    /// Glyph stroke: joined polyline with round ends.
    fn line(&mut self, pts: &[[f32; 2]], width: f32, color: theme::Rgba) {
        if let Some(c) = self.clip {
            let mut min_y = f32::INFINITY;
            let mut max_y = f32::NEG_INFINITY;
            for p in pts {
                min_y = min_y.min(p[1]);
                max_y = max_y.max(p[1]);
            }
            if max_y < c.y || min_y > c.bottom() {
                return;
            }
        }
        let color = self.ink(color);
        self.layers[self.layer].strokes.push(Stroke {
            pts: pts.to_vec(),
            width,
            color,
            baseline: None,
            round: true,
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
        let color = self.ink(color);
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
        // Center on the em box. A tall 1.3 line box left unused descender
        // room that lifted labels; glyphon still paints ink a hair low in the
        // em, so nudge up to land on the geometric mid of the pill.
        let em = t.size;
        let lh = em * 1.2;
        let x = match align {
            Align::Left => r.x,
            Align::Right => r.right() - tw,
            Align::Center => r.x + (r.w - tw) * 0.5,
        };
        let y = r.y + (r.h - em) * 0.5 - em * 0.08;
        self.layers[self.layer].labels.push(Label {
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
            clip: self.clip,
        });
    }

    /// Telemetry graph: faint quarter grid, hairline baseline, thin traces.
    ///
    /// X spans a fixed window of samples (from the settings) so the scale does
    /// not stretch as the ring fills. `head` places the right edge relative to
    /// the newest sample (<= 0) and moves continuously, so the trace scrolls at
    /// a constant speed and new data is drawn in rather than popped in. Smooth
    /// curves are uniform cubic B-splines: C2-smooth, and never outside the
    /// samples around them.
    fn graph(&mut self, r: Rect, series: &[(&[f32], theme::Rgba)], max: f32, head: f32) {
        if r.w < 4.0 || r.h < 4.0 || !self.visible(r) {
            return;
        }
        let max = max.max(0.001);
        let head = head.min(0.0);
        let curve = self.prefs.curve;
        // Stroke width + miter + AA fringe need room above a 100% sample.
        let top_pad = 6.0;
        let usable = (r.h - top_pad).max(1.0);
        if r.h >= 80.0 && self.prefs.grid {
            for k in 1..4 {
                let gy = r.y + r.h * (k as f32) / 4.0;
                self.fill(Rect::new(r.x, gy, r.w, 1.0), 0.0, theme::GRID);
            }
        }
        self.hairline(Rect::new(r.x, r.bottom(), r.w, 1.0));
        let span = self.prefs.window() as f32;
        let slot_w = r.w / span;
        // Vertices sit at fixed fractions of each sample slot, so the polyline
        // is rigid in data space and only translates as it scrolls. Resampling
        // at fixed screen x instead makes sharp peaks shimmer frame to frame.
        let sub = ((slot_w / 1.5).ceil() as usize).clamp(4, 24);
        let y_of = |v: f32| r.bottom() - (v / max).clamp(0.0, 1.0) * usable;
        for (values, color) in series.iter() {
            if values.len() < 2 {
                continue;
            }
            let n = values.len();
            let right = (n - 1) as f32 + head;
            let x_of = |idx: f32| r.right() - (right - idx) * slot_w;
            let last = (n - 1) * sub;
            let lo = (((right - span) * sub as f32).floor() as i64 - 1).clamp(0, last as i64);
            let hi = ((right * sub as f32).ceil() as i64 + 1).clamp(0, last as i64);
            // History starts at launch: before the first sample the plot stays
            // empty, and the trace grows in from the right edge.
            let vals: Vec<(f32, f32)> = (lo..=hi)
                .map(|j| {
                    let idx = j as f32 / sub as f32;
                    (x_of(idx), curve_at(values, idx, curve))
                })
                .collect();
            // Never clip a trace flat against the top while an eased scale
            // is still catching up with it.
            let peak = vals.iter().map(|v| v.1).fold(0.0_f32, f32::max);
            let top = max.max(peak);
            let raw: Vec<[f32; 2]> = vals
                .iter()
                .map(|&(x, v)| [x, y_of(v * max / top)])
                .collect();
            let pts = clip_polyline_x(&raw, r.x, r.right());
            if pts.len() < 2 {
                continue;
            }
            let color = self.ink(*color);
            self.layers[self.layer].strokes.push(Stroke {
                pts,
                width: 1.25,
                color,
                baseline: self.prefs.fill.then_some(r.bottom()),
                round: false,
            });
        }
    }
}

fn curve_at(values: &[f32], idx: f32, curve: Curve) -> f32 {
    match curve {
        Curve::Smooth => sample_hist(values, idx),
        Curve::Linear => sample_linear(values, idx),
    }
}

/// Straight segments through the samples: exact peaks, visible corners.
fn sample_linear(values: &[f32], idx: f32) -> f32 {
    let n = values.len();
    if n == 0 {
        return 0.0;
    }
    let idx = idx.clamp(0.0, (n - 1) as f32);
    let i = (idx.floor() as usize).min(n.saturating_sub(2));
    let t = idx - i as f32;
    let b = values[(i + 1).min(n - 1)];
    values[i] + (b - values[i]) * t
}

/// Uniform cubic B-spline sample. C2-smooth, so climbs bend over a whole
/// sample instead of kinking, and always inside the range of the four samples
/// around it, so it never overshoots. It does not pass through the samples:
/// an isolated one-sample spike peaks at two thirds of its height. Segment
/// `i` reads samples `i - 1 ..= i + 2`; the ends repeat their edge sample.
fn sample_hist(values: &[f32], idx: f32) -> f32 {
    let n = values.len();
    if n == 0 {
        return 0.0;
    }
    if n == 1 {
        return values[0];
    }
    let idx = idx.clamp(0.0, (n - 1) as f32);
    let i = (idx.floor() as usize).min(n - 2);
    let t = idx - i as f32;
    let at = |k: isize| values[k.clamp(0, n as isize - 1) as usize];
    let i = i as isize;
    let (p0, p1, p2, p3) = (at(i - 1), at(i), at(i + 1), at(i + 2));
    let t2 = t * t;
    let t3 = t2 * t;
    let u = 1.0 - t;
    (u * u * u * p0
        + (3.0 * t3 - 6.0 * t2 + 4.0) * p1
        + (-3.0 * t3 + 3.0 * t2 + 3.0 * t + 1.0) * p2
        + t3 * p3)
        / 6.0
}

/// Keep a scrolling series inside the chart: drop off-screen points and insert
/// edge intersections so the ribbon does not bleed past the plot.
fn clip_polyline_x(pts: &[[f32; 2]], x0: f32, x1: f32) -> Vec<[f32; 2]> {
    let mut out = Vec::with_capacity(pts.len() + 2);
    let push = |out: &mut Vec<[f32; 2]>, p: [f32; 2]| {
        if out
            .last()
            .is_none_or(|q: &[f32; 2]| (q[0] - p[0]).abs() > 1e-3 || (q[1] - p[1]).abs() > 1e-3)
        {
            out.push(p);
        }
    };
    let intersect = |a: [f32; 2], b: [f32; 2], edge: f32| -> Option<[f32; 2]> {
        let dx = b[0] - a[0];
        if dx.abs() < 1e-6 {
            return None;
        }
        let t = (edge - a[0]) / dx;
        if !(0.0..=1.0).contains(&t) {
            return None;
        }
        Some([edge, a[1] + (b[1] - a[1]) * t])
    };
    let inside = |x: f32| x >= x0 && x <= x1;

    for w in pts.windows(2) {
        let (a, b) = (w[0], w[1]);
        match (inside(a[0]), inside(b[0])) {
            (true, true) => {
                push(&mut out, a);
            }
            (true, false) => {
                push(&mut out, a);
                let edge = if b[0] < x0 { x0 } else { x1 };
                if let Some(p) = intersect(a, b, edge) {
                    push(&mut out, p);
                }
            }
            (false, true) => {
                let edge = if a[0] < x0 { x0 } else { x1 };
                if let Some(p) = intersect(a, b, edge) {
                    push(&mut out, p);
                }
            }
            (false, false) => {
                // Segment may still cross the visible band (both ends outside).
                if (a[0] < x0 && b[0] > x1) || (a[0] > x1 && b[0] < x0) {
                    let (left, right) = if a[0] < b[0] {
                        (intersect(a, b, x0), intersect(a, b, x1))
                    } else {
                        (intersect(a, b, x1), intersect(a, b, x0))
                    };
                    if let (Some(p0), Some(p1)) = (left, right) {
                        push(&mut out, p0);
                        push(&mut out, p1);
                    }
                }
            }
        }
    }
    if let Some(&last) = pts.last() {
        if inside(last[0]) {
            push(&mut out, last);
        }
    }
    out
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
    Power,
    /// Settings: a six-tooth cog with a hub.
    Cog,
    Chip,
    Mem,
    Gpu,
    Disk,
    Net,
    Search,
    Min,
    Close,
    /// Row density: three open rules.
    Loose,
    /// Row density: four tight rules.
    Dense,
}

/// Hairline glyph weight: rules, chevrons, pins.
const ICON_W: f32 = 1.35;
/// Nav glyph weight. Matches the list dots so the three pages read as one set.
const GLYPH_W: f32 = 2.4;
/// Slab border that lands at `GLYPH_W` on screen. The border band straddles
/// the edge, so its ink is about twice the requested width plus the fringe.
const RING_W: f32 = 1.0;

/// Outlined rounded box at glyph weight.
fn ring(d: &mut DrawList, r: Rect, radius: f32, c: theme::Rgba) {
    d.slab(r, radius, NONE, c, RING_W);
}

/// Soft disc. Slabs already antialias.
fn cap(d: &mut DrawList, x: f32, y: f32, diameter: f32, c: theme::Rgba) {
    d.fill(
        Rect::new(x - diameter * 0.5, y - diameter * 0.5, diameter, diameter),
        diameter * 0.5,
        c,
    );
}

/// Axis-aligned bar with round ends. Prefer this over a stroke for H/V ink.
fn bar(d: &mut DrawList, x0: f32, y0: f32, x1: f32, y1: f32, thickness: f32, c: theme::Rgba) {
    let t = thickness;
    if (y0 - y1).abs() < 0.01 {
        let (a, b) = if x0 <= x1 { (x0, x1) } else { (x1, x0) };
        d.fill(Rect::new(a, y0 - t * 0.5, (b - a).max(t), t), t * 0.5, c);
    } else if (x0 - x1).abs() < 0.01 {
        let (a, b) = if y0 <= y1 { (y0, y1) } else { (y1, y0) };
        d.fill(Rect::new(x0 - t * 0.5, a, t, (b - a).max(t)), t * 0.5, c);
    } else {
        d.line(&[[x0, y0], [x1, y1]], t, c);
    }
}

/// Vertical arrow: a chevron head and a shaft that starts exactly where it
/// leaves the head's inner edge, so the two never stack ink.
fn arrow(d: &mut DrawList, cx: f32, tip: f32, tail: f32, half: f32, t: f32, c: theme::Rgba) {
    let up = tail > tip;
    let dir = if up { 1.0 } else { -1.0 };
    d.line(
        &[
            [cx - half, tip + half * dir],
            [cx, tip],
            [cx + half, tip + half * dir],
        ],
        t,
        c,
    );
    // Inner crotch sits t/2 * sqrt(2) behind the tip; the shaft's corners meet
    // the inner edges another t/2 further on. Overlap by a hair, not a cap.
    let start = tip + dir * (t * 0.5 * std::f32::consts::SQRT_2 + t * 0.5 - 0.4);
    bar(d, cx, start, cx, tail, t, c);
}

/// Draw a 14x14 line icon at (x, y).
fn icon(d: &mut DrawList, kind: Icon, x: f32, y: f32, c: theme::Rgba) {
    let w = ICON_W;
    match kind {
        // Pages. Same grid as the resources: ink from 0.2 to 13.2, mass in
        // the body, one hairline detail.

        // Three records: a marker and a rule each. Rules carry weight so the
        // silhouette is the list, not the dots; uneven lengths keep it from
        // collapsing into a menu mark.
        Icon::List => {
            let rows = [(2.6_f32, 12.4_f32), (7.0, 9.8), (11.4, 11.4)];
            for (yy, x1) in rows {
                cap(d, x + 2.6, y + yy, GLYPH_W, c);
                bar(d, x + 5.6, y + yy, x + x1, y + yy, 2.0, c);
            }
        }
        // A trend over its axis: one dip, then the climb. One mitered ribbon
        // with round ends; the hairline baseline makes it a chart.
        Icon::Pulse => {
            d.line(
                &[
                    [x + 1.6, y + 9.4],
                    [x + 5.2, y + 4.8],
                    [x + 8.2, y + 7.2],
                    [x + 12.4, y + 1.4],
                ],
                GLYPH_W,
                c,
            );
            bar(d, x + 0.8, y + 12.5, x + 13.2, y + 12.5, w, c);
        }
        // Power: an open ring with the switch bar standing in its gap.
        Icon::Power => {
            let (cx, cy, r) = (x + 7.0, y + 7.2, 4.8);
            let (from, to) = (50.0_f32, 310.0_f32);
            let n = 20;
            let arc: Vec<[f32; 2]> = (0..=n)
                .map(|i| {
                    let a = (from + (to - from) * i as f32 / n as f32).to_radians();
                    [cx + r * a.sin(), cy - r * a.cos()]
                })
                .collect();
            d.line(&arc, GLYPH_W, c);
            bar(d, cx, y + 0.6, cx, y + 6.4, GLYPH_W, c);
        }
        // Cog: six teeth as one closed outline, so no ink stacks at the
        // joins, around a hub dot.
        Icon::Cog => {
            let (cx, cy) = (x + 7.0, y + 7.0);
            // Radius follows a softened square wave, so teeth have flat tops
            // and rounded shoulders instead of corners.
            let (teeth, r_mid, amp, soft) = (6.0_f32, 5.0_f32, 0.85_f32, 2.2_f32);
            let n = 144;
            let mut pts: Vec<[f32; 2]> = (0..n)
                .map(|i| {
                    let a = i as f32 / n as f32 * std::f32::consts::TAU;
                    let wave = (soft * (teeth * a).cos()).tanh() / soft.tanh();
                    let r = r_mid + amp * wave;
                    [cx + r * a.cos(), cy + r * a.sin()]
                })
                .collect();
            pts.push(pts[0]);
            d.line(&pts, w, c);
            let hub: Vec<[f32; 2]> = (0..=40)
                .map(|i| {
                    let a = i as f32 / 40.0 * std::f32::consts::TAU;
                    [cx + 1.9 * a.cos(), cy + 1.9 * a.sin()]
                })
                .collect();
            d.line(&hub, w, c);
        }
        // Resources. Five silhouettes that cannot be confused for each other:
        // a square with pins, a bar with legs, a card with a fan, a platter,
        // and a pair of arrows. Bodies at glyph weight, pins at hairline,
        // like the list's dots and rules.

        // Package: square die, two pins a side. Pins stop at the ink edge.
        Icon::Chip => {
            ring(d, Rect::new(x + 3.5, y + 3.5, 7.0, 7.0), 1.9, c);
            for k in [5.3_f32, 8.7] {
                bar(d, x + k, y + 0.9, x + k, y + 2.3, w, c);
                bar(d, x + k, y + 11.7, x + k, y + 13.1, w, c);
                bar(d, x + 0.9, y + k, x + 2.3, y + k, w, c);
                bar(d, x + 11.7, y + k, x + 13.1, y + k, w, c);
            }
        }
        // Module: long body over three contacts.
        Icon::Mem => {
            ring(d, Rect::new(x + 1.4, y + 2.2, 11.2, 6.0), 1.6, c);
            for k in [4.0_f32, 7.0, 10.0] {
                bar(d, x + k, y + 9.9, x + k, y + 12.7, w, c);
            }
        }
        // Card: wide body, open fan toward the back, bracket along the front.
        Icon::Gpu => {
            ring(d, Rect::new(x + 1.0, y + 1.6, 12.0, 8.4), 1.9, c);
            cap(d, x + 9.1, y + 5.8, 2.6, c);
            bar(d, x + 1.9, y + 12.2, x + 8.2, y + 12.2, w, c);
        }
        // Platter with a spindle.
        Icon::Disk => {
            ring(d, Rect::new(x + 2.0, y + 2.0, 10.0, 10.0), 5.0, c);
            cap(d, x + 7.0, y + 7.0, GLYPH_W, c);
        }
        // Down on the left, up on the right: receive and transmit. The heads
        // sit at opposite ends so the pair fits at glyph weight.
        Icon::Net => {
            let t = 2.0;
            arrow(d, x + 3.7, y + 12.0, y + 2.4, 2.7, t, c);
            arrow(d, x + 10.3, y + 2.0, y + 11.6, 2.7, t, c);
        }
        // Ring plus handle. Thin stroke so the lens hole reads open, not a bead.
        Icon::Search => {
            d.slab(Rect::new(x + 1.85, y + 1.85, 8.3, 8.3), 4.15, NONE, c, 1.0);
            d.line(&[[x + 9.85, y + 9.85], [x + 12.6, y + 12.6]], w, c);
        }
        Icon::Min => d.line(&[[x + 3.0, y + 7.0], [x + 11.0, y + 7.0]], w, c),
        Icon::Close => {
            d.line(&[[x + 3.5, y + 3.5], [x + 10.5, y + 10.5]], w, c);
            d.line(&[[x + 10.5, y + 3.5], [x + 3.5, y + 10.5]], w, c);
        }
        Icon::Loose => {
            for yy in [2.75_f32, 7.0, 11.25] {
                bar(d, x + 2.0, y + yy, x + 12.0, y + yy, w, c);
            }
        }
        Icon::Dense => {
            for yy in [2.75_f32, 5.5, 8.25, 11.0] {
                bar(d, x + 2.0, y + yy, x + 12.0, y + yy, w, c);
            }
        }
    }
}

/// Status chroma for a usage value; `base` when it is not hot or status color is off.
fn heat(d: &DrawList, v: f32, base: theme::Rgba) -> theme::Rgba {
    if !d.prefs.heat {
        base
    } else if v >= 90.0 {
        theme::HOT
    } else if v >= 70.0 {
        theme::WARN
    } else {
        base
    }
}

// --- Toolbar pills ----------------------------------------------------------
//
// One height, one outline weight, one horizontal rhythm. Text-only pills pad
// 16 each side; a leading glyph sits 12 in with an 8 gap to its label.

const PILL_H: f32 = 32.0;
const PILL_PAD: f32 = 16.0;
const PILL_GAP: f32 = 10.0;

fn pill_w(label: &str, glyph: bool) -> f32 {
    // Ceil so the fitted label never loses its last glyph to rounding.
    let text = measure(label, PILL).ceil() + 1.0;
    if glyph {
        12.0 + 14.0 + 8.0 + text + PILL_PAD
    } else {
        text + PILL_PAD * 2.0
    }
}

/// Ghost pill: transparent, 1px outline, optional glyph, label. The only button.
/// Hover, enabled and danger each cross-fade; `id` keys that state.
#[allow(clippy::too_many_arguments)]
fn ghost_pill(
    d: &mut DrawList,
    id: HitKind,
    r: Rect,
    glyph: Option<Icon>,
    label: &str,
    mouse: [f32; 2],
    enabled: bool,
    danger: bool,
) {
    let hot = enabled && r.contains(mouse[0], mouse[1]);
    let h = d.anim.hover(key("pill-hover", id), hot);
    let en = d.anim.toggle(key("pill-enabled", id), enabled);
    let dz = d.anim.toggle(key("pill-danger", id), danger);
    let mut line = mix_rgba(theme::GHOST_LINE, theme::ACCENT_LINE, h);
    let mut ink = mix_rgba(theme::INK_2, theme::INK, h);
    let mut glyph_ink = mix_rgba(theme::INK_3, theme::INK, h);
    line = mix_rgba(theme::HAIRLINE, line, en);
    ink = mix_rgba(theme::INK_4, ink, en);
    glyph_ink = mix_rgba(theme::INK_4, glyph_ink, en);
    line = mix_rgba(line, theme::DANGER_LINE, dz);
    ink = mix_rgba(ink, theme::DANGER_INK, dz);
    glyph_ink = mix_rgba(glyph_ink, theme::DANGER_INK, dz);
    d.slab(r, r.h * 0.5, mix_rgba(NONE, theme::HOVER, h), line, 1.0);
    match glyph {
        Some(ic) => {
            icon(d, ic, r.x + 12.0, r.y + (r.h - 14.0) * 0.5, glyph_ink);
            d.text(
                label,
                Rect::new(r.x + 34.0, r.y, r.w - 34.0 - PILL_PAD + 2.0, r.h),
                PILL,
                ink,
            );
        }
        None => d.text_c(label, r, PILL, ink),
    }
}

// Inset from the outer ring; equal side padding inside each cell so every
// label, short "Flat" or long "Grouped", sits on the same rhythm.
const SEG_INSET: f32 = 3.0;
const SEG_PAD_X: f32 = 16.0;

fn segmented_w(items: &[(&str, bool, HitKind)]) -> f32 {
    items
        .iter()
        .map(|(l, ..)| measure(l, PILL) + SEG_PAD_X * 2.0)
        .sum::<f32>()
        + SEG_INSET * 2.0
}

/// Segmented control: one ghost outline, the active segment is the white pill.
/// The pill glides between segments; each label inverts as the pill covers it.
fn segmented(
    d: &mut DrawList,
    x: f32,
    y: f32,
    items: &[(&str, bool, HitKind)],
    mouse: [f32; 2],
) -> Rect {
    let seg = Rect::new(x, y, segmented_w(items), PILL_H);
    d.outline(seg, PILL_H * 0.5, theme::GHOST_LINE);
    let Some(first) = items.first() else {
        return seg;
    };
    let id = key("segmented", first.2);
    let mut cx = seg.x + SEG_INSET;
    let cells: Vec<Rect> = items
        .iter()
        .map(|(label, ..)| {
            let w = measure(label, PILL) + SEG_PAD_X * 2.0;
            let r = Rect::new(cx, y + SEG_INSET, w, PILL_H - SEG_INSET * 2.0);
            cx += w;
            r
        })
        .collect();
    let active = items.iter().position(|(_, on, _)| *on);
    // Offsets from the control's left edge, so moving the control (a resize,
    // a scroll) never drags the pill behind it.
    let (lo, hi) = match active {
        Some(a) => (cells[a].x - seg.x, cells[a].right() - seg.x),
        None => (
            d.anim.peek(key("lo", id)).unwrap_or(0.0),
            d.anim.peek(key("hi", id)).unwrap_or(0.0),
        ),
    };
    let (lo, hi) = d.anim.glide(id, lo, hi);
    let shown = d
        .anim
        .toggle(key("segmented-on", first.2), active.is_some());
    let pill = Rect::new(seg.x + lo, y + SEG_INSET, hi - lo, PILL_H - SEG_INSET * 2.0);
    let hovers: Vec<f32> = items
        .iter()
        .zip(&cells)
        .map(|((_, on, kind), r)| {
            let hot = !*on && r.contains(mouse[0], mouse[1]);
            d.anim.hover(key("segment-hover", *kind), hot)
        })
        .collect();
    for (r, h) in cells.iter().zip(&hovers) {
        d.fill(*r, r.h * 0.5, mix_rgba(NONE, theme::HOVER, *h));
    }
    d.faded(shown, |d| d.fill(pill, pill.h * 0.5, theme::ACCENT));
    for (((label, _, kind), r), h) in items.iter().zip(&cells).zip(&hovers) {
        let overlap = (pill.right().min(r.right()) - pill.x.max(r.x)).max(0.0);
        let cover = (overlap / r.w.max(1.0)).clamp(0.0, 1.0) * shown;
        let rest = mix_rgba(theme::INK_2, theme::INK, *h);
        let t = if cover > 0.5 { PILL_ON } else { PILL };
        d.text_c(label, *r, t, mix_rgba(rest, theme::ON_ACCENT, cover));
        d.hit(*r, *kind);
    }
    seg
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
    let mut d = DrawList::new(state.settings, std::mem::take(&mut state.anim));
    d.anim
        .begin(state.settings.animations, (state.width, state.height));
    crate::format::set_decimal(state.settings.units == Units::Decimal);
    // An open menu is modal: nothing beneath it hovers.
    let pointer = mouse;
    let mouse = if state.menu.is_some() {
        [f32::NEG_INFINITY; 2]
    } else {
        mouse
    };
    state.scroll_bar = None;
    if state.page == Page::Performance {
        state.perf_smooth.tick(snap, &state.settings, state.paused);
    }
    if state.reset_armed.is_some_and(|t| t <= Instant::now()) {
        state.reset_armed = None;
    }
    let w = state.width.max(420.0);
    let h = state.height.max(320.0);

    // One window, one sheet of black glass.
    let root = Rect::new(0.0, 0.0, w, h);
    let mut canvas = theme::CANVAS;
    canvas[3] = state.settings.glass.alpha();
    d.slab(root, 10.0, canvas, theme::CANVAS_LINE, 1.0);

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

    // The chrome fades in over the glass at launch.
    d.fade = d.anim.mix_from(key("intro", ()), 0.0, 1.0, anim::ENTER);
    d.hairline(Rect::new(0.0, bar.bottom(), w, 1.0));
    d.hairline(Rect::new(nav.right(), body.y, 1.0, body.h));
    if let Some((sub, _)) = perf {
        d.hairline(Rect::new(sub.right(), body.y, 1.0, body.h));
    }

    title_bar(&mut d, state, snap, bar, mouse);
    nav_items(&mut d, state, nav, mouse);

    d.hit(
        Rect::new(nav.right() - 2.0, body.y, 5.0, body.h),
        HitKind::DragNav,
    );

    // A page enters by fading up from a short drop; list rows follow in a
    // cascade (see `stagger`).
    let enter = d
        .anim
        .mix_from(key("page", state.page), 0.0, 1.0, anim::ENTER);
    d.enter = enter;
    d.fade = enter;
    let lift = |r: Rect| Rect::new(r.x, r.y + (1.0 - enter) * PAGE_RISE, r.w, r.h);
    match (state.page, perf) {
        (Page::Performance, Some((sub, detail))) => {
            d.hit(
                Rect::new(sub.right() - 2.0, body.y, 5.0, body.h),
                HitKind::DragSub,
            );
            performance(&mut d, state, snap, lift(sub), detail, mouse)
        }
        (Page::Startup, _) => startup_page(&mut d, state, startup, lift(main), mouse),
        (Page::Processes, _) => processes(&mut d, state, snap, lift(main), mouse),
        (Page::Settings, _) => settings_page(&mut d, state, lift(main), mouse),
        _ => {}
    }
    d.fade = 1.0;

    toast(&mut d, state, main);
    if state.page == Page::Processes {
        context_menu(&mut d, state, snap, Rect::new(0.0, 0.0, w, h), pointer);
    } else {
        state.menu = None;
        state.menu_ghost = None;
    }
    d.anim.end();
    state.anim = std::mem::take(&mut d.anim);
    d
}

/// Drop a page or section rises from as it enters.
const PAGE_RISE: f32 = 10.0;
/// Drop each list row rises from in the entrance cascade.
const ROW_RISE: f32 = 8.0;
/// Farthest a process row travels when re-sorted, in rows.
const MAX_ROW_TRAVEL: f32 = 3.0;

/// Extra fade for the `i`th visible row of a list while its page enters, on
/// top of the page fade, so rows arrive in a quick cascade.
fn stagger(d: &DrawList, i: usize) -> f32 {
    let t = d.enter;
    if t >= 1.0 {
        1.0
    } else if t <= 0.0 {
        0.0
    } else {
        (cascade(t, i) / t).min(1.0)
    }
}

const MENU_ITEM_H: f32 = 30.0;
const MENU_PAD: f32 = 6.0;
const MENU_HEAD_H: f32 = 44.0;
/// Gap between the header rule and the first item, so its highlight clears it.
const MENU_HEAD_GAP: f32 = 5.0;
const MENU_SEP_H: f32 = 11.0;

/// True while something on screen is mid-animation and needs frames.
pub fn animating(state: &AppState) -> bool {
    state.anim.busy() || state.menu_ghost.is_some()
}

const CARET_BLINK: Duration = Duration::from_millis(530);

fn caret_blinks(state: &AppState) -> bool {
    state.settings.animations && state.search_focused && state.page == Page::Processes
}

fn caret_on(state: &AppState) -> bool {
    !caret_blinks(state)
        || (state.typed_at.elapsed().as_millis() / CARET_BLINK.as_millis()) % 2 == 0
}

/// Next moment a still screen must repaint without input: a notice or an
/// armed confirmation expiring (so it can fade out), or the caret blinking.
pub fn wake_at(state: &AppState) -> Option<Instant> {
    let now = Instant::now();
    let mut next: Option<Instant> = None;
    let mut add = |t: Instant| {
        if t > now {
            next = Some(next.map_or(t, |n: Instant| n.min(t)));
        }
    };
    if let Some(n) = &state.notice {
        add(n.until);
    }
    if let Some(a) = &state.armed {
        add(a.until);
    }
    if let Some(t) = state.reset_armed {
        add(t);
    }
    if caret_blinks(state) {
        let blink = CARET_BLINK.as_millis();
        let ticks = state.typed_at.elapsed().as_millis() / blink + 1;
        add(state.typed_at + Duration::from_millis((ticks * blink) as u64));
    }
    next
}

enum MenuRow {
    Item {
        action: MenuAction,
        label: String,
        hint: Option<&'static str>,
        danger: bool,
    },
    Sep,
}

/// Right-click menu for the process list, plus the fading ghost of one that
/// just closed.
fn context_menu(d: &mut DrawList, state: &mut AppState, snap: &Snap, win: Rect, mouse: [f32; 2]) {
    if let Some(mut ghost) = state.menu_ghost.take() {
        // Leave from wherever the entrance got to.
        let from = d.anim.peek(key("menu-in", ghost.opened)).unwrap_or(1.0);
        let a = d
            .anim
            .mix_from(key("menu-out", ghost.opened), from, 0.0, anim::MENU_OUT);
        let off = [f32::NEG_INFINITY; 2];
        if a > 0.0 && menu_panel(d, &mut ghost, snap, win, off, a, false) {
            state.menu_ghost = Some(ghost);
        }
    }
    if let Some(mut menu) = state.menu.take() {
        let a = d
            .anim
            .mix_from(key("menu-in", menu.opened), 0.0, 1.0, anim::MENU_IN);
        if menu_panel(d, &mut menu, snap, win, mouse, a, true) {
            state.menu = Some(menu);
        }
    }
}

/// A floating black-glass panel on the overlay layer. It fades in while
/// settling 6 px down, and leaves the same way. Only a `live` menu takes
/// hits. Returns false once none of its processes exist.
fn menu_panel(
    d: &mut DrawList,
    menu: &mut ContextMenu,
    snap: &Snap,
    win: Rect,
    mouse: [f32; 2],
    a: f32,
    live: bool,
) -> bool {
    let procs: Vec<&Proc> = menu
        .pids
        .iter()
        .filter_map(|pid| snap.procs.iter().find(|p| p.pid == *pid))
        .collect();
    if procs.is_empty() {
        return false;
    }
    let n = procs.len();
    let single = n == 1;
    let any_running = procs.iter().any(|p| !p.stopped);
    let any_stopped = procs.iter().any(|p| p.stopped);

    let mut rows = vec![
        MenuRow::Item {
            action: MenuAction::EndTask,
            label: if single {
                "End task".into()
            } else {
                format!("End {n} tasks")
            },
            hint: None,
            danger: false,
        },
        MenuRow::Item {
            action: MenuAction::ForceKill,
            label: if menu.confirm_kill {
                "Click again to force kill".into()
            } else if single {
                "Force kill".into()
            } else {
                format!("Force kill {n}")
            },
            hint: Some("SIGKILL"),
            danger: true,
        },
        MenuRow::Sep,
    ];
    if any_running {
        rows.push(MenuRow::Item {
            action: MenuAction::Suspend,
            label: "Suspend".into(),
            hint: Some("SIGSTOP"),
            danger: false,
        });
    }
    if any_stopped {
        rows.push(MenuRow::Item {
            action: MenuAction::Resume,
            label: "Resume".into(),
            hint: Some("SIGCONT"),
            danger: false,
        });
    }
    rows.push(MenuRow::Sep);
    if single {
        rows.push(MenuRow::Item {
            action: MenuAction::OpenLocation,
            label: "Open file location".into(),
            hint: None,
            danger: false,
        });
        rows.push(MenuRow::Item {
            action: MenuAction::CopyCommand,
            label: "Copy command line".into(),
            hint: None,
            danger: false,
        });
    }
    rows.push(MenuRow::Item {
        action: MenuAction::CopyPid,
        label: if single {
            "Copy PID".into()
        } else {
            "Copy PIDs".into()
        },
        hint: None,
        danger: false,
    });
    menu.items = rows
        .iter()
        .filter_map(|r| match r {
            MenuRow::Item { action, .. } => Some(*action),
            MenuRow::Sep => None,
        })
        .collect();
    if menu.focus.is_some_and(|i| i >= menu.items.len()) {
        menu.focus = None;
    }

    let (title, detail) = if single {
        (procs[0].name.clone(), format!("PID {}", procs[0].pid))
    } else {
        (format!("{n} processes"), "Selection".to_string())
    };
    let label_w = rows
        .iter()
        .map(|r| match r {
            MenuRow::Item { label, hint, .. } => {
                measure(label, BODY) + hint.map_or(0.0, |h| measure(h, MICRO_NUM) + 24.0)
            }
            MenuRow::Sep => 0.0,
        })
        .fold(measure(&title, BODY), f32::max);
    let w = (label_w + 28.0 + MENU_PAD * 2.0).clamp(200.0, 320.0);
    let h = MENU_PAD * 2.0
        + MENU_HEAD_H
        + MENU_HEAD_GAP
        + rows
            .iter()
            .map(|r| match r {
                MenuRow::Item { .. } => MENU_ITEM_H,
                MenuRow::Sep => MENU_SEP_H,
            })
            .sum::<f32>();

    // Open down-right of the pointer; flip at the window edges.
    let margin = 8.0;
    let mut x = menu.x + 2.0;
    if x + w > win.right() - margin {
        x = menu.x - w - 2.0;
    }
    let mut y = menu.y + 2.0;
    if y + h > win.bottom() - margin {
        y = menu.y - h - 2.0;
    }
    let x = x.clamp(margin, (win.right() - margin - w).max(margin));
    let y = y.clamp(margin, (win.bottom() - margin - h).max(margin));

    let y = y - 6.0 * (1.0 - a);
    let focus = menu.focus;
    let confirm = menu.confirm_kill;
    let id = menu.opened;

    d.layer = OVERLAY;
    let saved_fade = d.fade;
    d.fade = a;
    let panel = Rect::new(x, y, w, h);
    d.slab(panel, 10.0, theme::MENU, theme::GHOST_LINE, 1.0);
    if live {
        d.hit(panel, HitKind::MenuPanel);
    }

    let inner_x = x + MENU_PAD;
    let inner_w = w - MENU_PAD * 2.0;
    let mut cy = y + MENU_PAD;
    d.text(
        &title,
        Rect::new(inner_x + 10.0, cy + 6.0, inner_w - 20.0, 16.0),
        BODY,
        theme::INK,
    );
    d.text(
        &detail,
        Rect::new(inner_x + 10.0, cy + 24.0, inner_w - 20.0, 12.0),
        MICRO_NUM,
        theme::INK_4,
    );
    cy += MENU_HEAD_H;
    d.fill(Rect::new(x, cy - 1.0, w, 1.0), 0.0, theme::HAIRLINE);
    cy += MENU_HEAD_GAP;

    // Item rects first, so the highlight can glide under them.
    let mut item_rects = Vec::new();
    let mut ry = cy;
    for row in &rows {
        match row {
            MenuRow::Sep => ry += MENU_SEP_H,
            MenuRow::Item { .. } => {
                item_rects.push(Rect::new(inner_x, ry, inner_w, MENU_ITEM_H));
                ry += MENU_ITEM_H;
            }
        }
    }
    let hot_index = item_rects
        .iter()
        .position(|r| r.contains(mouse[0], mouse[1]))
        .or(focus);
    let hl = key("menu-highlight", id);
    let ha = d
        .anim
        .hover(key("menu-highlight-on", id), hot_index.is_some());
    let span = match hot_index.and_then(|i| item_rects.get(i)) {
        Some(r) => {
            let (lo, hi) = (r.y - y, r.bottom() - y);
            // Appearing from nothing: light up in place instead of gliding in
            // from wherever the pointer last left.
            if ha < 0.05 {
                d.anim.place(hl, lo, hi);
            }
            Some((lo, hi))
        }
        None => d.anim.peek(key("lo", hl)).zip(d.anim.peek(key("hi", hl))),
    };
    if let Some((lo, hi)) = span {
        let (lo, hi) = d.anim.glide(hl, lo, hi);
        d.faded(ha, |d| {
            d.fill(
                Rect::new(inner_x, y + lo, inner_w, hi - lo),
                6.0,
                theme::HOVER,
            )
        });
    }
    let armed_t = d.anim.toggle(key("menu-armed", id), confirm);

    let mut index = 0;
    for row in &rows {
        match row {
            MenuRow::Sep => {
                d.fill(
                    Rect::new(inner_x + 10.0, cy + MENU_SEP_H * 0.5, inner_w - 20.0, 1.0),
                    0.0,
                    theme::HAIRLINE,
                );
                cy += MENU_SEP_H;
            }
            MenuRow::Item {
                action,
                label,
                hint,
                danger,
            } => {
                let r = Rect::new(inner_x, cy, inner_w, MENU_ITEM_H);
                let hot = hot_index == Some(index);
                let h = d.anim.hover(key("menu-item", (id, index)), hot);
                if *danger {
                    d.slab(
                        r,
                        6.0,
                        NONE,
                        mix_rgba(NONE, theme::DANGER_LINE, armed_t),
                        1.0,
                    );
                }
                let ink = if *danger {
                    mix_rgba(theme::INK_2, theme::DANGER_INK, h.max(armed_t))
                } else {
                    mix_rgba(theme::INK_2, theme::INK, h)
                };
                d.text(
                    label,
                    Rect::new(r.x + 10.0, r.y, r.w - 20.0, r.h),
                    BODY,
                    ink,
                );
                if let Some(hint) = hint {
                    d.text_r(
                        hint,
                        Rect::new(r.x + 10.0, r.y, r.w - 20.0, r.h),
                        MICRO_NUM,
                        theme::INK_4,
                    );
                }
                if live {
                    d.hit(r, HitKind::MenuItem(*action));
                }
                cy += MENU_ITEM_H;
                index += 1;
            }
        }
    }
    d.fade = saved_fade;
    d.layer = BASE;
    true
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
        (Icon::Min, HitKind::Minimize),
        (Icon::Close, HitKind::Close),
    ];
    let cy = bar.y + bar.h * 0.5;
    let n = specs.len() as f32;
    let mut left_edge = bar.right();
    for (i, (ic, kind)) in specs.iter().enumerate() {
        let r = Rect::new(
            bar.right() - 12.0 - (n - i as f32) * 32.0,
            cy - 14.0,
            28.0,
            28.0,
        );
        left_edge = left_edge.min(r.x);
        let hot = r.contains(mouse[0], mouse[1]);
        let is_close = matches!(kind, HitKind::Close);
        let h = d.anim.hover(key("control", *kind), hot);
        d.fill(r, 14.0, mix_rgba(NONE, theme::HOVER, h));
        let lit = if is_close {
            theme::DANGER_INK
        } else {
            theme::INK
        };
        icon(d, *ic, r.x + 7.0, r.y + 7.0, mix_rgba(theme::INK_3, lit, h));
        d.hit(r, *kind);
    }

    // Live readout, instrument style, ahead of the controls.
    let mut right = left_edge - 24.0;
    let shown = d.anim.toggle(key("readout", ()), state.settings.readout);
    if shown > 0.0 {
        let gpu = snap
            .gpus
            .first()
            .and_then(|g| g.util)
            .map(percent)
            .unwrap_or_else(|| "—".into());
        let readout = format!(
            "cpu {}   mem {}   gpu {}",
            percent(snap.cpu_total),
            mem_short(snap),
            gpu
        );
        let rw = measure(&readout.to_uppercase(), MICRO_NUM);
        d.faded(shown, |d| {
            d.text_r(
                &readout,
                Rect::new(right - rw, bar.y, rw + 2.0, bar.h),
                MICRO_NUM,
                theme::INK_3,
            )
        });
        if state.settings.readout {
            right -= rw + 24.0;
        }
    }
    // Frozen numbers must never pass for live ones, so this shows either way.
    let paused = d.anim.toggle(key("paused", ()), state.paused);
    if paused > 0.0 {
        let pw = measure("PAUSED", MICRO_NUM);
        d.faded(paused, |d| {
            d.text_r(
                "paused",
                Rect::new(right - pw, bar.y, pw + 2.0, bar.h),
                MICRO_NUM,
                theme::WARN,
            )
        });
    }
}

/// Side list entry. The selected fill is drawn by [`side_highlight`], which
/// glides between entries; this draws hover and ink, both cross-faded.
#[allow(clippy::too_many_arguments)]
fn side_item(
    d: &mut DrawList,
    id: HitKind,
    r: Rect,
    ic: Icon,
    label: &str,
    detail: Option<&str>,
    on: bool,
    mouse: [f32; 2],
) {
    let hot = !on && r.contains(mouse[0], mouse[1]);
    let h = d.anim.hover(key("side-hover", id), hot);
    let o = d.anim.toggle(key("side-on", id), on);
    d.fill(r, 6.0, mix_rgba(NONE, theme::HOVER, h));
    // Glyph and label share one ink, so no page reads as disabled.
    let ink = mix_rgba(theme::INK_2, theme::INK, h.max(o));
    icon(d, ic, r.x + 12.0, r.y + (r.h - 14.0) * 0.5, ink);
    d.text(label, Rect::new(r.x + 36.0, r.y, r.w * 0.55, r.h), NAV, ink);
    if let Some(detail) = detail {
        d.text_r(
            detail,
            Rect::new(r.x + r.w * 0.5, r.y, r.w * 0.5 - 12.0, r.h),
            MICRO_NUM,
            mix_rgba(theme::INK_4, theme::INK_2, o),
        );
    }
}

/// Selected fill behind a side list. Offsets are from `origin` (the list's
/// top), so the fill follows the list but glides between entries.
fn side_highlight(d: &mut DrawList, tag: &str, origin: f32, sel: Option<Rect>) {
    let Some(r) = sel else { return };
    let (lo, hi) = d.anim.glide(
        key("side-highlight", tag),
        r.y - origin,
        r.bottom() - origin,
    );
    d.fill(Rect::new(r.x, origin + lo, r.w, hi - lo), 6.0, theme::GHOST);
}

fn nav_items(d: &mut DrawList, state: &AppState, nav: Rect, mouse: [f32; 2]) {
    eyebrow(d, nav.x + 20.0, nav.y + 18.0, nav.w - 40.0, "Monitor", None);
    let mut entries: Vec<(Icon, &str, Page, Rect)> = [
        (Icon::List, "Processes", Page::Processes),
        (Icon::Pulse, "Performance", Page::Performance),
        (Icon::Power, "Startup", Page::Startup),
    ]
    .iter()
    .enumerate()
    .map(|(i, (ic, label, page))| {
        let y = nav.y + 44.0 + i as f32 * 38.0;
        (
            *ic,
            *label,
            *page,
            Rect::new(nav.x + 10.0, y, nav.w - 20.0, 34.0),
        )
    })
    .collect();
    // Settings is about the app, not the machine: pinned to the foot, clear
    // of the monitor pages, just above the version.
    let y = nav.y + 44.0 + 3.0 * 38.0;
    entries.push((
        Icon::Cog,
        "Settings",
        Page::Settings,
        Rect::new(
            nav.x + 10.0,
            (nav.bottom() - 82.0).max(y + 8.0),
            nav.w - 20.0,
            34.0,
        ),
    ));
    let sel = entries.iter().find(|e| e.2 == state.page).map(|e| e.3);
    side_highlight(d, "nav", nav.y, sel);
    for (ic, label, page, r) in entries {
        let id = HitKind::Page(page);
        side_item(d, id, r, ic, label, None, state.page == page, mouse);
        d.hit(r, id);
    }
    d.text(
        &format!("v{}", env!("CARGO_PKG_VERSION")),
        Rect::new(nav.x + 20.0, nav.bottom() - 30.0, nav.w - 40.0, 14.0),
        MICRO_NUM,
        theme::INK_4,
    );
}

/// Floating notice. It rises in, re-fits its width when the message
/// changes, and sinks out after it expires; then it clears the notice.
fn toast(d: &mut DrawList, state: &mut AppState, main: Rect) {
    let Some(notice) = &state.notice else { return };
    let visible = notice.until > Instant::now();
    let label = notice.label.clone();
    let a = d
        .anim
        .mix_from(key("toast", ()), 0.0, visible as u8 as f32, anim::TOAST);
    if !visible && a <= 0.0 {
        state.notice = None;
        return;
    }
    let label_w = measure(&label, BODY);
    let w = d.anim.slide(
        key("toast-w", ()),
        (label_w + 36.0).max(120.0),
        anim::TOGGLE,
    );
    let rise = (1.0 - a) * 14.0;
    let r = Rect::new(
        main.x + (main.w - w) * 0.5,
        main.bottom() - 60.0 + rise,
        w,
        38.0,
    );
    d.layer = OVERLAY;
    d.faded(a, |d| {
        d.slab(r, 19.0, theme::TOAST, theme::GHOST_LINE, 1.0);
        d.text(
            &label,
            Rect::new(r.x + (r.w - label_w) * 0.5 - 2.0, r.y, label_w + 4.0, r.h),
            BODY,
            theme::INK,
        );
    });
    d.layer = BASE;
}

// --- Processes --------------------------------------------------------------

fn processes(d: &mut DrawList, state: &mut AppState, snap: &Snap, main: Rect, mouse: [f32; 2]) {
    let inner = Rect::new(main.x + 24.0, main.y + 18.0, main.w - 48.0, main.h - 42.0);
    let y = inner.y;
    let ctl_h = PILL_H;

    // Two clusters on one line. Left narrows the list: scope, then search.
    // Right acts on it: density, then End task at the far edge.
    let views = [
        ("Grouped", ProcView::Grouped),
        ("Flat", ProcView::Flat),
        ("User", ProcView::User),
        ("System", ProcView::System),
    ];
    let view_items: Vec<(&str, bool, HitKind)> = views
        .iter()
        .map(|(l, v)| (*l, state.view == *v, HitKind::View(*v)))
        .collect();

    let end = selection_label(state);
    let (dense_icon, dense) = if state.density == Density::Compact {
        (Icon::Dense, "Compact")
    } else {
        (Icon::Loose, "Comfortable")
    };
    // Pills re-fit their labels smoothly; search takes up the slack.
    let ew = d.anim.slide(
        key("pill-w", HitKind::EndTask),
        pill_w(&end, false),
        anim::TOGGLE,
    );
    let dw = d.anim.slide(
        key("pill-w", HitKind::Density),
        pill_w(dense, true),
        anim::TOGGLE,
    );

    let seg = segmented(d, inner.x, y, &view_items, mouse);

    // Right cluster anchors the trailing edge; search fills the mid band so the
    // gap matches PILL_GAP on both sides (a 240px cap left a dead hole before).
    let er = Rect::new(inner.right() - ew, y, ew, ctl_h);
    let dr = Rect::new(er.x - PILL_GAP - dw, y, dw, ctl_h);
    let search_x = seg.right() + PILL_GAP;
    let search_w = (dr.x - PILL_GAP - search_x).max(96.0);
    let sr = Rect::new(search_x, y, search_w, ctl_h);
    let focus = state.search_focused;
    let typing = focus || !state.query.is_empty();
    let hot = sr.contains(mouse[0], mouse[1]);
    let h = d.anim.hover(key("search-hover", ()), hot);
    let f = d.anim.toggle(key("search-focus", ()), focus);
    let t = d.anim.toggle(key("search-typing", ()), typing);
    d.slab(
        sr,
        ctl_h * 0.5,
        mix_rgba(NONE, theme::GHOST, f),
        mix_rgba(theme::GHOST_LINE, theme::ACCENT_LINE, h.max(f)),
        1.0,
    );
    icon(
        d,
        Icon::Search,
        sr.x + 12.0,
        sr.y + (ctl_h - 14.0) * 0.5,
        mix_rgba(theme::INK_3, theme::INK, h.max(t)),
    );
    // Mono, so the blinking caret swaps with a space without shifting text.
    let q = if state.query.is_empty() && !focus {
        "Search".to_string()
    } else if focus {
        let caret = if caret_on(state) { '|' } else { ' ' };
        format!("{}{caret}", state.query)
    } else {
        state.query.clone()
    };
    d.text(
        &q,
        Rect::new(sr.x + 34.0, sr.y, sr.w - 34.0 - PILL_PAD, sr.h),
        if typing { NUM } else { PILL },
        if typing {
            theme::INK
        } else if hot {
            theme::INK_2
        } else {
            theme::INK_3
        },
    );
    d.hit(sr, HitKind::Search);

    let armed = state
        .armed
        .as_ref()
        .is_some_and(|a| a.until > Instant::now() && a.pids == state.selected);
    ghost_pill(
        d,
        HitKind::EndTask,
        er,
        None,
        &end,
        mouse,
        !state.selected.is_empty(),
        armed,
    );
    d.hit(er, HitKind::EndTask);

    ghost_pill(
        d,
        HitKind::Density,
        dr,
        Some(dense_icon),
        dense,
        mouse,
        true,
        false,
    );
    d.hit(dr, HitKind::Density);

    // Column header metrics — painted after the list so scrolled row ink
    // cannot cover the labels / hairline.
    let y = y + ctl_h + 20.0;
    let cols = columns(state.density, &state.settings, inner.w);
    let cpu_div = match state.settings.proc_cpu {
        ProcCpu::Core => 1.0,
        ProcCpu::Machine => snap.cpu_per.len().max(1) as f32,
    };
    let header = Rect::new(inner.x, y, inner.w, 16.0);
    let hair_y = y + 24.0;
    let y = y + 25.0;
    let list = Rect::new(inner.x, y, inner.w, (inner.bottom() - y).max(20.0));
    d.list_rect = Some(list);

    // Drop selections whose process has exited so End task cannot hit a recycled PID.
    if !state.selected.is_empty() {
        state
            .selected
            .retain(|pid| snap.procs.iter().any(|p| p.pid == *pid));
        state.pinned.retain(|pid, _| state.selected.contains(pid));
        if state.selected.is_empty() {
            state.pinned.clear();
            state.anchor = None;
        }
    }
    let rows = visible_rows(state, snap);
    // Processes missing from the last list fade in where they land.
    let known: std::collections::HashSet<i32> = state.visible_pids.iter().copied().collect();
    state.visible_pids = rows
        .iter()
        .filter_map(|r| match r {
            Row::Proc(p) => Some(p.pid),
            Row::Header { .. } => None,
        })
        .collect();
    // After a view/group change cleared pins, freeze selected rows at their new spots.
    if !state.selected.is_empty() && state.pinned.is_empty() {
        for &pid in &state.selected {
            if let Some(i) = state.visible_pids.iter().position(|p| *p == pid) {
                state.pinned.insert(pid, i);
            }
        }
    }
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
    let scroll = smooth_scroll(d, state, ScrollBar::Processes, (), state.scroll);
    let drawn_h = d.anim.slide(key("row-h", ()), row_h, anim::REORDER);
    // Under the rows: empty list space clears selection / unfreezes pins.
    d.hit(list, HitKind::Deselect);
    d.clip = Some(list);
    let fresh_ok = !known.is_empty();
    let mut shown = 0;
    let (near_lo, near_hi) = (scroll - row_h * 2.0, scroll + list.h + row_h * 2.0);
    let near = |y: f32| y > near_lo && y < near_hi;
    for (i, row) in rows.iter().enumerate() {
        // Rows glide to a new slot. One sorted in from far away starts a few
        // rows short of it instead of streaking across the list, and a move
        // that starts and ends off screen just lands.
        let id = match row {
            Row::Header { user, .. } => key("row-header", *user),
            Row::Proc(p) => key("row", p.pid),
        };
        let target = i as f32 * row_h;
        if let Some(cur) = d.anim.peek(id) {
            if !near(cur) && !near(target) {
                d.anim.set(id, target);
            } else if (cur - target).abs() > row_h * MAX_ROW_TRAVEL {
                d.anim.set(
                    id,
                    target + (cur - target).signum() * row_h * MAX_ROW_TRAVEL,
                );
            }
        }
        let slot_y = d.anim.slide(id, target, anim::REORDER);
        let ry = list.y + slot_y - scroll;
        if ry + drawn_h < list.y || ry > list.bottom() {
            continue;
        }
        let mut a = stagger(d, shown);
        shown += 1;
        if let Row::Proc(p) = row {
            let from = if fresh_ok && !known.contains(&p.pid) {
                0.0
            } else {
                1.0
            };
            a *= d
                .anim
                .mix_from(key("row-in", p.pid), from, 1.0, anim::ENTER);
        }
        let ry = ry + (1.0 - a) * ROW_RISE;
        let rr = Rect::new(list.x, ry, list.w, drawn_h);
        let saved_fade = d.fade;
        d.fade *= a;
        match row {
            Row::Header {
                title,
                count,
                open,
                user,
            } => {
                let turn = d.anim.toggle(key("chevron", *user), *open);
                chevron(
                    d,
                    list.x + 2.0,
                    ry + drawn_h * 0.5 - 7.0,
                    turn,
                    theme::INK_4,
                );
                eyebrow(
                    d,
                    list.x + 24.0,
                    ry + (drawn_h - 14.0) * 0.5,
                    200.0,
                    title,
                    Some(&count.to_string()),
                );
                d.hit(rr, HitKind::Group(*user));
            }
            Row::Proc(p) => {
                let on = state.selected.contains(&p.pid);
                let hot = !on && rr.contains(mouse[0], mouse[1]);
                let h = d.anim.hover(key("row-hover", p.pid), hot);
                let sel = d.anim.toggle(key("row-selected", p.pid), on);
                let hover = mix_rgba(NONE, theme::HOVER, h);
                d.fill(rr, 6.0, mix_rgba(hover, theme::SELECTED, sel));
                draw_proc(d, &cols, rr, p, cpu_div);
                d.hit(rr, HitKind::Proc { pid: p.pid });
            }
        }
        d.fade = saved_fade;
    }
    d.clip = None;
    draw_header(d, &cols, header, state.sort);
    d.hairline(Rect::new(inner.x, hair_y, inner.w, 1.0));
    scrollbar(
        d,
        state,
        list,
        content_h,
        scroll,
        ScrollBar::Processes,
        mouse,
    );
    if rows.is_empty() {
        let a = d.anim.mix_from(key("empty", ()), 0.0, 1.0, anim::ENTER);
        d.faded(a, |d| {
            d.text(
                "No matching processes",
                Rect::new(list.x, list.y + 12.0 + (1.0 - a) * ROW_RISE, list.w, 24.0),
                BODY,
                theme::INK_3,
            )
        });
    }
}

/// Group disclosure chevron: points right when `open` is 0 and turns a
/// quarter clockwise to point down at 1.
fn chevron(d: &mut DrawList, x: f32, y: f32, open: f32, c: theme::Rgba) {
    let (sin, cos) = (open * std::f32::consts::FRAC_PI_2).sin_cos();
    let (cx, cy) = (x + 7.0, y + 7.0);
    let pts: Vec<[f32; 2]> = [[-2.0_f32, -4.0], [2.0, 0.0], [-2.0, 4.0]]
        .iter()
        .map(|[dx, dy]| [cx + dx * cos - dy * sin, cy + dx * sin + dy * cos])
        .collect();
    d.line(&pts, ICON_W, c);
}

/// Scroll offset to draw a pane at. It glides toward the target set by the
/// wheel and keys, but follows a dragged thumb exactly. `scope` separates
/// contents that share a pane, so switching them does not scroll between.
fn smooth_scroll(
    d: &mut DrawList,
    state: &AppState,
    which: ScrollBar,
    scope: impl std::hash::Hash,
    target: f32,
) -> f32 {
    let k = key("scroll", (which, scope));
    if matches!(state.drag, Some(Drag::Scroll { which: w, .. }) if w == which) {
        d.anim.set(k, target);
        target
    } else {
        d.anim.slide(k, target, anim::SCROLL)
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
        let procs = pin_procs(procs, &state.pinned, 0);
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
    let mut pin_base = 0usize;
    if state.user_open {
        let n = user.len();
        let user = pin_procs(user, &state.pinned, pin_base);
        pin_base += n;
        rows.extend(user.into_iter().map(Row::Proc));
    }
    rows.push(Row::Header {
        title: "System",
        count: system.len(),
        open: state.system_open,
        user: false,
    });
    if state.system_open {
        let system = pin_procs(system, &state.pinned, pin_base);
        rows.extend(system.into_iter().map(Row::Proc));
    }
    rows
}

/// Keep pinned processes at the indices captured when they were selected.
/// `index_base` is this slice's start inside `visible_pids`.
fn pin_procs<'a>(
    procs: Vec<&'a Proc>,
    pinned: &std::collections::BTreeMap<i32, usize>,
    index_base: usize,
) -> Vec<&'a Proc> {
    if pinned.is_empty() || procs.is_empty() {
        return procs;
    }
    let n = procs.len();
    let mut slots: Vec<Option<&'a Proc>> = vec![None; n];
    let mut rest = Vec::with_capacity(n);
    for p in procs {
        if let Some(&idx) = pinned.get(&p.pid) {
            if idx >= index_base && idx < index_base + n {
                let local = idx - index_base;
                if slots[local].is_none() {
                    slots[local] = Some(p);
                    continue;
                }
            }
        }
        rest.push(p);
    }
    let mut ri = 0usize;
    for slot in &mut slots {
        if slot.is_none() {
            if let Some(p) = rest.get(ri) {
                *slot = Some(*p);
                ri += 1;
            }
        }
    }
    let mut out: Vec<&'a Proc> = slots.into_iter().flatten().collect();
    if ri < rest.len() {
        out.extend(rest.into_iter().skip(ri));
    }
    out
}

fn sort_procs(procs: &mut [&Proc], col: Col, desc: bool) {
    procs.sort_by(|a, b| {
        let ord = match col {
            Col::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            Col::Cpu => a
                .cpu
                .partial_cmp(&b.cpu)
                .unwrap_or(std::cmp::Ordering::Equal),
            Col::Gpu => a
                .gpu
                .partial_cmp(&b.gpu)
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

fn columns(density: Density, prefs: &Settings, width: f32) -> Vec<ColSpec> {
    let all: &[(Col, f32, bool, bool)] = if density == Density::Compact {
        &[
            (Col::Pid, 68.0, true, true),
            (Col::Memory, 88.0, true, true),
            (Col::Gpu, 64.0, true, true),
            (Col::Cpu, 68.0, true, true),
        ]
    } else {
        &[
            (Col::Threads, 76.0, true, true),
            (Col::User, 96.0, false, false),
            (Col::Pid, 72.0, true, true),
            (Col::Disk, 96.0, true, true),
            (Col::Memory, 88.0, true, true),
            (Col::Gpu, 68.0, true, true),
            (Col::Cpu, 72.0, true, true),
        ]
    };
    let spec: Vec<(Col, f32, bool, bool)> = all
        .iter()
        .copied()
        .filter(|(c, ..)| prefs.shows(*c))
        .collect();
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
        Col::Gpu => "GPU",
        Col::Memory => "Memory",
        Col::Disk => "Disk",
        Col::Pid => "PID",
        Col::User => "User",
        Col::Threads => "Threads",
    }
}

/// Column titles. The sorted column brightens and shows a caret that
/// flattens and flips when the direction changes.
fn draw_header(d: &mut DrawList, cols: &[ColSpec], row: Rect, sort: Sort) {
    for c in cols {
        let r = Rect::new(row.x + c.x, row.y, c.w - 10.0, row.h);
        let active = c.col == sort.col;
        let on = d.anim.toggle(key("sort-on", c.col), active);
        let color = mix_rgba(theme::INK_3, theme::INK, on);
        let title = col_title(c.col);
        if c.right {
            d.text_r(title, r, MICRO, color);
        } else {
            d.text(title, r, MICRO, color);
        }
        if on > 0.0 {
            let tw = measure(&title.to_uppercase(), MICRO);
            let cx = if c.right {
                r.right() - tw - 12.0
            } else {
                r.x + tw + 6.0
            };
            let cy = row.y + row.h * 0.5 - 2.0;
            let desc = d.anim.toggle(key("sort-desc", c.col), sort.desc);
            let edge = lerp(cy + 3.5, cy, desc);
            let mid = lerp(cy, cy + 3.5, desc);
            d.faded(on, |d| {
                d.line(
                    &[[cx, edge], [cx + 3.0, mid], [cx + 6.0, edge]],
                    1.1,
                    theme::INK_2,
                )
            });
        }
        d.hit(
            Rect::new(row.x + c.x, row.y - 4.0, c.w, row.h + 8.0),
            HitKind::Sort(c.col),
        );
    }
}

/// `cpu_div` rescales per-process CPU: 1 for share of one core, the core
/// count for share of the whole machine.
fn draw_proc(d: &mut DrawList, cols: &[ColSpec], row: Rect, p: &Proc, cpu_div: f32) {
    let cpu = p.cpu / cpu_div;
    for c in cols {
        let r = Rect::new(row.x + c.x, row.y, c.w - 10.0, row.h);
        let text = match c.col {
            Col::Name => p.name.clone(),
            Col::Cpu => cpu_pct(cpu),
            Col::Gpu => cpu_pct(p.gpu),
            Col::Memory => bytes(p.rss),
            Col::Disk => disk_cell(p.read_bps, p.write_bps),
            Col::Pid => p.pid.to_string(),
            Col::User => p.user.clone(),
            Col::Threads => p.threads.to_string(),
        };
        // Idle cells step back so the rows doing something read first.
        let idle = match c.col {
            Col::Cpu => cpu < 0.05,
            Col::Gpu => p.gpu < 0.05,
            Col::Disk => disk_sum(p) < 1.0,
            _ => false,
        };
        let color = match c.col {
            Col::Name if p.stopped => theme::INK_3,
            Col::Name => theme::INK,
            Col::Cpu if idle => theme::INK_4,
            Col::Cpu => heat(d, cpu, theme::INK),
            Col::Gpu if idle => theme::INK_4,
            Col::Gpu => heat(d, p.gpu, theme::INK),
            _ if idle => theme::INK_4,
            _ => theme::INK_2,
        };
        let t = if c.mono { NUM } else { BODY };
        if c.right {
            d.text_r(&text, r, t, color);
        } else {
            d.text(&text, r, t, color);
        }
        if c.col == Col::Name && p.stopped {
            let nw = measure(&text, BODY);
            let tag = Rect::new(r.x + nw + 10.0, r.y, (r.w - nw - 10.0).max(0.0), r.h);
            if tag.w >= measure("SUSPENDED", MICRO) {
                d.text("suspended", tag, MICRO, theme::INK_4);
            }
        }
    }
}

/// Room kept free at the right of a scrolling list so its thumb never sits
/// on a row's trailing control.
const SCROLL_GUTTER: f32 = 16.0;

fn scrollbar(
    d: &mut DrawList,
    state: &mut AppState,
    viewport: Rect,
    content_h: f32,
    scroll: f32,
    which: crate::model::ScrollBar,
    mouse: [f32; 2],
) {
    if content_h <= viewport.h + 1.0 {
        return;
    }
    let thumb_h = (viewport.h * viewport.h / content_h).clamp(24.0, viewport.h);
    let max_scroll = content_h - viewport.h;
    let track = (viewport.h - thumb_h).max(1.0);
    let t = (scroll / max_scroll).clamp(0.0, 1.0);
    let thumb_y = viewport.y + track * t;
    let hit = Rect::new(viewport.right() - 10.0, viewport.y, 12.0, viewport.h);
    let hot = hit.contains(mouse[0], mouse[1])
        || matches!(state.drag, Some(crate::model::Drag::Scroll { which: w, .. }) if w == which);
    // Visual thumb stays slim and thickens a little under the pointer; the
    // hit strip is wider so it is easy to grab.
    let h = d.anim.hover(key("thumb", which), hot);
    let tw = 2.0 + 1.5 * h;
    d.fill(
        Rect::new(viewport.right() - 1.0 - tw, thumb_y, tw, thumb_h),
        tw * 0.5,
        mix_rgba(theme::INK_4, theme::INK_2, h),
    );
    state.scroll_bar = Some(crate::model::ScrollGeom {
        which,
        track_y: viewport.y,
        track_h: viewport.h,
        thumb_y,
        thumb_h,
        max_scroll,
    });
    d.hit(hit, HitKind::Scroll(which));
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
            snap.gpus.first().and_then(|g| g.util.map(percent)),
        ),
        (Icon::Disk, "Disk", Section::Disk, None),
        (Icon::Net, "Network", Section::Net, None),
    ];
    let slot = |i: usize| {
        Rect::new(
            sub.x + 10.0,
            sub.y + 44.0 + i as f32 * 38.0,
            sub.w - 20.0,
            34.0,
        )
    };
    let sel = items.iter().position(|it| it.2 == state.section).map(slot);
    side_highlight(d, "section", sub.y, sel);
    for (i, (ic, label, section, extra)) in items.into_iter().enumerate() {
        let r = slot(i);
        let id = HitKind::Section(section);
        side_item(
            d,
            id,
            r,
            ic,
            label,
            extra.as_deref(),
            state.section == section,
            mouse,
        );
        d.hit(r, id);
    }

    // A new section enters like a page, inside the detail pane only.
    let enter = d
        .anim
        .mix_from(key("section", state.section), 0.0, 1.0, anim::ENTER);
    let rise = (1.0 - enter.min(d.enter)) * PAGE_RISE;
    let view = Rect::new(
        detail.x + 28.0,
        detail.y + 22.0,
        detail.w - 56.0,
        detail.h - 46.0,
    );
    d.detail_rect = Some(view);
    let scroll = smooth_scroll(
        d,
        state,
        ScrollBar::Performance,
        state.section,
        state.perf_scroll,
    );
    let y0 = view.y - scroll + rise;
    let head = state.perf_smooth.head();
    d.clip = Some(view);
    let saved_fade = d.fade;
    d.fade *= enter;
    let content_bottom = match state.section {
        Section::Cpu => cpu_page(d, snap, view, y0, head),
        Section::Memory => memory_page(d, snap, view, y0, head),
        Section::Gpu => gpu_page(d, state, snap, view, y0, head),
        Section::Disk => io_page(d, state, view, y0, true, snap, head),
        Section::Net => io_page(d, state, view, y0, false, snap, head),
    };
    d.fade = saved_fade;
    d.clip = None;
    let content_h = (content_bottom - y0).max(0.0);
    let max_scroll = (content_h - view.h).max(0.0);
    if state.perf_scroll > max_scroll {
        state.perf_scroll = max_scroll;
    }
    scrollbar(
        d,
        state,
        view,
        content_h,
        scroll,
        ScrollBar::Performance,
        mouse,
    );
}

fn window_label(prefs: &Settings) -> String {
    format!("{} window", prefs.history.label())
}

fn mem_short_n(used: f32, total: u64) -> String {
    if total == 0 {
        "—".into()
    } else {
        percent(used / total as f32 * 100.0)
    }
}

fn mem_short(snap: &Snap) -> String {
    mem_short_n(snap.mem_used as f32, snap.mem_total)
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
    head: f32,
) {
    d.graph(r, series, max, head);
    d.text_r(
        max_label,
        Rect::new(r.x, r.y - 16.0, r.w, 14.0),
        MICRO_NUM,
        theme::INK_4,
    );
}

fn cpu_page(d: &mut DrawList, snap: &Snap, view: Rect, mut y: f32, head: f32) -> f32 {
    let top = y;
    let prefs = d.prefs;
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

    eyebrow(
        d,
        view.x,
        y,
        view.w,
        "Utilization",
        Some(&window_label(&prefs)),
    );
    y += 24.0;
    scaled_graph(
        d,
        Rect::new(view.x, y, view.w, graph_h),
        &[(&snap.cpu_hist, theme::TRACE)],
        100.0,
        "100%",
        head,
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
        Some(&format!("{n} logical   peak {}", percent(peak))),
    );
    y += 24.0;
    // Bars read the per-core rings at the graph's playback head, so they glide
    // on the same clock and curve as the utilization trace above.
    let cores: Vec<f32> = snap
        .cpu_per_hist
        .iter()
        .map(|h| {
            let idx = h.len() as f32 - 1.0 + head;
            if idx < 0.0 {
                0.0
            } else {
                curve_at(h, idx, prefs.curve)
            }
        })
        .collect();
    equalizer(d, Rect::new(view.x, y, view.w, cores_h), &cores);
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
        let color = heat(d, *v, theme::INK_2);
        d.fill(Rect::new(x, r.bottom() - h, bw, h), 1.0, color);
    }
    d.hairline(Rect::new(r.x, r.bottom(), r.w, 1.0));
}

fn memory_page(d: &mut DrawList, snap: &Snap, view: Rect, mut y: f32, head: f32) -> f32 {
    let top = y;
    let prefs = d.prefs;
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

    eyebrow(d, view.x, y, view.w, "Usage", Some(&window_label(&prefs)));
    y += 24.0;
    scaled_graph(
        d,
        Rect::new(view.x, y, view.w, graph_h),
        &[(&snap.mem_hist, theme::TRACE)],
        1.0,
        &bytes(snap.mem_total),
        head,
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
            head,
        );
        y += swap_h + 20.0;
    }
    y
}

fn gpu_page(
    d: &mut DrawList,
    state: &AppState,
    snap: &Snap,
    view: Rect,
    mut y: f32,
    head: f32,
) -> f32 {
    let top = y;
    let prefs = d.prefs;
    if snap.gpus.is_empty() {
        y = page_title(d, "Graphics", "No GPU reported", view, y);
        return y;
    }
    for (gi, g) in snap.gpus.iter().enumerate() {
        y = page_title(d, "Graphics", &g.name, view, y);
        let util = g.util.map(percent).unwrap_or_else(|| "—".into());
        y = readout(d, view, y, &util, "Utilization");

        // Telemetry as a stat row.
        let mut stats: Vec<(String, &str)> = Vec::new();
        if let Some(t) = g.temp_c {
            stats.push((prefs.temp(t), "Temp"));
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
        eyebrow(
            d,
            view.x,
            y,
            view.w,
            "Utilization",
            Some(&window_label(&prefs)),
        );
        y += 24.0;
        scaled_graph(
            d,
            Rect::new(view.x, y, view.w, graph_h),
            &[(&g.util_hist, theme::TRACE)],
            100.0,
            "100%",
            head,
        );
        y += graph_h + 28.0;

        if g.mem_total > 0 {
            let frac = state.perf_smooth.vram.get(gi).copied().unwrap_or(0.0);
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

fn io_page(
    d: &mut DrawList,
    state: &AppState,
    view: Rect,
    mut y: f32,
    disk: bool,
    snap: &Snap,
    head: f32,
) -> f32 {
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
        let goal = io_scale(a_hist, b_hist, head, state.settings.window());
        let key = format!("{}:{name}", if disk { "disk" } else { "net" });
        let max = state.perf_smooth.io_max(&key).unwrap_or(goal);
        let gr = Rect::new(view.x, y, view.w, graph_h);
        d.graph(
            gr,
            &[(a_hist, theme::TRACE), (b_hist, theme::TRACE_2)],
            max,
            head,
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
            &rate(goal as f64),
            Rect::new(gr.x, ly, gr.w, 12.0),
            MICRO_NUM,
            theme::INK_4,
        );
        y += graph_h + 24.0;
    }
    y
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
    let scroll = smooth_scroll(d, state, ScrollBar::Startup, (), state.startup_scroll);
    let first = (scroll / row_h).floor() as usize;
    let nvis = ((list.h / row_h).ceil() as usize) + 2;
    d.clip = Some(list);
    let full = list;
    let list = Rect::new(list.x, list.y, (list.w - SCROLL_GUTTER).max(40.0), list.h);
    for (n, (i, entry)) in startup
        .iter()
        .enumerate()
        .skip(first)
        .take(nvis)
        .enumerate()
    {
        let ry = list.y + i as f32 * row_h - scroll;
        if ry + row_h < list.y || ry > list.bottom() {
            continue;
        }
        let a = stagger(d, n);
        let ry = ry + (1.0 - a) * ROW_RISE;
        let saved_fade = d.fade;
        d.fade *= a;
        let rr = Rect::new(full.x, ry, full.w, row_h);
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
            Rect::new(list.right() - 32.0, ry + 17.0, 32.0, 18.0),
            entry.enabled,
            key("startup-switch", &entry.path),
        );
        d.hit(rr, HitKind::Startup(i));
        d.fade = saved_fade;
    }
    d.clip = None;
    scrollbar(d, state, full, content_h, scroll, ScrollBar::Startup, mouse);
}

// --- Settings ---------------------------------------------------------------

enum Ctl {
    /// Segmented control. Items are (label, on, hit).
    Choice(Vec<(&'static str, bool, HitKind)>),
    Switch(Opt, bool),
    /// Independent toggles, one ghost pill each; on is the white pill.
    Chips(Vec<(&'static str, bool, HitKind)>),
    Scale(f32),
    Reset(bool),
}

struct SetRow {
    label: &'static str,
    desc: &'static str,
    ctl: Ctl,
}

fn choice<T: Choice>(opt: Opt, current: T) -> Ctl {
    Ctl::Choice(
        T::ALL
            .iter()
            .enumerate()
            .map(|(i, v)| (v.label(), *v == current, HitKind::Setting(opt, i as u8)))
            .collect(),
    )
}

fn settings_groups(state: &AppState) -> Vec<(&'static str, Vec<SetRow>)> {
    let s = &state.settings;
    let row = |label, desc, ctl| SetRow { label, desc, ctl };
    let mut speed = match choice(Opt::Speed, s.speed) {
        Ctl::Choice(items) => items,
        _ => unreachable!(),
    };
    if state.paused {
        for item in &mut speed {
            item.1 = false;
        }
    }
    speed.push((
        "Pause",
        state.paused,
        HitKind::Setting(Opt::Speed, Speed::ALL.len() as u8),
    ));
    let density = if state.density == Density::Compact {
        1
    } else {
        0
    };
    vec![
        (
            "Appearance",
            vec![
                row(
                    "Interface scale",
                    "Ctrl + and Ctrl - work anywhere, Ctrl 0 resets",
                    Ctl::Scale(state.ui_scale),
                ),
                row(
                    "Glass",
                    "How much of the desktop blur shows through the window",
                    choice(Opt::Glass, s.glass),
                ),
                row(
                    "Animations",
                    "Fades, glides and transitions across the interface",
                    Ctl::Switch(Opt::Animations, s.animations),
                ),
                row(
                    "Graph motion",
                    "Reduced steps graphs once per sample and snaps their meters",
                    choice(Opt::Motion, s.motion),
                ),
                row(
                    "Row density",
                    "Spacing of the process list",
                    Ctl::Choice(vec![
                        (
                            "Comfortable",
                            density == 0,
                            HitKind::Setting(Opt::Density, 0),
                        ),
                        ("Compact", density == 1, HitKind::Setting(Opt::Density, 1)),
                    ]),
                ),
                row(
                    "Status color",
                    "Amber from 70% and red from 90%. Off keeps everything monochrome",
                    Ctl::Switch(Opt::Heat, s.heat),
                ),
                row(
                    "Title bar readout",
                    "CPU, memory and GPU beside the window controls",
                    Ctl::Switch(Opt::Readout, s.readout),
                ),
            ],
        ),
        (
            "Graphs",
            vec![
                row(
                    "History",
                    "Time span across every graph",
                    choice(Opt::History, s.history),
                ),
                row(
                    "Curves",
                    "Smooth bends between samples. Linear hits every peak exactly",
                    choice(Opt::Curve, s.curve),
                ),
                row(
                    "Fill",
                    "Faint wash under each trace",
                    Ctl::Switch(Opt::Fill, s.fill),
                ),
                row(
                    "Grid",
                    "Quarter lines behind large graphs",
                    Ctl::Switch(Opt::Grid, s.grid),
                ),
            ],
        ),
        (
            "Data",
            vec![
                row(
                    "Update speed",
                    "How often every reading refreshes. Space pauses on Performance",
                    Ctl::Choice(speed),
                ),
                row(
                    "Process CPU",
                    "Per core counts one busy core as 100%. Whole machine divides by core count",
                    choice(Opt::ProcCpu, s.proc_cpu),
                ),
                row(
                    "Byte units",
                    "Bytes per kilobyte",
                    choice(Opt::Units, s.units),
                ),
                row(
                    "Temperature",
                    "GPU temperature unit",
                    choice(Opt::Temp, s.temp),
                ),
            ],
        ),
        (
            "Processes",
            vec![
                row(
                    "Columns",
                    "Name, CPU and Memory always show. Compact also hides Disk, User and Threads",
                    Ctl::Chips(
                        OPTIONAL_COLS
                            .iter()
                            .map(|c| {
                                let on = s.shows(*c);
                                (
                                    col_title(*c),
                                    on,
                                    HitKind::Setting(Opt::Column(*c), !on as u8),
                                )
                            })
                            .collect(),
                    ),
                ),
                row(
                    "Confirm ending",
                    "End task and Force kill ask for a second click",
                    Ctl::Switch(Opt::Confirm, s.confirm),
                ),
            ],
        ),
        (
            "General",
            vec![
                row(
                    "Open on",
                    "Page shown when ZIGX starts",
                    choice(Opt::OpenOn, s.open_on),
                ),
                row(
                    "Reset",
                    "Every setting, the zoom and row density back to defaults",
                    Ctl::Reset(state.reset_armed.is_some()),
                ),
            ],
        ),
    ]
}

const CHIP_GAP: f32 = 8.0;
const STEP_D: f32 = 32.0;
const STEP_VALUE_W: f32 = 64.0;

fn reset_label(armed: bool) -> &'static str {
    if armed {
        "Click again to reset"
    } else {
        "Reset all"
    }
}

fn ctl_w(ctl: &Ctl) -> f32 {
    match ctl {
        Ctl::Choice(items) => segmented_w(items),
        Ctl::Switch(..) => 32.0,
        Ctl::Chips(items) => {
            items.iter().map(|(l, ..)| pill_w(l, false)).sum::<f32>()
                + CHIP_GAP * (items.len().saturating_sub(1)) as f32
        }
        Ctl::Scale(_) => STEP_D * 2.0 + STEP_VALUE_W,
        Ctl::Reset(armed) => pill_w(reset_label(*armed), false),
    }
}

fn draw_ctl(d: &mut DrawList, ctl: &Ctl, x: f32, y: f32, mouse: [f32; 2]) {
    match ctl {
        Ctl::Choice(items) => {
            segmented(d, x, y, items, mouse);
        }
        Ctl::Switch(opt, on) => switch(
            d,
            Rect::new(x, y + 7.0, 32.0, 18.0),
            *on,
            key("switch", *opt),
        ),
        Ctl::Chips(items) => {
            let mut cx = x;
            for (label, on, kind) in items {
                let r = Rect::new(cx, y, pill_w(label, false), PILL_H);
                // Chip ids flip with their value, so key on the label.
                let h = d
                    .anim
                    .hover(key("chip-hover", label), r.contains(mouse[0], mouse[1]));
                let o = d.anim.toggle(key("chip-on", label), *on);
                let rest = mix_rgba(NONE, theme::HOVER, h);
                let line = mix_rgba(theme::GHOST_LINE, theme::ACCENT_LINE, h);
                d.slab(
                    r,
                    r.h * 0.5,
                    mix_rgba(rest, theme::ACCENT, o),
                    mix_rgba(line, theme::ACCENT, o),
                    1.0,
                );
                let ink = mix_rgba(mix_rgba(theme::INK_2, theme::INK, h), theme::ON_ACCENT, o);
                d.text_c(label, r, if o > 0.5 { PILL_ON } else { PILL }, ink);
                d.hit(r, *kind);
                cx += r.w + CHIP_GAP;
            }
        }
        Ctl::Scale(scale) => {
            let lo = crate::model::step_ui_scale(*scale, -1) < *scale - 0.001;
            let hi = crate::model::step_ui_scale(*scale, 1) > *scale + 0.001;
            for (dx, delta, enabled) in [(0.0, -1_i8, lo), (STEP_D + STEP_VALUE_W, 1, hi)] {
                let r = Rect::new(x + dx, y, STEP_D, STEP_D);
                let id = HitKind::Zoom(delta);
                ghost_pill(d, id, r, None, "", mouse, enabled, false);
                let hot = enabled && r.contains(mouse[0], mouse[1]);
                let h = d.anim.hover(key("step-hover", delta), hot);
                let en = d.anim.toggle(key("step-enabled", delta), enabled);
                let ink = mix_rgba(theme::INK_4, mix_rgba(theme::INK_2, theme::INK, h), en);
                let (cx, cy) = (r.x + r.w * 0.5, r.y + r.h * 0.5);
                bar(d, cx - 4.5, cy, cx + 4.5, cy, ICON_W, ink);
                if delta > 0 {
                    bar(d, cx, cy - 4.5, cx, cy + 4.5, ICON_W, ink);
                }
                if enabled {
                    d.hit(r, HitKind::Zoom(delta));
                }
            }
            d.text_c(
                &percent(scale * 100.0),
                Rect::new(x + STEP_D, y, STEP_VALUE_W, STEP_D),
                NUM,
                theme::INK,
            );
        }
        Ctl::Reset(armed) => {
            let label = reset_label(*armed);
            let r = Rect::new(x, y, pill_w(label, false), PILL_H);
            ghost_pill(
                d,
                HitKind::ResetSettings,
                r,
                None,
                label,
                mouse,
                true,
                *armed,
            );
            d.hit(r, HitKind::ResetSettings);
        }
    }
}

const SET_ROW_H: f32 = 60.0;
const SET_GROUP_H: f32 = 44.0;

fn settings_page(d: &mut DrawList, state: &mut AppState, main: Rect, mouse: [f32; 2]) {
    let inner = Rect::new(main.x + 28.0, main.y + 22.0, main.w - 56.0, main.h - 46.0);
    let path = crate::settings::settings_path();
    let home = std::env::var("HOME").unwrap_or_default();
    let shown = match path.to_str() {
        Some(p) if !home.is_empty() && p.starts_with(&home) => format!("~{}", &p[home.len()..]),
        Some(p) => p.to_string(),
        None => "settings.txt".into(),
    };
    let sub = format!("Changes apply at once and are saved to {shown}");
    let y = page_title(d, "Settings", &sub, inner, inner.y);
    d.hairline(Rect::new(inner.x, y - 8.0, inner.w, 1.0));
    let list = Rect::new(
        inner.x,
        y - 7.0,
        inner.w,
        (inner.bottom() - y + 7.0).max(20.0),
    );
    d.settings_rect = Some(list);

    let groups = settings_groups(state);
    let list_full = list;
    let list = Rect::new(list.x, list.y, (list.w - SCROLL_GUTTER).max(40.0), list.h);
    // Controls that would crowd the label drop below it.
    let stacked = |ctl: &Ctl| ctl_w(ctl) > list.w * 0.58;
    let row_h = |ctl: &Ctl| {
        if stacked(ctl) {
            SET_ROW_H + PILL_H + 8.0
        } else {
            SET_ROW_H
        }
    };
    let content_h: f32 = groups
        .iter()
        .map(|(_, rows)| SET_GROUP_H + rows.iter().map(|r| row_h(&r.ctl)).sum::<f32>())
        .sum::<f32>()
        + 12.0;
    let max_scroll = (content_h - list.h).max(0.0);
    if state.settings_scroll > max_scroll {
        state.settings_scroll = max_scroll;
    }
    let scroll = smooth_scroll(d, state, ScrollBar::Settings, (), state.settings_scroll);

    d.clip = Some(list_full);
    let mut cursor = list.y - scroll;
    // Entrance cascade over what is on screen; rows above it count as zero.
    let mut n = 0;
    let mut next = |d: &DrawList, top: f32| {
        let a = if top + SET_ROW_H < list.y {
            1.0
        } else {
            n += 1;
            stagger(d, n - 1)
        };
        (a, (1.0 - a) * ROW_RISE)
    };
    for (title, rows) in &groups {
        let (a, drop) = next(d, cursor);
        d.faded(a, |d| {
            eyebrow(d, list.x, cursor + 20.0 + drop, list.w, title, None)
        });
        cursor += SET_GROUP_H;
        for row in rows {
            let (a, drop) = next(d, cursor);
            let saved_fade = d.fade;
            d.fade *= a;
            let y = cursor + drop;
            let h = row_h(&row.ctl);
            let rr = Rect::new(list.x, y, list.w, h);
            let cw = ctl_w(&row.ctl);
            let stack = stacked(&row.ctl);
            let text_w = if stack {
                list.w
            } else {
                (list.w - cw - 24.0).max(40.0)
            };
            if let Ctl::Switch(opt, on) = row.ctl {
                // The whole row is the switch, like a startup entry.
                d.hit(rr, HitKind::Setting(opt, !on as u8));
            }
            d.text(
                row.label,
                Rect::new(list.x, y + 12.0, text_w, 18.0),
                BODY,
                theme::INK,
            );
            d.text(
                row.desc,
                Rect::new(list.x, y + 32.0, text_w, 16.0),
                SUB,
                theme::INK_3,
            );
            let (cx, cy) = if stack {
                (list.x, y + SET_ROW_H - 4.0)
            } else {
                (list.right() - cw, y + (SET_ROW_H - PILL_H) * 0.5)
            };
            draw_ctl(d, &row.ctl, cx, cy, mouse);
            d.hairline(Rect::new(list.x, y + h - 1.0, list.w, 1.0));
            d.fade = saved_fade;
            cursor += h;
        }
    }
    d.clip = None;
    scrollbar(
        d,
        state,
        list_full,
        content_h,
        scroll,
        ScrollBar::Settings,
        mouse,
    );
}

/// On/off switch. The track fills as the knob slides across, and the knob
/// stretches mid-travel so the flip reads as a throw rather than a jump.
fn switch(d: &mut DrawList, r: Rect, on: bool, id: Key) {
    let t = d.anim.toggle(id, on);
    d.slab(
        r,
        r.h * 0.5,
        mix_rgba(NONE, theme::ACCENT, t),
        mix_rgba(theme::GHOST_LINE, theme::ACCENT, t),
        1.0,
    );
    let kw = 12.0 + 6.0 * 4.0 * t * (1.0 - t);
    let kx = lerp(r.x + 3.0, r.right() - 3.0 - kw, t);
    d.fill(
        Rect::new(kx, r.y + 3.0, kw, 12.0),
        6.0,
        mix_rgba(theme::INK_3, theme::ON_ACCENT, t),
    );
}

#[cfg(test)]
mod tests {
    use super::{animating, build, sample_hist};
    use crate::model::{AppState, Page, Proc, Snap};

    fn proc(pid: i32, cpu: f32) -> Proc {
        Proc {
            pid,
            uid: 1000,
            user: "me".into(),
            name: format!("p{pid}"),
            cpu,
            gpu: 0.0,
            rss: 1 << 20,
            read_bps: None,
            write_bps: None,
            threads: 1,
            is_user: pid % 2 == 0,
            stopped: false,
        }
    }

    /// Paint frames at roughly display rate until nothing moves.
    fn settles(state: &mut AppState, snap: &Snap) -> bool {
        for _ in 0..120 {
            build(state, snap, &[], [0.0, 0.0]);
            if !animating(state) {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(8));
        }
        false
    }

    #[test]
    fn every_page_entrance_settles_and_goes_idle() {
        let mut snap = Snap::placeholder();
        snap.procs = (1..40).map(|p| proc(p, p as f32)).collect();
        let mut state = AppState::new(1240.0, 780.0);
        for page in [
            Page::Processes,
            Page::Performance,
            Page::Startup,
            Page::Settings,
            Page::Processes,
        ] {
            state.page = page;
            assert!(settles(&mut state, &snap), "{page:?} never settled");
        }
        // A re-sort glides rows to their new slots, then stops.
        snap.procs.reverse();
        for p in &mut snap.procs {
            p.cpu = 100.0 - p.cpu;
        }
        build(&mut state, &snap, &[], [0.0, 0.0]);
        assert!(settles(&mut state, &snap), "reorder never settled");
    }

    #[test]
    fn animations_off_never_request_frames() {
        let mut snap = Snap::placeholder();
        snap.procs = (1..10).map(|p| proc(p, 1.0)).collect();
        let mut state = AppState::new(1240.0, 780.0);
        state.settings.animations = false;
        for page in [Page::Processes, Page::Startup, Page::Settings] {
            state.page = page;
            build(&mut state, &snap, &[], [0.0, 0.0]);
            assert!(!animating(&state), "{page:?}");
        }
    }

    #[test]
    fn curve_stays_inside_its_samples() {
        let v = [0.0, 100.0, 0.0, 40.0, 45.0, 90.0, 90.0, 90.0, 10.0];
        for s in 0..=800 {
            let idx = s as f32 / 100.0;
            let i = (idx.floor() as usize).min(v.len() - 2);
            let near = &v[i.saturating_sub(1)..(i + 3).min(v.len())];
            let lo = near.iter().copied().fold(f32::MAX, f32::min) - 1e-3;
            let hi = near.iter().copied().fold(f32::MIN, f32::max) + 1e-3;
            let y = sample_hist(&v, idx);
            assert!(y >= lo && y <= hi, "idx {idx}: {y} outside {lo}..{hi}");
        }
        // A held plateau reads exactly.
        assert!((sample_hist(&v, 6.0) - 90.0).abs() < 1e-3);
    }

    #[test]
    fn drawn_segments_ignore_samples_that_have_not_played() {
        // With the right edge two samples back, a new sample must not reshape
        // anything left of it.
        let old = [10.0, 80.0, 20.0, 60.0, 30.0];
        let new = [10.0, 80.0, 20.0, 60.0, 30.0, 95.0];
        let right = (old.len() - 1) as f32 - crate::model::GRAPH_DELAY as f32 + 1.0;
        for s in 0..=((right * 100.0) as usize) {
            let idx = s as f32 / 100.0;
            assert!((sample_hist(&old, idx) - sample_hist(&new, idx)).abs() < 1e-4);
        }
    }
}
