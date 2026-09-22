//! ZIGX — a Linux system monitor drawn as ink on compositor blur.
//!
//! Sampling talks to `/proc` and NVML on a side thread. The window is a
//! transparent GPU surface; Hyprland (or any compositor with blur) supplies
//! the frost. Nothing here is a toolkit port.

mod anim;
mod format;
mod frame;
mod interact;
mod model;
mod persist;
mod sample;
mod settings;
mod startup;

pub use anim::{Spring, ZOOM};
pub use frame::{
    animating, build, hit_at, wake_at, DrawList, Hit, HitKind, Label, Layer, Rect, Slab, Stroke,
};
pub use interact::{
    clear_selection, close_menu, end_hold, expire, note_drag_origin, on_key, on_move, on_press,
    on_release, on_wheel, open_menu, send_signal, Effect, KeyIn, Sig,
};
pub use model::*;
pub use persist::{load_ui, save_ui};
pub use sample::{spawn, Hub};
pub use settings::{load_settings, Settings};
pub use startup::{load_startup, write_enabled};
