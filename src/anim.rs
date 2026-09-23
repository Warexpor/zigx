//! Interface motion: hovers, highlights, toggles, and transitions.
//!
//! The frame is immediate mode, so animated values live here, keyed by a
//! stable id. Each frame a widget asks for its value with the target it wants;
//! the store steps a critically damped spring toward it and reports whether
//! anything is still moving, so a settled screen stops requesting frames.
//! Slots not asked for during a frame are dropped.
//!
//! With animations off every query returns its target at once.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::time::Instant;

pub type Key = u64;

/// Stable id for an animated value: a tag plus anything hashable.
pub fn key(tag: &str, id: impl Hash) -> Key {
    let mut h = DefaultHasher::new();
    tag.hash(&mut h);
    id.hash(&mut h);
    h.finish()
}

/// Smooth times, seconds. A critically damped spring is about 90% of the way
/// at twice its smooth time and visually settled at three times.
pub const HOVER_IN: f32 = 0.035;
pub const HOVER_OUT: f32 = 0.08;
/// Switch knobs, segment pills, color changes on a state flip.
pub const TOGGLE: f32 = 0.065;
/// Leading and trailing edges of a gliding highlight. The lead runs ahead so
/// the highlight stretches toward its target and then gathers behind it.
pub const GLIDE_LEAD: f32 = 0.05;
pub const GLIDE_TRAIL: f32 = 0.085;
/// Launch intro and new rows.
pub const ENTER: f32 = 0.085;
/// Page and section switches: a short fade in place, no travel.
pub const PAGE: f32 = 0.05;
/// Wheel and keyboard scrolling.
pub const SCROLL: f32 = 0.055;
/// Process rows moving to a new slot after a re-sort or filter.
pub const REORDER: f32 = 0.075;
pub const MENU_IN: f32 = 0.05;
pub const MENU_OUT: f32 = 0.04;
pub const TOAST: f32 = 0.07;
/// Interface zoom.
pub const ZOOM: f32 = 0.07;

/// Settle thresholds: unit values (alpha, progress) and design pixels. Text
/// lands on whole pixels, so a fifth of one is below anything visible.
const EPS_UNIT: f32 = 0.002;
const EPS_PX: f32 = 0.2;

/// Critically damped follower: never overshoots, and keeps its velocity when
/// the target moves mid-flight.
#[derive(Clone, Copy, Debug)]
pub struct Spring {
    pub value: f32,
    pub vel: f32,
}

impl Spring {
    pub fn new(value: f32) -> Self {
        Self { value, vel: 0.0 }
    }

    /// Advance by `dt` toward `target`. Returns true once settled on it.
    pub fn step(&mut self, target: f32, smooth_time: f32, dt: f32, eps: f32) -> bool {
        let omega = 2.0 / smooth_time.max(1e-4);
        let x = omega * dt;
        let decay = 1.0 / (1.0 + x + 0.48 * x * x + 0.235 * x * x * x);
        let change = self.value - target;
        let temp = (self.vel + omega * change) * dt;
        self.vel = (self.vel - omega * temp) * decay;
        let out = target + (change + temp) * decay;
        // Never cross the target: land on it instead.
        if (target > self.value) == (out > target) && out != target {
            self.value = target;
            self.vel = 0.0;
        } else {
            self.value = out;
        }
        self.settle(target, eps)
    }

