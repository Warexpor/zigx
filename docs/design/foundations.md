# Foundations

## Color

All tokens live in the `theme` module of `src/model.rs`. Neutral tokens are the
spectral white `rgb(240, 240, 250)` at different alphas; percentages below are
alpha out of 255.

### Ink

| Token | Alpha | Use |
| --- | --- | --- |
| `INK` | 255 (100%) | Primary text, active labels, readouts, process names |
| `INK_2` | 170 (67%) | Secondary text, inactive nav, cell values, bar fill |
| `INK_3` | 118 (46%) | Captions, micro labels, idle glyphs, placeholders |
| `INK_4` | 80 (31%) | Quietest text: scale notes, version, idle cells, chevrons |

### Surfaces and structure

| Token | Value | Use |
| --- | --- | --- |
| `CANVAS` | black at 200 (78%) | The window sheet. The desktop blur shows through it. The Glass setting swaps the alpha: 140 clear, 200 glass, 255 solid |
| `CANVAS_LINE` | ink at 36 (14%) | Window edge |
| `HAIRLINE` | ink at 30 (12%) | Dividers, graph baselines, disabled pill outline |
| `GRID` | ink at 14 (5%) | Graph quarter grid, VRAM track |
| `GHOST` | ink at 22 (9%) | Active side item, focused search field |
| `GHOST_LINE` | ink at 66 (26%) | Pill, segmented control, and off-switch outlines |
| `HOVER` | ink at 16 (6%) | Hover fill for process rows, side items, window controls (not rows with a switch) |
| `SELECTED` | ink at 30 (12%) | Selected process rows |
| `TOAST` | black at 230 (90%) | Notice toast |
| `MENU` | black at 250 (98%) | Context menu, which sits over dense rows |

### Accent

| Token | Value | Use |
| --- | --- | --- |
| `ACCENT` | ink at 26 (10%) | The one filled element: active segment, on chip, on switch |
| `ACCENT_EDGE` | ink at 195 (76%) | Half-pixel edge of the `ACCENT` pill |
| `ACCENT_HALO` | ink at 14 (5%) | Soft ring 1 px outside the `ACCENT` pill's edge |
| `ON_ACCENT` | `INK` | Text and knob on `ACCENT` |
| `ACCENT_LINE` | ink at 140 (55%) | Hovered pill and focused search outline |

### Status

| Token | Value | Use |
| --- | --- | --- |
| `WARN` | `#F5A623` | Load at or above 70% |
| `HOT` | `#FF5C5C` | Load at or above 90% |
| `DANGER_LINE` | `#FF5C5C` at 150 (59%) | Armed End task outline, armed Force kill item |
| `DANGER_INK` | `#FF8A8A` | Armed End task label, Force kill item, hovered close glyph |

Load coloring goes through `heat()` in `src/frame.rs`: below 70 is the normal
ink for that element, 70 is `WARN`, 90 is `HOT`. Idle process cells (CPU and
GPU under 0.05%, disk under 1 B/s) drop to `INK_4` so active rows read first.
With Status color off in Settings, `heat()` always returns the normal ink and
the app is fully monochrome apart from danger states and the pause flag.
The title bar "PAUSED" flag is `WARN`: frozen numbers must never pass for live
ones.

### Traces

| Token | Value | Use |
| --- | --- | --- |
| `TRACE` | ink at 235 (92%) | Primary series: utilization, read, receive |
| `TRACE_2` | ink at 110 (43%) | Secondary series: swap, write, transmit |

