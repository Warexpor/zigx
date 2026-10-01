# Components

Every component is a function in `src/frame.rs` that emits slabs, strokes, and
labels. The shared rhythm is a 32 high control, 16 horizontal padding, and a 10
gap between controls.

## Ghost pill

`ghost_pill()`. The standard button style: a transparent, fully rounded shape with
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
- Every state change cross-fades; a pill whose label changes (End task, row
  density) glides to its new width.

## Segmented control

`segmented()`. A single `GHOST_LINE` outline 32 high. Segments sit 3 inside it
with 16 side padding each, so short and long labels share one rhythm. The
active segment is the `ACCENT` pill with a `PILL_ON` label in `ON_ACCENT`.
Inactive labels are `INK_2`; hover gives a `HOVER` pill and `INK`. It is used
for the process view and for every one-of-many setting. The smoked pill
glides between segments, stretching toward the new one, and each label
brightens and gains weight as the pill covers it.

## Chips

Independent toggles in a row with an 8 gap, one per value (the optional process
columns). Off is a text-only ghost pill; on is the `ACCENT` pill with a
`PILL_ON` label. Unlike a segmented control, any number can be on.

## Stepper

The interface scale control: two 32 x 32 round ghost pills holding a hairline
minus and plus, with the value in `NUM` / `INK` centered in 64 between them. A
button at the end of the zoom range is disabled.

## Search field

A pill-shaped field with a search glyph at 12 and text from 34.

| State | Outline | Fill | Text |
| --- | --- | --- | --- |
| Rest | `GHOST_LINE` | None | "Search" placeholder, `PILL`, `INK_3` |
| Hover | `ACCENT_LINE` | None | Placeholder `INK_2` |
| Focused | `ACCENT_LINE` | `GHOST` | Query in `NUM`, `INK`, with a caret |

Search takes keys only while the field is focused, by click or `Ctrl+F`.
`Esc` clears it, and `Ctrl+Backspace` empties it. `Space` freezes the process
list until the field is focused.

## End task

A ghost pill whose label follows the selection; the context menu offers the
same action without the confirm step. With nothing selected it is
disabled and reads "End task". The first press, or `Delete`, arms it for 4
seconds: it switches to the danger style and reads "Confirm N". A second press,
or `Enter`, sends SIGTERM. Changing the selection disarms it. With Confirm
ending off in Settings, the first press sends SIGTERM.

## Side item

`side_item()`. 34 high, radius 6, used by the nav and the Performance sub-nav.
The glyph sits at 12 and the label at 36. An optional value sits right-aligned
in `MICRO_NUM`.

