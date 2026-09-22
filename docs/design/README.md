# ZIGX design

ZIGX is a system monitor drawn directly on compositor blur. There is no widget
toolkit: every surface, glyph, and trace is emitted by `src/frame.rs` and
rendered by `src/gfx.rs`. That makes the design system the code itself, and
these documents the contract for changing it.

| Document | Covers |
| --- | --- |
| [Foundations](foundations.md) | Color, type, shape, iconography |
| [Layout](layout.md) | Window shell, pages, measurements, scaling |
| [Components](components.md) | Pills, segmented control, search, rows, switches, toast |
| [Motion and graphics](motion.md) | Data cadence, graph playback, curves, easing, rendering |

## Principles

**Glass, not paint.** The window is transparent black glass over the desktop
blur. ZIGX never fakes frost, gradients, sheen, or elevation. Depth comes from
the compositor; everything ZIGX draws is flat.

**One ink.** All neutral color is a single spectral white stepped by alpha.
Hierarchy is carried by alpha, size, and case, never by weight or hue.

**Hairlines over boxes.** Structure is 1 px rules and ghost fills. Nothing is
lifted, shadowed, or outlined heavier than it needs to be to read.

**One filled element.** The active choice is a solid white pill with black
text. It is the only opaque fill in the interface, so it always marks "this one".

**Chroma is status.** Amber and red appear only when something is hot or
destructive. Color never decorates.

**Numbers are instruments.** Every number is monospace so columns do not
jitter. Text changes once per second; only graphics move between samples.

**Motion is honest.** Graphs animate real samples on a steady clock. Nothing
overshoots a value that was measured, and nothing moves when there is no new
information.

## Where things live

| Concern | Source |
| --- | --- |
| Color tokens | `theme` module in `src/model.rs` |
| Type roles, layout, components, graphs | `src/frame.rs` |
| Shape SDFs, stroke ribbons, wash, text | `src/gfx.rs` |
| Graph playback clock and easing | `PerfSmooth` in `src/model.rs` |
| Sampling cadence | `spawn` and `Engine::tick` in `src/sample.rs` |
| Input, drags, zoom | `src/interact.rs`, `src/main.rs` |

## Changing the design

- Add a color by adding a token to `theme`, never an inline `[u8; 4]` in a view.
- Add text through an existing type role. A new role needs a reason no current
  role covers.
- Keep controls on the 32 px pill height and the 10 px gap rhythm.
- Anything that animates must be time-based (frame-rate independent), must
  settle, and must not request frames when it has settled.
- Update these documents in the same change.