    fn settle(&mut self, target: f32, eps: f32) -> bool {
        if (self.value - target).abs() < eps && (self.vel * 0.05).abs() < eps {
            self.value = target;
            self.vel = 0.0;
            true
        } else {
            false
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Slot {
    spring: Spring,
    frame: u32,
}

#[derive(Debug, Default)]
pub struct Anim {
    slots: HashMap<Key, Slot>,
    at: Option<Instant>,
    dt: f32,
    frame: u32,
    on: bool,
    /// The layout jumped (window resize or zoom): positions land at once.
    snap: bool,
    size: Option<(f32, f32)>,
    busy: bool,
}

impl Anim {
    /// Start a frame. `size` is the layout size, to detect resizes.
    pub fn begin(&mut self, on: bool, size: (f32, f32)) {
        let now = Instant::now();
        let raw = self
            .at
            .map_or(0.0, |t| now.saturating_duration_since(t).as_secs_f32());
        // After an idle stretch nothing was moving, so the first step of a new
        // animation is one display frame, not the whole gap.
        self.dt = if self.busy {
            raw.min(0.05)
        } else {
            raw.min(1.0 / 60.0)
        };
        self.at = Some(now);
        self.on = on;
        self.snap = self.size.is_some_and(|s| s != size);
        self.size = Some(size);
        self.busy = false;
        self.frame = self.frame.wrapping_add(1);
    }

    /// Finish a frame: forget every value nothing asked for.
    pub fn end(&mut self) {
        let f = self.frame;
        self.slots.retain(|_, s| s.frame == f);
    }

    /// Something is mid-flight and needs another frame.
    pub fn busy(&self) -> bool {
        self.busy
    }

    pub fn enabled(&self) -> bool {
        self.on
    }

    fn run(&mut self, k: Key, from: f32, target: f32, time: f32, eps: f32, snap: bool) -> f32 {
        let frame = self.frame;
        let (on, dt) = (self.on && !snap, self.dt);
        let slot = self.slots.entry(k).or_insert(Slot {
            spring: Spring::new(from),
            frame,
        });
        slot.frame = frame;
        if !on {
            slot.spring = Spring::new(target);
            return target;
        }
        if !slot.spring.step(target, time, dt, eps) {
            self.busy = true;
        }
        slot.spring.value
    }

    /// A 0..1 value (alpha, progress). New slots start at the target.
    pub fn mix(&mut self, k: Key, target: f32, time: f32) -> f32 {
        self.run(k, target, target, time, EPS_UNIT, false)
    }

    /// Like [`mix`](Self::mix), but a new slot starts at `from`: entrances
    /// and exits.
    pub fn mix_from(&mut self, k: Key, from: f32, target: f32, time: f32) -> f32 {
        self.run(k, from, target, time, EPS_UNIT, false)
    }

    /// Hover emphasis: quick to light, slower to let go.
    pub fn hover(&mut self, k: Key, hot: bool) -> f32 {
        let t = if hot { HOVER_IN } else { HOVER_OUT };
        self.mix(k, hot as u8 as f32, t)
    }

    /// Two-state flip, 0 or 1.
    pub fn toggle(&mut self, k: Key, on: bool) -> f32 {
        self.mix(k, on as u8 as f32, TOGGLE)
    }

    /// A position or size in design pixels. Lands at once when the window
    /// is resized, so layout never trails the window edge.
    pub fn slide(&mut self, k: Key, target: f32, time: f32) -> f32 {
        let snap = self.snap;
        self.run(k, target, target, time, EPS_PX, snap)
    }

    /// Jump a value, e.g. a scroll position under a dragged thumb.
    pub fn set(&mut self, k: Key, v: f32) {
        let frame = self.frame;
        self.slots.insert(
            k,
            Slot {
                spring: Spring::new(v),
                frame,
            },
        );
    }

    /// Value from the previous frame, if the slot exists.
    pub fn peek(&self, k: Key) -> Option<f32> {
        self.slots.get(&k).map(|s| s.spring.value)
    }

    /// A span (`lo..hi`) that glides to a new place: the edge in the
    /// direction of travel leads, the other trails, so it stretches and
    /// gathers instead of translating rigidly.
    pub fn glide(&mut self, k: Key, lo: f32, hi: f32) -> (f32, f32) {
        let (klo, khi) = (key("lo", k), key("hi", k));
        let cur = self.peek(klo).unwrap_or(lo);
        let (tlo, thi) = if lo > cur {
            (GLIDE_TRAIL, GLIDE_LEAD)
        } else {
            (GLIDE_LEAD, GLIDE_TRAIL)
        };
        (self.slide(klo, lo, tlo), self.slide(khi, hi, thi))
    }

    /// Put a glide on its target without travel, e.g. when it fades in.
    pub fn place(&mut self, k: Key, lo: f32, hi: f32) {
        self.set(key("lo", k), lo);
        self.set(key("hi", k), hi);
    }
}

pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Blend two colors in premultiplied space, so fading to or from a
/// transparent color never darkens the ink on the way.
pub fn mix_rgba(a: [u8; 4], b: [u8; 4], t: f32) -> [u8; 4] {
    let t = t.clamp(0.0, 1.0);
    if t <= 0.0 {
        return a;
    }
    if t >= 1.0 {
        return b;
    }
    let (aa, ba) = (a[3] as f32 / 255.0, b[3] as f32 / 255.0);
    let alpha = lerp(aa, ba, t);
    if alpha <= 1e-4 {
        return [0, 0, 0, 0];
    }
    let mut out = [0u8; 4];
    for i in 0..3 {
        let c = lerp(a[i] as f32 * aa, b[i] as f32 * ba, t) / alpha;
        out[i] = c.round().clamp(0.0, 255.0) as u8;
    }
    out[3] = (alpha * 255.0).round() as u8;
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_frames(a: &mut Anim, k: Key, target: f32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|_| {
                a.begin(true, (100.0, 100.0));
                a.dt = 1.0 / 60.0;
                let v = a.mix_from(k, 0.0, target, TOGGLE);
                a.end();
                v
            })
            .collect()
    }

    #[test]
    fn springs_settle_without_overshoot() {
        let mut a = Anim::default();
        let vals = run_frames(&mut a, key("t", 0), 1.0, 60);
        assert!(vals.windows(2).all(|w| w[1] >= w[0]), "monotonic");
        assert!(vals.iter().all(|v| *v <= 1.0));
        assert_eq!(*vals.last().unwrap(), 1.0);
        assert!(!a.busy(), "settled springs stop requesting frames");
    }

    #[test]
    fn off_returns_targets_at_once() {
        let mut a = Anim::default();
        a.begin(false, (100.0, 100.0));
        assert_eq!(a.mix_from(key("t", 1), 0.0, 1.0, ENTER), 1.0);
        assert_eq!(a.slide(key("t", 2), 40.0, SCROLL), 40.0);
        assert!(!a.busy());
    }

    #[test]
    fn untouched_slots_are_dropped() {
        let mut a = Anim::default();
        a.begin(true, (100.0, 100.0));
        a.mix(key("t", 3), 1.0, TOGGLE);
        a.end();
        a.begin(true, (100.0, 100.0));
        a.end();
        assert!(a.peek(key("t", 3)).is_none());
    }

    #[test]
    fn resize_lands_positions_but_not_fades() {
        let mut a = Anim::default();
        a.begin(true, (100.0, 100.0));
        a.slide(key("p", 0), 0.0, GLIDE_LEAD);
        a.mix(key("f", 0), 0.0, TOGGLE);
        a.end();
        a.begin(true, (120.0, 100.0));
        assert_eq!(a.slide(key("p", 0), 50.0, GLIDE_LEAD), 50.0);
        assert!(a.mix(key("f", 0), 1.0, TOGGLE) < 1.0);
    }

    #[test]
    fn color_mix_keeps_ink_through_transparent() {
        let c = mix_rgba([0, 0, 0, 0], [240, 240, 250, 200], 0.5);
        assert_eq!(&c[..3], &[240, 240, 250]);
        assert_eq!(c[3], 100);
    }
}
