---
name: zcam-expansion
description: "Use for any Z-CAM work: it says where Z-CAM CAM features go (the real app, never a mock binary), which skills to read first, and that CAM data, toolpaths and dialogs are clean-room from the Product Owner's plain-English specifications."
---

# Z-CAM expansion rules

Z-CAM is the CAM product built on SolidCraft. These rules are mandatory for every Z-CAM change.

## Build in place
- Z-CAM UI and CAM features go into `crates/ui-egui`, `apps/solidcraft` and `apps/solidcraft-web`.
- Never add a standalone mock or demo binary: the production wgpu viewport, the camera path and the B-rep kernel stay live and are what the feature uses.
- A Z-CAM feature is a command in `crates/engine/src/cmd/` plus a dialog in those crates, never a parallel app.

## Read the skill first
Before touching UI, commands or viewport state, read:
- `skills/solidcraft-ground-rules/SKILL.md`
- `skills/solidcraft-ui/SKILL.md`
- `skills/solidcraft-ui-text/SKILL.md`
- `skills/solidcraft-commands/SKILL.md`
- `skills/solidcraft-performance/SKILL.md`
- `skills/solidcraft-checking/SKILL.md`

## Clean room
- CAM data structures (`crates/doc/src/cam.rs`), toolpath algorithms and dialogs are written from the Product Owner's plain-English functional specifications alone.
- Do not read or copy another CAM product's code, screenshots, icons or file formats; `skills/solidcraft-ground-rules/SKILL.md` has the licence and name rules.
- Every CAM command validates hostile parameters and decides with exact geometry, never with a display mesh: `cargo xtask mesh-decisions`.