- Active: `GHOST` fill, `INK` glyph and label, value in `INK_2`.
- Hover: `HOVER` fill, `INK`.
- Rest: `INK_2` glyph and label, value in `INK_4`.
- The active fill is one shape per list (`side_highlight()`) that glides
  between items rather than jumping.

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
selected. Rows glide to new slots on a re-sort, filter or group change, and
new processes fade in; see [Interface animation](motion.md#what-moves).

- The name is `BODY` / `INK`.
- Numbers are `NUM`, right-aligned.
- CPU and GPU cells are colored by `heat()`.
- Idle cells step down to `INK_4`.

Group header rows show a chevron (`INK_4`), the program name with a process
count, and the same metric columns as a process row with values summed across
members (CPU, GPU, memory, disk, threads). Groups start collapsed; clicking a
header expands it, and the chevron turns a quarter between right and down.
Only names with two or more processes get a header; one-offs stay as ordinary
process rows. PID is an em dash on the header; User shows when every member
shares one.

## Column header

`MICRO` titles, right-aligned for numeric columns. The sorted column is `INK`
with a 6 px chevron beside it pointing in the sort direction. It flattens and
flips when the direction changes, and fades across when the sorted column
changes. Every header cell
is a sort target with 4 px of extra hit area above and below.

## Switch

`switch()`. 32 x 18 with a 12 px knob.

- On: a smoked `ACCENT` track with an `ON_ACCENT` knob at the right.
- Off: a `GHOST_LINE` outline with an `INK_3` knob at the left.
- The knob slides between the two while the track fills, stretching to 18 px
  wide mid-travel.
- A row that carries a switch (startup entries, setting rows) is clickable end
  to end but shows no hover fill: the switch is the only state on the row.

## Toast

`toast()`. A status notice centered at the bottom of the main area, 60 above
the edge: 38 high, fully rounded, `TOAST` fill, `GHOST_LINE` outline, `BODY`
label. It reports results (signals sent, copies, errors) and carries no
actions. It draws on the overlay layer so rows never show through it. It rises
14 px as it fades in, sinks out after it expires, and glides to a new width
when a new message replaces the old one.

| Notice | Duration |
| --- | --- |
| Signal result | 4 s |
| Copied, opening a folder | 3 s |
| Errors | 4 to 6 s |

## Context menu

`context_menu()`. Right-clicking a process row or a group header opens a
floating panel at the pointer on the overlay layer.

- **Target.** A row outside the selection becomes the selection first. The
  menu acts on the selection as it was when the menu opened. On a group
  header the selection clears, the header takes focus, and the menu acts on
  every member the list shows under the current view and search. Members
  that exit while it is open drop out of its counts.
- **Panel.** Radius 10, `MENU` fill, `GHOST_LINE` outline, 6 padding, 200 to
  320 wide from its longest label. It opens down and right of the pointer and
  flips at the window edges with an 8 margin.
- **Header.** 44 high: the process name (`BODY` / `INK`) over "PID n",
  "Selection" or "Group · n processes" (`MICRO_NUM` / `INK_4`), closed by a
  full-width hairline with a
  5 gap before the first item so its highlight clears the rule.
- **Items.** 30 high, radius 6, `BODY` label 10 in, optional right-aligned
  `MICRO_NUM` hint in `INK_4` naming the signal. At rest the label is `INK_2`;
  hover or keyboard focus gives a `HOVER` fill and `INK`. The fill is one
  highlight that glides between items.
- **Groups**, split by inset hairlines:
  - End task (SIGTERM) and Force kill (SIGKILL).
  - Suspend (SIGSTOP) and Resume (SIGCONT), each shown only when some target
    can use it.
  - Group menu only: Expand or Collapse group, and Select all n, which opens
    the group and selects its members.
  - Open file location (single process or group), Copy command line (single
    process only), then Copy PID or PIDs.
  - A group menu labels the signals "End all n" and "Force kill all n".
- **Force kill.** The label is `DANGER_INK` on hover. The first click arms it:
  a `DANGER_LINE` outline and "Click again to force kill". The second click
  sends the signal. With Confirm ending off, the first click sends it.
- **Modal hover.** While the menu is open nothing beneath it shows hover.
- **Dismissal.** Any click outside, `Esc`, the wheel, a right-click off a row,
  or leaving the page closes it without acting. The menu also closes by itself
  if every target process exits.
- **Keyboard.** Up and Down move focus, wrapping; `Enter` activates.
- **Motion.** It fades in while settling 6 px downward, and fades out the
  same way when it closes (drawn without hit targets). A menu opened over
  another replaces it with a cross-fade. With animations off it appears and
  disappears at once.

Suspended processes show a `MICRO` "suspended" tag after their name in
`INK_4`, and their name drops to `INK_3`.

## Shortcut sheet

`keys_sheet()`. Opened by `?`, `F1`, or the Keys legend at the right of the
version line in the nav foot. A modal panel centered on the window, on the
overlay layer.

- **Keys legend.** `keys_hint()`. A 16 px "?" cap drawn like the sheet's keys,
  then "Keys" in `MICRO`, set like the version beside it. No container at
  rest, `INK_3`; hover gives a radius 6 `HOVER` wash, like a nav item, and
  `INK`. Its right edge lines up with the nav items.

- **Backdrop.** The whole window dims under black at 120 alpha. Clicking it
  closes the sheet.
- **Panel.** Up to 860 wide with a 24 margin at every window edge, radius 14,
  `MENU` fill, `GHOST_LINE` outline, 28 padding.
- **Header.** "Keyboard" in `TITLE` over a `SUB` line in `INK_3`, "Esc to
  close" right-aligned in `MICRO_NUM` / `INK_4`, closed by a full-width
  hairline.
- **Groups.** An eyebrow per page, General first, then the page you are on
  (its eyebrow detail reads "This page"), then the rest. Two columns when each
  can hold 340, one otherwise; each group goes to the shorter column.
- **Rows.** 26 high. Keys sit in a 140 column as caps: 20 high, radius 5,
  `HOVER` fill, `GHOST_LINE` outline, and `NUM_SMALL` / `INK` labels. `+` joins a chord and `/` separates
  alternatives, both in `INK_4`. The action follows in `BODY` / `INK_2`.
- **Overflow.** Content scrolls under the header by wheel, arrows, and page
  keys, with a 2 px `GHOST_LINE` thumb at the right edge.
- **Modal.** While it is open nothing beneath it hovers or takes keys.
- **Motion.** Fades in while settling 6 px downward and fades out the same
  way. With animations off it appears and disappears at once.

## Graph

`DrawList::graph()`.

- A quarter grid in `GRID` when the graph is at least 80 tall and Grid is on,
  and a `HAIRLINE` baseline.
- Traces 1.25 wide with a wash beneath each when Fill is on.
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

## Setting row

`settings_page()`. 60 high, closed by a hairline. The label is `BODY` / `INK`
at 12 and a one-line description `SUB` / `INK_3` at 32; the control sits at the
right edge, centered vertically. When the control is wider than 58% of the
row, it drops below the description (row 100 high) so narrow windows never
crowd the label. A switch row is clickable end to end, like a startup entry.

Reset is a ghost pill, "Reset all". The first click arms it for 4 seconds in the
danger style ("Click again to reset"); the second restores every setting, the
zoom and row density, and confirms with a toast.

## Window controls

Minimize and close are bare 14 px hairline glyphs in 28 x 28 hit areas. Hover
fades in a round `HOVER` disc, and close additionally turns `DANGER_INK`.