Each trace carries a wash beneath it at 7% of its alpha, capped at 18, which is
about 6% for `TRACE`. See [Motion and graphics](motion.md#rendering).

## Type

Two families, one weight. Hierarchy comes from size, alpha, and case. Micro
labels are uppercase and tracked like panel engraving; every number is
monospace.

| Role | Size | Family | Weight | Tracking | Case | Use |
| --- | --- | --- | --- | --- | --- | --- |
| `WORDMARK` | 11.5 | Sans | 500 | 0.18 | Upper | "ZIGX" in the title bar |
| `MICRO` | 10 | Sans | 400 | 0.10 | Upper | Eyebrows, column headers, captions |
| `MICRO_NUM` | 10 | Mono | 400 | 0.06 | Upper | Title bar readout, eyebrow details, scale notes |
| `NAV` | 13 | Sans | 400 | 0 | As is | Side items |
| `BODY` | 12.5 | Sans | 400 | 0 | As is | Process names, startup names, toast |
| `PILL` | 12 | Sans | 400 | 0 | As is | Pill and segment labels |
| `PILL_ON` | 12 | Sans | 500 | 0 | As is | Label on `ACCENT` |
| `NUM` | 12 | Mono | 400 | 0 | As is | Table numbers, search query, device names |
| `NUM_SMALL` | 10.5 | Mono | 400 | 0 | As is | Exec lines, per-device rates |
| `TITLE` | 22 | Sans | 400 | -0.012 | As is | Page title |
| `SUB` | 12 | Sans | 400 | 0 | As is | Line under a page title |
| `READOUT` | 44 | Mono | 400 | -0.02 | As is | Headline metric |
| `READOUT_SUB` | 16 | Mono | 400 | 0 | As is | "/ total" after a readout |
| `STAT` | 15 | Mono | 400 | 0 | As is | Stat row values |

Sizes are in design pixels (see [Layout](layout.md#scaling)). Tracking is in
em.

The only weight change is 500 on the wordmark and on text sitting on the
accent pill, where it carries the active state along with the fill.

### Font stack

Sans: Inter, Adwaita Sans, Cantarell, Noto Sans, Liberation Sans, DejaVu Sans.

Mono: JetBrains Mono (Nerd Font), Adwaita Mono, Cascadia Mono, Fira Code, Noto
Sans Mono, Liberation Mono, DejaVu Sans Mono.

`ZIGX_SANS` and `ZIGX_MONO` override the choice with an installed family name.

### Number formatting

Helpers are in `src/format.rs`.

- System percentages (`percent`): whole numbers, "37%".
- Per-process CPU and GPU (`cpu_pct`): one decimal under 10%, whole above, so
  the many near-idle rows still sort visibly.
- Bytes: binary steps shown as KB, MB, GB, TB. The Byte units setting switches
  to decimal steps of 1000 with the same labels.
- Temperature: "62 °C", or whole degrees Fahrenheit when set.
- Missing values: an em dash.

## Shape

- Pills, the search field, segments, and switches are fully rounded: radius is
  half the height.
- Rows and side items: radius 6.
- Window sheet: radius 10.
- Borders are 1 px and straddle the edge. The shape shader antialiases fills
  across about 1.5 px and borders across about 1 px, so edges stay soft at any
  zoom.
- No drop shadows in the UI. The shape shader supports one, but the design
  does not use it.

## Iconography

Icons are 14 x 14 line drawings built from strokes, bars, rings, and dots in
`icon()` in `src/frame.rs`. They are geometry, not font glyphs.

- **Hairline weight** (`ICON_W`, 1.35): rules, chevrons, pins, window controls.
- **Glyph weight** (`GLYPH_W`, 2.4): the bodies of page and resource icons, so
  the nav set reads as one family.
- **Ring weight** (`RING_W`, 1.0 border): outlined bodies that land at glyph
  weight on screen.

| Icon | Drawing |
| --- | --- |
| Processes | Three records, each a dot and a rule of uneven length |
| Performance | A trend line with one dip and a climb, over a baseline |
| Startup | Power: an open ring with the switch bar in its gap |
| Settings | Cog: one closed outline whose radius follows a softened square wave (six teeth with flat tops and rounded shoulders), around an open hub ring, both at hairline weight (1.35) |
| CPU | Square die with two pins per side |
| Memory | Long module over three contacts |
| GPU | Card with a fan disc and a front bracket |
| Disk | Platter ring with a spindle dot |
| Network | Down arrow left, up arrow right |
| Search | Lens ring and handle |
| Density | Three open rules (comfortable) or four tight rules (compact) |

Each silhouette must stay distinct from the others at 14 px. Glyphs and labels
in a side item share one ink, so no item looks disabled.
