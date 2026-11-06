---
name: solidcraft-ground-rules
description: "Use before adding any dependency, icon, image, font, test reference data or product wording to Z-CAM (SolidCraft fork). Covers clean-room independence from other CAD/CAM products, GPL code, asset attribution and the Z-CAM product name."
---

# Independence, licences, assets and the Z-CAM name

The rules that cannot be broken. `AGENTS.md` lists them in one line each; the reasons are in CONTRIBUTING.md.

## Independent (Strict Clean-Room)
- Behaviour comes from Product Owner specifications, public specifications, and general CAD/CAM machining conventions.
- Never copy another product's code, icons, artwork, help text, tooltips, templates or materials.
- Reference tests use analytic values (`tools/oracle-analytic/`) or open-source tools. Expected values never come from a commercial CAD/CAM product.
- Do not drive, script, automate, read or extract data from commercial CAD/CAM products or their proprietary documentation files.
- Command ids are our own: dotted, lower-case `<area>.<thing>[.<variant>]` (for example, `cam.contour.2d`, `cam.tool_library`), never another product's id.

## No GPL, LGPL or AGPL code
- Do not read, copy or link OpenCASCADE, FreeCAD (including planegcs), SolveSpace, CadQuery/OCP, LibreCAD or QCAD. Do not even read their sources.
- Permissive dependencies (MIT, Apache-2.0, BSD) are fine: note any adapted permissive code in ATTRIBUTION.md.

## Assets: the most serious rule
- Z-CAM contains no iconography, images, fonts or artwork from commercial CAD/CAM products, ever. Screenshots of commercial software are never committed.
- Every file asset is original or openly licensed and has a row in ATTRIBUTION.md.
- Command icons are the original SVG masters in `assets/icons/ui/design/svg/`, compiled through `crates/ui-egui/src/icon_svg.rs`.

## The Name (Z-CAM on SolidCraft)
- **User-Facing Branding is Z-CAM:** Window titles, top-level UI branding, application headers, and user-facing dialogs use the name **Z-CAM**.
- **Internal Crate & File Compatibility:** Keep existing internal Rust crate names (`solidcraft-*`), binaries (`solidcraft`, `solidcraft-web`), `.sc` designs, the `solidcraft/N` format tag, and `SOLIDCRAFT_*` environment variables intact for build compatibility.
- Units are millimetres and radians internally; Z is up.
