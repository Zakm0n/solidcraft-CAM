# Constant-engagement (dynamic) milling

The engine lives in `crates/cam/src/engagement.rs`. It generates overlapping trochoidal loops
along a guide chain so radial engagement stays near a target, rather than burying the cutter in
a corner. It emits the normal `Toolpath`, so caching, posting, cutrail simulation and the
pre/post G-code comparison work unchanged.

## Mastercam parameters modelled

| Mastercam parameter | Our field | Default | Meaning |
| --- | --- | --- | --- |
| Micro lift | `EngagementParams::micro_lift_mm` | 0.2 mm | Lift on the non-cutting back half of each loop and between loops; clears chips and reduces rubbing/heat |
| Back feed | `EngagementParams::back_feed_mm_min` | 1.5 × cutting feed | Feed used for the lifted back moves; independent of the cutting feed |
| Minimum toolpath radius | `EngagementParams::min_toolpath_radius_mm` | 10% of tool diameter | Smallest loop radius the motion may use; `minimum_radius_from_percent(tool, pct)` converts Mastercam's percentage |
| Radial stepover | `stepover_mm` | cutting data, capped at ½ tool diameter | Advance between loop centres |
| Loop radius | `loop_radius_mm` | 45% of tool diameter | Requested trochoid radius; raised to the minimum |

Related formulas:

- Engagement angle for a radial stepover `ae` on tool radius `r`: `cos(θ) = 1 − ae/r`.
- `engagement_angle_deg(r, ae)` and `stepover_for_angle(r, θ)` implement it.
- `effective_radius()` is `max(loop_radius, min_toolpath_radius, 0.05)`.

## Motion

For every depth pass, loop centres are placed every `stepover_mm` along the guide's arc
length. Each loop is split:

1. Cutting half (0° → 180°) at cut depth, at the cutting feed.
2. Back half (180° → 360°) at `depth + micro_lift`, at the back feed.
3. A lifted back move to the next loop start, then a plunge back to depth.

`dynamic_contour(chain, side, params, tool)` offsets the wall by the cutter radius
(`Compensation::None/Left/Right`) and runs the trochoid along it.

## Status

The engine and its parameters are implemented and tested. The Mastercam *operations* named
Dynamic Contour / Dynamic Mill / Peel Mill / Area Mill are still marked **Deferred** in
`docs/mastercam-mapping.md` because they are not yet exposed as `OperationKind` variants or
`cam.*` generator commands; the engine is the foundation those operations will use.

## Sources

Public Mastercam dynamic-milling training material: Mastercam 2017 Introduction to Dynamic
Milling (mcamnw.com), Auburn University Dynamic Milling training, eMastercam dynamic-milling
courseware, and the Public What's New notes. No manuals were uploaded; these are public
sources and the mapping is marked as such.
