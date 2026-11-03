# GRBL scope for SolidCraft CAM

The first post-processor targets GRBL 1.1-style controllers. This document is the
contract for what the post writes, what the G-code reader accepts, and what is
deliberately out of scope.

## Supported

| Code | Meaning | Notes |
| --- | --- | --- |
| `G0` | rapid move | X/Y/Z, modal |
| `G1` | linear feed | X/Y/Z and `F`, modal |
| `G2` / `G3` | clockwise / counter-clockwise arc | X/Y/Z, `I`/`J` centre offsets from the arc start, `F` |
| `G17` | XY plane | the only plane we post or read |
| `G20` / `G21` | inches / millimetres | reader converts inches to mm; post always writes mm |
| `G90` | absolute distance mode | the post always writes absolute |
| `G54` | work offset 1 | the CAM setup is the G54 frame |
| `M3` / `M5` | spindle on (with `S` rpm) / off | spindle speed comes from `CuttingData` |
| `M8` / `M9` | flood coolant on / off | off by default; post option |
| `M30` | program end and reset | footer |
| `;` and `( … )` | comments | ignored by the reader; post writes operation names |
| `N` | line number | post option |

## Not supported (reported, never silently ignored)

| Code | Why |
| --- | --- |
| `T` / `M6` | GRBL has no automatic tool changer; the post names the tool in a comment |
| `G43` / `G49` | no tool-length compensation; Z is already the tool tip |
| `G41` / `G42` | cutter compensation is applied in CAM, not by the controller |
| canned cycles (`G81`–`G89`) | drilling is posted as explicit moves |
| `G18` / `G19` | arcs are only posted in the XY plane |
| `G91` | incremental distance mode is out of scope for the first post |
| subprograms, macros, `M98`/`M99` | not produced or accepted |
| `G28`/`G30` machine home, probe cycles | out of scope |

The G-code reader reports an unsupported or malformed block with its line number and
keeps the rest of the programme; it never guesses a motion.

## Coordinates and units

- CAM works in WCS millimetres; the post emits absolute millimetres (`G21 G90`).
- Arcs use `I`/`J` as the centre offset **from the arc start point**, matching GRBL.
- Helical arcs carry a changing `Z` on the same block; GRBL accepts this.
- The setup origin is G54; the post does not emit machine coordinates.
