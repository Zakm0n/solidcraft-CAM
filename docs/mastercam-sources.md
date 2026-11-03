# Mastercam reference sources

Uploaded manuals: **none supplied yet**. The mappings below were extracted from public
Mastercam training material and documentation. Uploaded manuals will supersede any
assumption and the affected rows will be re-marked.

| # | Source | Used for |
| --- | --- | --- |
| S1 | Mastercam X Entry (`mastercam.dk/pdf/X4-Entry.pdf`) | entry motions: plunge, helical, ramp; ramp angle and "preferred profile length"; entry feed |
| S2 | Harvard GSD Mastercam 2020 step-by-step guide | contour page: compensation type/direction, lead in/out, depth cuts, linking heights, tabs; material/tool defaults |
| S3 | eMastercam X7/X9 handbook samples | lead in/out fields, multi passes, stock to leave, finish/spring passes, tab definition |
| S4 | Mastercam 2022 / 2025 What's New | arc filter/tolerance page, hole-making lead in/out, thread mill lead in/out |
| S5 | Mastercam X5 Reference Guide (caminstructor) | engraving parameters (clearance, retract, feed plane, depth, direction); slot mill description (obround slots) |
| S6 | Mastercam Dynamic Milling training PDFs (Auburn, Mastercam NW, colla.lv) | dynamic milling/peel mill concepts for the deferred operations |

## Parameter pages extracted

### Contour
- Compensation type: computer / controller / wear / reverse wear → our `Compensation` is
  computer-side only (`None`/`Left`/`Right`); controller and wear modes are unsupported.
- Lead in/out: line, arc, length, ramp → our `lead_in_mm` covers a straight lead-in only;
  lead-out and arc leads are deferred.
- Depth cuts: maximum depth, number/equal depths, finish step (spring pass) → our
  `depth_per_pass_mm` plus `finish_pass`.
- Multi passes: number, spacing, finish pass → partial via `finish_passes` (recorded) and
  `finish_pass`; no separate wall spacing.
- Linking: clearance, retract, feed plane, top of stock, depth → our clearance/retract/depth;
  feed plane and top-of-stock fields are deferred.
- Tabs: width, height, position → our `Tab` (recorded; simulation does not lift over them yet).
- Stock to leave: wall/floor → deferred.

### Pocket
- Cutting method: zig-zag, constant overlap spiral, parallel spiral, morph → our
  `PocketStrategy::{ZigZag, Spiral}`; constant-overlap and morph are deferred.
- Stepover (distance/percentage) → `stepover_mm` (percentage input not modelled).
- Depth cuts → `depth_per_pass_mm`.
- Finish passes → `finish_pass` (no wall/floor separation).
- Entry: plunge/helical/ramp → `EntryStyle` plus ramp angle/depth.
- Islands → CAM regions with islands.
- Stock to leave → deferred.

### Face
- Cutting angle → `facing_angle_deg`.
- Stepover/overlap → `stepover_mm`; separate overlap is deferred.
- Depth cuts → `depth_per_pass_mm`.
- Stock to leave → deferred.

### Circle Mill
- Circle diameter / centre point → `CircleMill { diameter_mm, center }`.
- Entry: helix/plunge → `EntryStyle::{Helix, Plunge}`.
- Depth cuts → `depth_per_pass_mm`.
- Lead in/out around the circle → deferred (path itself is a full circle).

### Slot Mill
- Slot width → `width_mm`.
- Centreline geometry → selected CAM chain.
- Compensation → `Compensation` (computer side).
- Entry/linking/depth → shared routing parameters.
- Multiple passes/stepover across a wide slot → deferred.

### Engrave
- Curves → selected CAM chain.
- Depth, direction → depth and chain direction.
- Clearance/retract/feed plane → clearance/retract; feed plane deferred.
- Stock to leave, roll-around-corners → deferred.

### Dynamic / trochoidal (deferred operations)
Dynamic Contour, Dynamic Mill, Peel Mill, Area Mill, Blend, Model Chamfer, Helix Bore and
Thread Mill are deferred until the constant-engagement engine exists. Their parameter
families (engagement angle, micro-lift, rest material, lead in/out for thread milling) are
recorded in the mapping table as deferred, not as implemented.
