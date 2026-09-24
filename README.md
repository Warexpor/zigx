# ZIGX

A fast Linux system monitor. Processes, a per-core CPU grid, memory, GPU, disk, network, and startup apps — spectral white ink on black glass.

![ZIGX Performance page showing GPU utilization over a blurred desktop](docs/media/showcase.jpg)

The window is transparent. On Hyprland and Omarchy the blur behind it is the glass. ZIGX does not paint a fake frost and it does not use a widget toolkit.

The visual system is monochrome and flat: one ink stepped by alpha, hairlines instead of shadows, tracked uppercase labels for structure, a monospace face for every number, and a single smoked pill, a faint wash with a fine bright edge, for the active choice. Chroma appears only as status, amber at 70 percent load and red at 90.

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

- `1` `2` `3` `4` switch pages (Processes, Performance, Startup, Settings). `Ctrl+,` opens Settings.
- On Performance, `c` `m` `g` `d` `n` select CPU, memory, GPU, disk, and network, and `Space` pauses or resumes sampling.
- Hold `Space` on Processes to freeze the list in the order on screen. Counts keep updating in place; letting go sorts again. Space still types into search while that field is focused.
- Search takes keys only while its field is focused. Click it, or press `Ctrl+F`. `Esc` clears it, `Ctrl+Backspace` empties it.
- Click selects a process; `Ctrl` and `Shift` extend the selection.
- Right-click a process for its menu: End task, Force kill (click twice), Suspend or Resume, Open file location, Copy command line, and Copy PID. It acts on the whole selection when the row is part of it. Arrow keys and `Enter` drive it, `Esc` closes it. Copying uses `wl-copy` (or `xclip` / `xsel`).
- `Delete` arms End task and a second press (or `Enter`) sends SIGTERM. Turn off Confirm ending in Settings to skip the second press.
- `PageUp` `PageDown` scroll the current page.

### Settings

The Settings page, at the foot of the nav, applies every change at once and saves it to `~/.config/zigx/settings.txt`, which is also safe to edit by hand.

- **Appearance:** interface scale, glass (clear, glass, solid), animations (on or off: fades, glides and transitions across the interface), graph motion (smooth or reduced), row density, status color, title bar readout.
- **Graphs:** history (30 s, 1 min, 2 min), curves (smooth or linear), fill under traces, grid.
- **Data:** update speed (0.5, 1 or 2 s, or Pause), process CPU as a share of one core or of the whole machine, byte units (1024 or 1000), temperature (°C or °F).
- **Processes:** which optional columns show (GPU, Disk, PID, User, Threads), and whether ending a task asks for a second click.
- **General:** the page ZIGX opens on, and Reset, which takes two clicks.

Pause lasts for the session only; ZIGX always launches live.

### Zoom

`Ctrl++` / `Ctrl+=` zooms in and `Ctrl+-` zooms out. Stops run from 80% to 180% (80, 90, 100, 110, 125, 140, 160, 180). `Ctrl+0` resets to 100%. Settings has the same control. The scale is saved in `~/.config/zigx/ui.txt` with the other remembered layout state.

Startup lists every `.desktop` in `/etc/xdg/autostart` (and `$XDG_CONFIG_DIRS`) merged with `~/.config/autostart`, where a user file overrides the system one of the same name. `OnlyShowIn` / `NotShowIn` / `TryExec` do not hide entries from the list. Turning an entry off writes `Hidden=true` into the user file, creating it from the system entry when needed, and keeps a one-time `.bak`.

## Fonts

ZIGX picks installed fonts in this order: Inter, Adwaita Sans, Cantarell, Noto Sans, Liberation Sans, DejaVu Sans for text, and JetBrains Mono (Nerd Font), Adwaita Mono, Cascadia Mono, Fira Code, Noto Sans Mono, Liberation Mono, DejaVu Sans Mono for numbers. Override with `ZIGX_SANS` and `ZIGX_MONO` set to an installed family name.

## Compositor

The Wayland app id is `zigx`. If the desktop blur does not pick the window up:

```
windowrule = blur on, match:class zigx
```

## License

Apache License 2.0. See [LICENSE](LICENSE).
