# Motion and graphics

The rule: **text changes once per second, graphics move continuously, and
nothing shows a value that was not measured.**

## Data cadence

The sampler (`spawn` in `src/sample.rs`) runs on its own thread with a fixed
deadline clock.

| Constant | Value | Meaning |
| --- | --- | --- |
| `SAMPLE_PERIOD_MS` | 1000 | One sample per second for every metric in the app |
| `PRIME_MS` | 250 | Gap between the priming read and the first real sample |
| `HIST_WINDOW` | 30 | Samples across a graph, so a 30 s window |
| `HIST_CAP` | 36 | Ring size: the window plus playback delay and curve taps |
| `GRAPH_DELAY` | 2.15 | How many samples behind the newest the graphs play back |

- Deadlines advance by exactly one period, so ticks do not drift by the cost
  of each sample. After a stall or suspend the clock re-anchors instead of
  bursting to catch up.
- Every source refreshes on the same tick: CPU, memory, GPU (including NVML),
  disk, network, and the process table. The whole app changes in lockstep.
- Rates (CPU %, disk, network, per-process CPU) are averages over the last
  second, which is also what makes them calm.
- History begins at launch. Graphs start empty and the trace grows in from the
  right edge.

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
- Every drawn segment is final. A curve segment depends on the sample after
  its end, so the right edge has to stay two samples back. With only one
  sample of delay, the end of the line reshaped whenever the next sample
  arrived, and snapped visibly at every peak where the direction reversed.

`PerfSmooth::tick` in `src/model.rs` keeps a playback position in sample units:

1. Each frame it advances by exactly `dt / period`, so the scroll speed is
   constant.
2. It then bleeds off the difference from the ideal position (newest sample
   minus the delay, plus time since that sample) with a 0.6 s time constant. Sampler jitter
   and drift are absorbed without a visible change in speed.
3. It is clamped so it never passes the newest sample. If the sampler is late
   by more than the 0.15 margin, the graph holds rather than invents data.
4. If it is more than two samples off, for example after the page was hidden,
   it snaps.

`head()` exposes the right edge relative to the newest sample (always <= 0).
Graphs and per-core bars both read from it, so they move on one clock.

The trade-off: a change appears in the numbers immediately and in the graph
about two seconds later, when the trace draws up to it.

## Curves

`sample_hist` in `src/frame.rs` evaluates a uniform cubic B-spline over the
samples:

- It is C2 smooth, so a climb bends over about a sample on each side instead of
  kinking at its foot and crest. A curve forced through every sample has to
  turn within a pixel or two when a large change lands in one second, and that
  reads as a hard edge.
- It never leaves the range of the four samples around it, so it cannot
  overshoot or dip below the data.
- Held plateaus read exactly. An isolated one-sample spike peaks at two thirds
  of its measured height. The readouts always show the exact value.

Unit tests guard these properties: `curve_stays_inside_its_samples`, and
`drawn_segments_ignore_samples_that_have_not_played` for the playback delay.

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

Disk and network graphs autoscale to the nice ceiling (1, 2, 5 x 10^n) of what
they show.

- **Target.** The target covers the visible window plus the two samples the
  curve is heading into. The scale therefore grows before a burst is drawn,
  and relaxes only after the burst scrolls out on the left.
- **Easing.** The scale follows its target in log space with a critically
  damped smooth-damp, 0.35 s when growing and 0.9 s when shrinking. Log space
  makes a 100x rescale read as an even zoom instead of an instant squash, and
  critical damping never overshoots.
- **No clipping.** If a trace would still exceed the eased scale, the graph
  uses the trace's peak for that frame, so a line is never clipped flat against
  the top.
- **Label.** The scale label shows the target.
- The per-core bars do not ease. They sample each core's history at the
  playback head, so they glide on the same clock and curve as the utilization
  graph, and their warning colors switch as the bar crosses 70% and 90%.

## Rendering

Frames are presented with FIFO (vsync). While the Performance page is open the
app redraws every display frame; on other pages it waits for input or a new
sample.

- **Strokes** (`stroke_line` in `src/gfx.rs`) are one mitered ribbon per
  polyline, so semi-transparent ink never doubles up at joins. Joins sharper
  than about 88 degrees fall back to a bevel so spikes do not throw long miters.
  Coverage is a signed distance with a 1.1 px soft fringe.
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

## Rules for new motion

- Base motion on time, not frame count. Use `dt` and a time constant.
- Every animation must settle, and a settled screen must stop requesting
  frames.
- Do not ease text. Numbers snap to the new sample.
- Do not overshoot measured data. No springs on values.
- Anything that should move with the graphs reads the playback head, not a
  separate timer.
