# Motion and graphics

The rule: **text changes once per sample, graphics move continuously, and
nothing shows a value that was not measured.** The default sample period is
one second; the rest of this page says "second" for it.

## Data cadence

The sampler (`spawn` in `src/sample.rs`) runs on its own thread with a fixed
deadline clock.

| Constant or setting | Value | Meaning |
| --- | --- | --- |
| Update speed | 0.5, 1 or 2 s | Sample period for every metric in the app |
| `PRIME_MS` | 50 | Delay before the priming read |
| `FIRST_SAMPLE_MS` | 220 | Gap after priming before the first real sample |
| History | 30 s, 1 min, 2 min | Time across a graph; the window in samples is history / period |
| `MAX_WINDOW` | 240 | Longest window: 2 min at 0.5 s |
| `HIST_CAP` | 248 | Ring size: the longest window plus playback delay and curve taps |
| `GRAPH_DELAY` | 3.15 | How many samples behind the newest the graphs play back |

- The period lives in an atomic on the `Hub`. The sampler re-reads it at least
  every 50 ms while it waits, so a new speed or a pause applies at once instead
  of after the old period.
- **Pause** sets the period to 0: no reads, no new snapshot, and the graph
  playhead holds still. Resuming re-primes the engine, so no rate is averaged
  across the gap and the first sample after it is an honest one-period read.
- Changing speed drops the rings and starts a fresh window: old samples are not
  one current period apart, so leaving them would compress or stretch the time
  axis.

- Deadlines advance by exactly one period, so ticks do not drift by the cost
  of each sample. After a stall or suspend the clock re-anchors instead of
  bursting to catch up.
- Every source refreshes on the same tick: CPU, memory, GPU (including NVML),
  disk, network, and the process table. The whole app changes in lockstep.
- Rates (CPU %, disk, network, per-process CPU) are averages over the last
  second, which is also what makes them calm.
- History begins at launch. Graphs start empty and the trace grows in from the
  right edge. While the ring is shorter than `GRAPH_DELAY`, playback stays
  closer to the tip so the first ink appears promptly instead of waiting out
  a full delay on an empty plot.

## Text

Readouts, stats, the sidebar, the title bar, and the process table read the
latest snapshot directly. They change exactly once per second, and never ease
or tick through intermediate values. The Processes page repaints only when a
new sample arrives or the user interacts.

## Graph playback

A graph with a new point every second has to either jump once a second or be
shown slightly in the past. ZIGX shows it in the past, by `GRAPH_DELAY`
samples, for two reasons:

- The right edge always sits between real samples, so it can move
  continuously.
- Every drawn segment is final. A curve segment depends on the two samples
  after its end, so the right edge has to stay three samples back. With only
  one sample of delay, the end of the line reshaped whenever the next sample
  arrived, and snapped visibly at every peak where the direction reversed.

`PerfSmooth::tick` in `src/model.rs` keeps a playback position in sample units:

1. Each frame it advances by exactly `dt / period`, so the scroll speed is
   constant.
2. It then bleeds off the difference from the ideal position (newest sample
   minus the delay, plus time since that sample) with a 0.6 s time constant. Sampler jitter
   and drift are absorbed without a visible change in speed.
3. It is clamped so it never passes the newest sample. If the sampler is late
   by more than the 0.15 margin, the graph holds rather than invents data. The
   time-since-sample term is capped at 1.5 samples, so resuming from a pause
   glides back into place rather than jumping.
4. If it is more than two samples off, for example after the page was hidden,
   it snaps.
5. While paused it does not advance at all.

`head()` exposes the right edge relative to the newest sample (always <= 0).
Graphs and per-core bars both read from it, so they move on one clock.

The trade-off: a change appears in the numbers immediately and in the graph
about three samples later, when the trace draws up to it. At launch the effective
delay is capped by how many samples exist, so the grow-in is not held back.

## Curves

`sample_hist` in `src/frame.rs` blends each sample 1:2:1 with its neighbours,
then evaluates a uniform cubic B-spline over the result:

- It is C2 smooth, so a climb bends over about two samples on each side
  instead of kinking at its foot and crest. A curve forced through every sample has to
  turn within a pixel or two when a large change lands in one second, and that
  reads as a hard edge.
- It never leaves the range of the six samples around it, so it cannot
  overshoot or dip below the data.
- Held plateaus read exactly. An isolated one-sample spike draws as a rounded
  hill about four samples wide, peaking at about two fifths of its measured
  height. At 60 samples across a graph, anything that passes through the
  peak is a needle. The readouts always show the exact value.

Unit tests guard these properties: `curve_stays_inside_its_samples`, and
`drawn_segments_ignore_samples_that_have_not_played` for the playback delay.

The Curves setting can switch to **Linear**: straight segments through every
sample (`sample_linear`). Peaks read at their exact height and corners are
visible. The per-core bars follow the same choice.

## Reduced graph motion

The Graph motion setting's Reduced mode keeps every value honest and removes
the movement between them:

