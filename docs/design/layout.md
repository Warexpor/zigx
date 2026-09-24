# Layout

All measurements are design pixels. `build()` in `src/frame.rs` lays out one
frame from scratch every paint; there is no retained widget tree.

## Window shell

```
+------------------------------------------------------------------+
| ZIGX                               CPU 12%  MEM 41%  GPU 3%  - x |  title bar, 48
+-----------+------------------------------------------------------+
| MONITOR   |                                                      |
| Processes |                     page content                     |
| Perform.  |                                                      |
| Startup   |                                                      |
|           |                                                      |
| Settings  |                                                      |
| v0.5.3    |                                                      |
+-----------+------------------------------------------------------+
   nav 200
```

- **Sheet.** One black glass slab covers the window, radius 10, with a
  `CANVAS_LINE` edge.
- **Title bar.** 48 tall and draggable everywhere without a control. It holds
  the wordmark 20 from the left, the live readout in `MICRO_NUM` / `INK_3`
  ending 24 before the window controls, and two 28 x 28 controls on a 32 pitch,
  12 from the right. Controls are bare glyphs; hover adds a round `HOVER` disc,
  and hovering close turns its glyph `DANGER_INK`.
- **Nav.** 200 wide by default, resizable by dragging its right hairline from
  160 to 300. It shows a "Monitor" eyebrow at 18, then side items from 44 on a
  38 pitch, with the version pinned 30 above the bottom. Settings is about the
  app rather than the machine, so it is pinned to the foot, 82 above the
  bottom, apart from the monitor pages.
- **Readout.** Hidden when Title bar readout is off. While sampling is paused
  a `WARN` "PAUSED" flag sits ahead of it (or in its place).
- **Dividers.** Hairlines under the title bar, right of the nav, and right of
  the Performance sub-nav. Resize handles are 5 px hit strips centered on
  their hairlines.
- **Minimum window.** 420 x 320. The first window opens at 70% of the primary
  monitor, keeping its aspect ratio.

## Processes

Content is inset 24 horizontally, 18 at the top, and 24 at the bottom.

1. **Toolbar**, one 32 high line with two clusters.
   - Left narrows the list: the view segmented control (Grouped, Flat, User,
     System), then search.
   - Right acts on it: the density pill, then End task at the trailing edge.
   - Search fills the space between the clusters, keeping a 10 gap on both
     sides and never narrower than 96.
2. **Column header** 20 below the toolbar: `MICRO` titles with a hairline 24
   under their top. The sorted column is `INK` with a small chevron. The header
   paints after the rows so scrolled content never covers it.
3. **List.** Rows are 32 comfortable or 26 compact. Grouped view clusters
   processes that share a program name under a collapsible header with a
   chevron and a count; groups start collapsed, and single-instance programs
   stay as ordinary rows. Sorting by Name floats those group headers above
   one-offs. User and System views filter by ownership without headers.

Columns are right-aligned mono numbers, filled from the right edge. Name takes
the rest. GPU, Disk, PID, User and Threads can be hidden in Settings; hidden
columns drop out of the table below and Name takes their width.

| Density | Columns, left to right after Name |
| --- | --- |
| Comfortable | CPU 72, GPU 68, Memory 88, Disk 96, PID 72, User 96, Threads 76 |
| Compact | CPU 68, GPU 64, Memory 88, PID 68 |

Selected rows stay pinned at their index while the list re-sorts under them.
Clicking empty list space clears the selection and unpins them.

Holding `Space` freezes every row in the arrangement currently on screen. The
name column reads `HELD`. Numbers keep updating. Releasing `Space`, or the
window losing focus, sorts again. A key repeat does not recapture.

## Performance

