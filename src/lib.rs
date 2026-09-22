//! ZIGX — a Linux system monitor drawn as ink on compositor blur.
//!
//! Sampling talks to `/proc` and NVML on a side thread. The window is a
//! transparent GPU surface; Hyprland (or any compositor with blur) supplies
//! the frost. Nothing here is a toolkit port.

mod format;
mod frame;
mod interact;
mod model;
mod persist;
mod sample;
mod startup;

pub use frame::{build, hit_at, DrawList, Hit, HitKind, Rect};
pub use interact::{
    expire, note_drag_origin, on_key, on_move, on_press, on_release, on_wheel, terminate, Effect,
    KeyIn,
};
pub use model::*;
pub use persist::{load_ui, save_ui};
pub use sample::{spawn, Hub};
pub use startup::{load_startup, restore_startup, write_enabled};