- The graph right edge moves in whole samples (`head()` floors the playhead),
  so traces step once per sample instead of scrolling.
- The VRAM meter and I/O graph scales jump to their target.

It only governs graphs. Interface animation has its own switch; see
[Interface animation](#interface-animation).

### Rigid polylines

Vertices are placed at fixed fractions of each sample slot: 4 to 24 per slot,
about one every 1.5 px. The real samples are always among them, so the
polyline is rigid in data space and only translates as it scrolls. Resampling
at fixed screen positions instead makes sharp peaks shimmer from frame to frame,
because the sample points land on them at a different spot each frame.

The polyline is clipped exactly at the graph's left and right edges.

## Eased graphics

A few graphic values are not histories. They approach their target with a
frame-rate independent exponential, `1 - exp(-dt / tau)`.

| Value | Tau |
| --- | --- |
| VRAM meter fill | 0.14 s |
| Playback clock correction | 0.6 s |

### I/O graph scale

Disk and network graphs autoscale to the nice ceiling (1, 2, 5 x 10^n) of recent
traffic.

- **Target.** The target covers roughly a quarter of the drawn window (clamped
  to 6–16 samples) plus the three samples the curve is heading into. The scale
  grows before a burst is drawn, and shrinks again when recent traffic is
  quieter — even while an older spike is still scrolling off on the left.
- **Easing.** The scale follows its target in log space with a critically
  damped smooth-damp, 0.35 s when growing and 0.5 s when shrinking. Log space
  makes a 100x rescale read as an even zoom instead of an instant squash, and
  critical damping never overshoots.
- **Overshoot.** Zooming back means an older burst can still be on screen
  above the scale. Its line runs off the top of the plot and is cut there
  (`clip_polyline_top`), so it reads as larger than the scale. It is never
  flattened into a plateau, which would read as a value held at the scale. The
  wash stays inside the plot, and nothing enters the top pad with the legend.
- **Label.** The scale label shows the target.
- The per-core bars do not ease. They sample each core's history at the
  playback head, so they glide on the same clock and curve as the utilization
  graph, and their warning colors switch as the bar crosses 70% and 90%.

## Rendering

Frames are presented with FIFO (vsync). While the Performance page is open the
app redraws every display frame; on other pages it waits for input or a new
sample.

- **Strokes** (`stroke_line` in `src/gfx.rs`) are one mitered ribbon per
  polyline, so semi-transparent ink never doubles up at joins. Miters stretch
  at most 2x; turns sharper than 120 degrees, such as the tip of a steep
  spike, split the ribbon and close the outside with a round fan. Sharing one
  vertex there collapsed the ribbon beside the tip, so spikes drew as two
  unjoined hairlines with a stray pixel above them.
  Coverage is a signed distance with a 1.1 px soft fringe. A polyline whose
  last point equals its first is closed: the seam is mitered like any other
  join and gets no caps (the cog glyph relies on this).
- **Wash** (`fill_under`) is a fill from the trace down to the baseline. Each
  vertex carries the trace height at its column and the baseline, which are
  linear across each slice. The fragment shader computes the fade per pixel
  from them, so the fade is exact at any slope and has no seams at the
  triangle splits.
- **Dither.** The wash peaks at about 6% opacity and would band in 8-bit
  color, so it gets a static screen-space dither of half a color step. The
  dither is fixed to the screen, not the content, so it cannot flicker.
- **Shapes** are rounded-box signed distance fields, antialiased over about 1.5
  px at any zoom.

### Snapshots

`cargo test --bin zigx snapshots -- --ignored` renders fixed Disk and CPU
frames through the real pipelines offscreen (`Gfx::headless`, `Gfx::capture`)
and writes them to `target/snapshots/`: each page at 1x, plus its detail pane
at 4x, where single stroke pixels are visible. The fixtures in
`src/snapshot.rs` include lone one-sample bursts, plateaus, and ramps. Look at
a snapshot before and after any change to curves, strokes, or graph scale.

### Layers

A frame has two paint layers: base and overlay. The renderer draws all of a
layer's shapes, strokes, and text before the next layer, so floating surfaces
(the toast and the context menu) fully cover the text beneath them. Everything
drawn also takes a fade multiplier (`DrawList::fade`, nested with
`DrawList::faded`), which entrances and exits use on either layer.

## Interface animation

Everything in the interface that changes state moves there instead of
jumping: hovers, selections, toggles, pages, lists, overlays, zoom. The
Animations setting (on by default, `animations=` in `settings.txt`) turns all
of it off; every value then lands on its target in the same frame. It is
separate from Graph motion, which only concerns graph playback. List
animations (`list_animations=`, on by default) is a narrower switch under
Processes: when it is off, process rows land at once while the rest of the
interface still follows Animations.

### Engine

`Anim` in `src/anim.rs` holds every animated value, keyed by a stable id
(`key(tag, id)`, usually built from the `HitKind` of the control). The frame
is immediate mode, so a widget asks for its value each frame with the target
it wants, and the store advances it:

- Values follow a critically damped spring (`Spring`): it eases in and out,
  never overshoots, and keeps its velocity when the target changes mid-flight,
  so a reversed hover or a second click redirects smoothly.
- `mix` is for 0 to 1 values (alpha, progress) and `slide` for positions in
  design pixels. `mix_from` gives a new slot a starting value, which is how
  entrances and exits begin at 0 or 1.
- `slide` values land at once when the window is resized or zoomed, so layout
  never trails the window edge. Fades still run.
- Values nothing asked for during a frame are dropped, so returning to a
  page starts fresh.
- The step is the real frame time, capped at 50 ms. After an idle stretch the
  first step is one display frame, not the gap, so an animation that starts
  on a still screen does not skip ahead.
- Colors blend in premultiplied space (`mix_rgba`), so fading to or from a
  transparent fill never darkens the ink on the way.

`animating()` is true while any value has not settled, or a closed menu is
still fading. Then the app requests frames at display rate; otherwise a static
page waits. `wake_at()` names the next moment a still screen must repaint on
its own: a toast or an armed confirmation expiring (so it can fade out) and
the caret's next blink.

### Timings

Smooth times in seconds. A spring is about 90% of the way at twice its smooth
time and settled at about three times.

| Constant | Time | Used for |
| --- | --- | --- |
| `HOVER_IN` / `HOVER_OUT` | 0.035 / 0.08 | Hover fills and ink: quick to light, slower to let go |
| `TOGGLE` | 0.065 | Switch knobs, pill widths, state color changes, the chevron and sort caret |
| `GLIDE_LEAD` / `GLIDE_TRAIL` | 0.05 / 0.085 | Gliding highlights |
| `ENTER` | 0.085 | Launch intro, new rows |
| `PAGE` | 0.05 | Page and section switches |
| `SCROLL` | 0.055 | Wheel and keyboard scrolling |
| `REORDER` | 0.075 | Process rows moving to a new slot, row height |
| `MENU_IN` / `MENU_OUT` | 0.05 / 0.04 | Context menu |
| `TOAST` | 0.07 | Toast |
| `ZOOM` | 0.07 | Interface zoom |

### What moves

- **Launch.** The chrome fades in over the glass; the glass itself is there
  from the first frame.
- **Pages.** A page fades in where it sits, settled in about 150 ms. Nothing
  travels and rows arrive together, so a switch never pulls the eye. On
  Performance, sections cross-fade inside the detail pane: the new one fades
  in while the one it replaces fades out at the same time, held at the scroll
  it was left at. Switching back mid-fade picks up from where it was.
- **Highlights.** The selected side item and the active segment's smoked pill
  glide to their new place. The edge in the direction of travel leads and the
  other trails (`Anim::glide`), so the highlight stretches toward its target
  and gathers behind it. Segment labels brighten and gain weight as the pill
  covers them. The context menu's hover highlight glides between items the
  same way, and lights up in place when it first appears.
- **Hover and state.** Every hover fill and ink change cross-fades, as do
  selected rows, focus on the search field, pill danger and disabled styles,
  chip on and off, the sorted column title, and the scrollbar thumb, which
  also thickens under the pointer.
- **Switches.** The track fills as the knob slides across; the knob stretches
  mid-travel so the flip reads as a throw.
- **Process list.** Rows glide to a new slot when the list is re-sorted,
  filtered, or a group opens or closes. A row sorted in from far away starts
  at most three rows from its slot instead of streaking across the list, and
  a move that starts and ends off screen just lands. Processes that were not
  in the previous list fade in where they land. Row height glides on a
  density change. The group chevron turns a quarter; the sort caret
  flattens and flips when the direction changes. Pills re-fit their labels
  ("End task" to "Confirm 2") smoothly and the search field takes the slack.
  List animations turns the row, height, chevron, hover and empty-state
  motion off without touching that chrome.
- **Scrolling.** Wheel and page keys glide to the new offset; a dragged thumb
  is followed exactly. Switching Performance sections does not scroll
  between them.
- **Overlays.** The context menu fades in while settling 6 px down, and fades
  out the same way when it closes or another replaces it. The toast rises in,
  re-fits its width when the message changes, and sinks out when it expires.
- **Title bar.** The readout and the PAUSED flag fade in and out.
- **Caret.** The search caret blinks every 530 ms, restarting on each
  keystroke. It is steady with animations off.
- **Zoom.** `Ctrl` `+` and `-` glide the whole interface to the new scale.

## Rules for new motion

- Base motion on time, not frame count. Use `dt` and a time constant.
- Interface motion goes through `Anim`, so the Animations switch covers it.
  Anything that can move must return its target at once when that is off.
  Process-list row motion also honors List animations via a scoped
  `Anim::enable` around the row block.
- Every animation must settle, and a settled screen must stop requesting
  frames. Something that must change on a timer with nothing moving goes in
  `wake_at()`, not a frame loop.
- Do not ease text. Numbers snap to the new sample.
- Do not overshoot measured data. No springs on values.
- Anything that should move with the graphs reads the playback head, not a
  separate timer.
