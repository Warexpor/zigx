# ZIGX

A fast Linux system monitor. Processes, a per-core CPU grid, memory, GPU, disk, network, and startup apps — spectral white ink on black glass.

The window is transparent. On Hyprland and Omarchy the blur behind it is the glass. ZIGX does not paint a fake frost and it does not use a widget toolkit.

The visual system is monochrome and flat: one ink stepped by alpha, hairlines instead of shadows, tracked uppercase labels for structure, a monospace face for every number, and a single filled white pill for the active choice. Chroma appears only as status, amber at 70 percent load and red at 90.

```bash
cargo run --release
```

Design system, layout, components, and motion are documented in [docs/design](docs/design/README.md).

## Install

```bash
cargo install --path .
install -Dm644 share/zigx.desktop ~/.local/share/applications/zigx.desktop
```

The desktop entry expects `zigx` on `PATH`; `~/.cargo/bin` is the default install location.

## Using it

Sampling runs off the UI thread and the window redraws when numbers change, not on a busy loop. The nav footer shows how long the last sample took.

Keys, when the search box is not focused:

- `1` `2` `3` switch pages.
- On Performance, `c` `m` `g` `d` `n` select CPU, memory, GPU, disk, and network.
- Typing starts a search. `Ctrl+F` focuses it, `Esc` clears it, `Ctrl+Backspace` empties it.
- Click selects a process; `Ctrl` and `Shift` extend the selection.
- `Delete` arms End task and a second press (or `Enter`) sends SIGTERM.
- `PageUp` `PageDown` scroll the current page.

### Zoom

`Ctrl++` / `Ctrl+=` zooms in and `Ctrl+-` zooms out. Stops run from 80% to 180% (80, 90, 100, 110, 125, 140, 160, 180). `Ctrl+0` resets to 100%. The scale is saved in `~/.config/zigx/ui.txt` with the other UI prefs.

Startup lists every autostart entry the session sees: `/etc/xdg/autostart` (and `$XDG_CONFIG_DIRS`) merged with `~/.config/autostart`, where a user file overrides the system one of the same name. Turning an entry off writes `Hidden=true` into the user file, creating it from the system entry when needed, and keeps a one-time `.bak`. Undo lasts 10 seconds.

## Fonts

ZIGX picks installed fonts in this order: Inter, Adwaita Sans, Cantarell, Noto Sans, Liberation Sans, DejaVu Sans for text, and JetBrains Mono (Nerd Font), Adwaita Mono, Cascadia Mono, Fira Code, Noto Sans Mono, Liberation Mono, DejaVu Sans Mono for numbers. Override with `ZIGX_SANS` and `ZIGX_MONO` set to an installed family name.

## Compositor

The Wayland app id is `zigx`. If the desktop blur does not pick the window up:

```
windowrule = blur on, match:class zigx
```

## License

Apache License 2.0. See [LICENSE](LICENSE).
