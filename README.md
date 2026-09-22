# ZIGX

A fast Linux system monitor. Processes, a per-core CPU grid, memory, GPU, disk, network, and startup apps — drawn as silver ink on compositor blur.

The window is transparent. On Hyprland and Omarchy the blur behind it is the glass. ZIGX does not paint a fake frost and it does not use a widget toolkit.

```bash
cargo run --release
```

Sampling runs off the UI thread and the window redraws when numbers change, not on a busy loop. The nav footer shows how long the last sample took.

Keys, when the search box is not focused: `1` `2` `3` switch pages. On Performance, `c` `m` `g` `d` `n` select CPU, memory, GPU, disk, and network. Typing starts a search, `Esc` clears it, `Delete` arms End task and a second press sends SIGTERM. Startup toggles write `Hidden=` in `~/.config/autostart` and keep a `.bak`. Undo lasts 10 seconds.

The Wayland app id is `zigx`. If the desktop blur does not pick the window up:

```
windowrule = blur on, match:class zigx
```
