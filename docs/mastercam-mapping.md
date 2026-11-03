# Mastercam → SolidCraft CAM mapping

Status values: **Mapped** (behaviour implemented), **Partial** (parameter carried and used,
but not the full Mastercam behaviour), **Deferred** (operation/parameter reserved for a later
milestone), **Unsupported** (deliberately not modelled). Every row is mirrored in
`crates/cam/src/mastercam.rs`; a test keeps this document and that table in sync.

## Contour

| Mastercam parameter | Our field | Default | Status | Notes |
| --- | --- | --- | --- | --- |
| Compensation type/direction | `OperationOptions::compensation` | `None` | Mapped | computer compensation only; controller/wear/reverse-wear not posted |
| Lead in (line length) | `OperationOptions::lead_in_mm` | 3.0 mm | Partial | straight lead-in only; lead-out and arc leads deferred |
| Ramp entry | `OperationOptions::entry`, `ramp_angle_deg`, `ramp_depth_per_pass_mm` | Plunge, 3° | Mapped | ramp follows the contour |
| Depth cuts (max depth, equal depths) | `RoutingParams::depth_per_pass_mm` | tool depth of cut | Mapped | final pass lands exactly on depth |
| Multi passes / finish step | `OperationKind::Contour::finish_passes` | 1 | Partial | count is recorded; wall spacing is not generated |
| Tabs | `toolpath::Tab` | none | Mapped | recorded and posted; simulation lift deferred |
| Stock to leave (wall/floor) | — | — | Unsupported | no allowance is generated yet |
| Linking: clearance / retract | `RoutingParams::clearance_mm`, `retract_mm` | 10 / 2 mm | Partial | feed plane and top-of-stock fields deferred |
| Feed rate / spindle speed | `CuttingData::feed_mm_min`, `spindle_rpm` | from material + tool | Mapped | |

## Pocket

| Mastercam parameter | Our field | Default | Status | Notes |
| --- | --- | --- | --- | --- |
| Cutting method | `OperationOptions::pocket_strategy` | Spiral | Partial | zig-zag and offset spiral; constant-overlap and morph deferred |
| Stepover | `stepover_mm` | tool cutting data | Mapped | percentage input not modelled |
| Depth cuts | `depth_per_pass_mm` | tool depth of cut | Mapped | |
| Finish pass | `OperationOptions::finish_pass` | true | Partial | one wall finish pass; separate wall/floor allowances deferred |
| Entry | `EntryStyle` | Plunge | Mapped | plunge, contour-following ramp, helix |
| Islands | `geometry::Region::islands` | — | Mapped | clipper subtracts islands |
| Stock to leave | — | — | Unsupported | |

## Face

| Mastercam parameter | Our field | Default | Status | Notes |
| --- | --- | --- | --- | --- |
| Cutting angle | `OperationOptions::facing_angle_deg` | 0° | Mapped | raster rotates about the stock centre |
| Stepover / overlap | `stepover_mm` | tool cutting data | Partial | separate overlap percentage is deferred |
| Depth cuts | `depth_per_pass_mm` | tool depth of cut | Mapped | |
| Stock to leave | — | — | Unsupported | |

## Circle Mill

| Mastercam parameter | Our field | Default | Status | Notes |
| --- | --- | --- | --- | --- |
| Circle diameter | `OperationKind::CircleMill::diameter_mm` | — | Mapped | tool diameter subtracted |
| Centre point | `OperationKind::CircleMill::center` | — | Mapped | |
| Helical entry | `EntryStyle::Helix` | Plunge | Mapped | descending full circle |
| Depth cuts | `depth_per_pass_mm` | tool depth of cut | Mapped | |
| Lead in/out around the circle | — | — | Deferred | path is a full circle |

## Slot Mill

| Mastercam parameter | Our field | Default | Status | Notes |
| --- | --- | --- | --- | --- |
| Slot width | `OperationKind::Slot::width_mm` | — | Mapped | two offset passes |
| Centreline | `Operation::selection` | — | Mapped | closed CAM chain |
| Compensation | `OperationOptions::compensation` | None | Mapped | computer side |
| Multiple width passes | — | — | Unsupported | wide slots use more passes in Mastercam |
| Entry/linking/depth | shared routing parameters | — | Mapped | |

## Engrave

| Mastercam parameter | Our field | Default | Status | Notes |
| --- | --- | --- | --- | --- |
| Curves | `Operation::selection` | — | Mapped | open or closed chains |
| Depth | `Operation::depth_mm` | — | Mapped | |
| Direction | chain direction / `Chain::reversed` | — | Mapped | |
| Clearance / retract | `clearance_mm`, `retract_mm` | 10 / 2 mm | Partial | feed plane deferred |
| Stock to leave / roll around corners | — | — | Unsupported | |

## Deferred operations

| Mastercam operation | Our state | Status | Note |
| --- | --- | --- | --- |
| Dynamic Contour | — | Deferred | engagement angle, micro-lift |
| Dynamic Mill | — | Deferred | rest material, constant engagement |
| Peel Mill | — | Deferred | trochoidal motion |
| Area Mill | — | Deferred | rest-material tracking |
| Blend Mill | — | Deferred | open-chain blending |
| Model Chamfer | — | Deferred | solid-model chamfer recognition |
| Helix Bore | — | Deferred | helical bore cycles |
| Thread Mill | — | Deferred | pitch and multi-helix paths |

## Deferred parameter detail

| Operation | Mastercam parameter | Status | Note |
| --- | --- | --- | --- |
| Dynamic Contour | Engagement angle | Deferred | constant-engagement engine not built |
| Dynamic Mill | Micro-lift | Deferred | rest material and constant engagement |
| Peel Mill | Trochoidal stepover | Deferred | trochoidal motion |
| Area Mill | Rest material | Deferred | rest-material tracking |
| Blend Mill | Blend chains | Deferred | open-chain blending |
| Model Chamfer | Chamfer width | Deferred | solid-model chamfer recognition |
| Helix Bore | Bore diameter | Deferred | helical bore cycles |
| Thread Mill | Thread pitch | Deferred | pitch and multi-helix paths |

## Deliberate divergences

- Only computer-side cutter compensation is modelled; controller-side wear offsets are not posted.
- There is no separate lead-out, feed plane, top-of-stock, wall spacing or stock-to-leave field yet.
- Mastercam percentage inputs (stepover %, overlap %) are millimetres here; the caller converts.
- Tabs are recorded and posted, but the cutrail simulation does not lift over them.
- Dynamic/trochoidal operations wait for the constant-engagement engine.
