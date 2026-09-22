# Components

Every component is a function in `src/frame.rs` that emits slabs, strokes, and
labels. The shared rhythm is a 32 high control, 16 horizontal padding, and a 10
gap between controls.

## Ghost pill

`ghost_pill()`. The only button style: a transparent, fully rounded shape with
a 1 px outline.

| State | Outline | Label | Fill |
| --- | --- | --- | --- |
| Rest | `GHOST_LINE` | `INK_2` (glyph `INK_3`) | None |
| Hover | `ACCENT_LINE` | `INK` | `HOVER` |
| Disabled | `HAIRLINE` | `INK_4` | None |
| Danger (armed) | `DANGER_LINE` | `DANGER_INK` | None |

- Text-only pills pad 16 on each side.
- With a leading glyph, the glyph sits 12 in and the label starts at 34.
- Width comes from the measured label, rounded up so the last glyph is never
  clipped.

## Segmented control

A single `GHOST_LINE` outline 32 high. Segments sit 3 inside it with 16 side
padding each, so short and long labels share one rhythm. The active segment is
the `ACCENT` pill with a `PILL_ON` label in `ON_ACCENT`. Inactive labels are
`INK_2`, or `INK` on hover.

## Search field

A pill-shaped field with a search glyph at 12 and text from 34.

| State | Outline | Fill | Text |
| --- | --- | --- | --- |
| Rest | `GHOST_LINE` | None | "Search" placeholder, `PILL`, `INK_3` |
| Hover | `ACCENT_LINE` | None | Placeholder `INK_2` |
| Focused | `ACCENT_LINE` | `GHOST` | Query in `NUM`, `INK`, with a caret |

Typing anywhere on the Processes page starts a search. `Ctrl+F` focuses it,
`Esc` clears it, and `Ctrl+Backspace` empties it.

## End task

A ghost pill whose label follows the selection; the context menu offers the
same action without the confirm step. With nothing selected it is
disabled and reads "End task". The first press, or `Delete`, arms it for 4
seconds: it switches to the danger style and reads "Confirm N". A second press,
or `Enter`, sends SIGTERM. Changing the selection disarms it.

## Side item

`side_item()`. 34 high, radius 6, used by the nav and the Performance sub-nav.
The glyph sits at 12 and the label at 36. An optional value sits right-aligned
in `MICRO_NUM`.

- Active: `GHOST` fill, `INK` glyph and label, value in `INK_2`.
- Hover: `HOVER` fill, `INK`.
- Rest: `INK_2` glyph and label, value in `INK_4`.

## Eyebrow

`eyebrow()`. A `MICRO` label in `INK_3`, optionally followed 14 later by a
`MICRO_NUM` detail in `INK_4`, for example "Utilization  30 s window". It heads
every section. Section heads never use bold or larger text.

## Stat

`stat()`. A `STAT` value in `INK` over a `MICRO` label 23 below it in `INK_3`.
It has no box. Stat rows divide the width into equal slots with 16 between
slots.

## Readout

`readout()`. The headline metric of a Performance page: a `READOUT` mono value
with a `MICRO` caption. On Memory, a `READOUT_SUB` "/ total" follows the value.

## Process row

32 or 26 high and radius 6, with a `HOVER` fill on hover and `SELECTED` when
selected.

- The name is `BODY` / `INK`.
- Numbers are `NUM`, right-aligned.
- CPU and GPU cells are colored by `heat()`.
- Idle cells step down to `INK_4`.

Group header rows show a chevron (`INK_4`), then an eyebrow with the group name
and count. Clicking a header toggles the group.

## Column header

`MICRO` titles, right-aligned for numeric columns. The sorted column is `INK`
with a 6 px chevron beside it pointing in the sort direction. Every header cell
is a sort target with 4 px of extra hit area above and below.

## Switch

`switch()`. 32 x 18 with a 12 px knob.

- On: a filled `ACCENT` track with an `ON_ACCENT` knob at the right.
- Off: a `GHOST_LINE` outline with an `INK_3` knob at the left.

## Toast

`toast()`. A status notice centered at the bottom of the main area, 60 above
the edge: 38 high, fully rounded, `TOAST` fill, `GHOST_LINE` outline, `BODY`
label. It reports results (signals sent, copies, errors) and carries no
actions. It draws on the overlay layer so rows never show through it.

| Notice | Duration |
| --- | --- |
| Signal result | 4 s |
| Copied, opening a folder | 3 s |
| Errors | 4 to 6 s |

## Context menu

`context_menu()`. Right-clicking a process row opens a floating panel at the
pointer on the overlay layer.

- **Target.** A row outside the selection becomes the selection first. The
  menu acts on the selection as it was when the menu opened.
- **Panel.** Radius 10, `MENU` fill, `GHOST_LINE` outline, 6 padding, 200 to
  320 wide from its longest label. It opens down and right of the pointer and
  flips at the window edges with an 8 margin.
- **Header.** 44 high: the process name (`BODY` / `INK`) over "PID n" or
  "Selection" (`MICRO_NUM` / `INK_4`), closed by a full-width hairline with a
  5 gap before the first item so its highlight clears the rule.
- **Items.** 30 high, radius 6, `BODY` label 10 in, optional right-aligned
  `MICRO_NUM` hint in `INK_4` naming the signal. At rest the label is `INK_2`;
  hover or keyboard focus gives a `HOVER` fill and `INK`.
- **Groups**, split by inset hairlines:
  - End task (SIGTERM) and Force kill (SIGKILL).
  - Suspend (SIGSTOP) and Resume (SIGCONT), each shown only when some target
    can use it.
  - Open file location and Copy command line (single process only), then Copy
    PID or PIDs.
- **Force kill.** The label is `DANGER_INK` on hover. The first click arms it:
  a `DANGER_LINE` outline and "Click again to force kill". The second click
  sends the signal.
- **Modal hover.** While the menu is open nothing beneath it shows hover.
- **Dismissal.** Any click outside, `Esc`, the wheel, a right-click off a row,
  or leaving the page closes it without acting. The menu also closes by itself
  if every target process exits.
- **Keyboard.** Up and Down move focus, wrapping; `Enter` activates.
- **Motion.** It fades in and settles 4 px downward over 140 ms with an
  ease-out cubic.

Suspended processes show a `MICRO` "suspended" tag after their name in
`INK_4`, and their name drops to `INK_3`.

## Graph

`DrawList::graph()`.

- A quarter grid in `GRID` when the graph is at least 80 tall, and a
  `HAIRLINE` baseline.
- Traces 1.25 wide with a wash beneath each.
- 6 px of headroom above 100% so the stroke and its antialiasing are never
  clipped.
- A scale note sits above the top-right corner in `MICRO_NUM` / `INK_4`.

Two-series graphs draw a legend at the top left: a 12 px line sample in the
series color, followed by its `MICRO` name.

Behavior is covered in [Motion and graphics](motion.md).

## Per-core bars

`equalizer()`. One bar per logical core on a hairline baseline. Each bar is 42%
of its slot, between 2 and 10 wide, with a minimum height of 2. Bars are
`INK_2`, `WARN` from 70%, and `HOT` from 90%.

## Meter

The VRAM meter is a 2 px `GRID` track with an `INK` fill. Its eyebrow reads
"used / total".

## Window controls

Minimize and close are bare 14 px hairline glyphs in 28 x 28 hit areas. Hover
adds a round `HOVER` disc, and close additionally turns `DANGER_INK`.