```
| nav | RESOURCES  |  Processor                                  |
|     | CPU        |  AMD Ryzen ...                              |
|     | Memory     |  37%                                        |
|     | GPU        |  TOTAL UTILIZATION                          |
|     | Disk       |  4.2 GHz   412   5,120   3d 2h   1.20       |
|     | Network    |  UTILIZATION  30 S WINDOW             100%  |
|     |            |  [graph]                                    |
|     |            |  CORES  16 LOGICAL  PEAK 88%                |
|     |            |  [per-core bars]                            |
```

- **Sub-nav.** 196 wide by default, resizable from 140 to 260, and never more
  than 40% of the main area. It lists the five resources by name only; live
  values live on the detail sheet.
- **Detail sheet.** Inset 28 horizontally, 22 at the top, 24 at the bottom,
  and scrollable when content overflows.
- **Page anatomy**, top to bottom:
  - Title (`TITLE`) with a subtitle 30 below it.
  - Headline readout (`READOUT`, 52 tall) with a `MICRO` caption at 56.
  - Stat row: up to five equal slots, each a `STAT` value over a `MICRO`
    label.
  - Sections, each an eyebrow (`MICRO` label and `MICRO_NUM` detail) followed
    by a graph, bars, or a meter.
- **Elastic graphs.** A graph takes spare height, clamped to 110 to 320. When
  a page has a secondary block (per-core bars, swap), the graph takes 62 to 68%
  of the spare height and the block takes the rest.
- **I/O pages.** A totals stat row, then one block per device: name, then R/W
  or RX/TX rates, then a two-series graph. The legend sits at the top left and
  the scale note at the top right. Spare height is split evenly between devices.
- **GPU page.** One section per GPU. With several GPUs, each utilization graph
  is a fixed 120. VRAM is a 2 px meter: an `INK` fill on a `GRID` track.

## Startup

Same insets as Processes. A title with an explanatory subtitle, then an
"Entries" eyebrow with an on-count, a hairline, and a list of 52 tall rows:
name (`BODY`), an optional "system" tag, the Exec line (`NUM_SMALL`), and a
switch at the right edge. Disabled entries drop to `INK_3` and `INK_4`. Like
Settings, the rows stop 16 short of the right edge (`SCROLL_GUTTER`) so the
scrollbar never sits on a switch.

## Settings

Same insets as Startup. A title whose subtitle names the settings file, a
hairline, then a scrolling list of groups. Each group is a 44 high eyebrow
followed by setting rows (see [Components](components.md#setting-row)). A 16
gutter on the right (`SCROLL_GUTTER`) keeps the scrollbar clear of the controls.

| Group | Rows |
| --- | --- |
| Appearance | Interface scale, Glass, Animations, Graph motion, Row density, Status color, Title bar readout |
| Graphs | History, Curves, Fill, Grid |
| Data | Update speed (with Pause), Process CPU, Byte units, Temperature |
| Processes | Columns, List animations, Confirm ending |
| General | Open on, Reset |

Every change applies on the next frame and is saved at once. Typing on this
page does not start a process search.

## Scaling

Layout is in design pixels. The framebuffer scale is the monitor DPI times the
user zoom.

- Zoom stops: 80, 90, 100, 110, 125, 140, 160, 180%. `Ctrl++` and `Ctrl+-`
  step between them and `Ctrl+0` resets.
- Zoom, pane widths, page, Performance section, process view, density, and sort
  persist in `~/.config/zigx/ui.txt`: state the app remembers on its own.
- Preferences set on the Settings page persist in
  `~/.config/zigx/settings.txt`, a commented `key=value` file that is safe to
  edit by hand. Unknown keys and values fall back to defaults. Pause is never
  saved, so ZIGX always launches live.
- Keep geometry on whole or half design pixels where possible. Hairlines are 1
  design pixel and are antialiased at fractional scales rather than snapped.

## Scrollbars

A slim 2 px thumb at the right edge of the scrolling area, `INK_4` at rest and
`INK_2` on hover or drag, with a minimum height of 24. The hit strip is 12 wide
so the thumb is easy to grab. There is no track, and nothing is drawn when the
content fits.
