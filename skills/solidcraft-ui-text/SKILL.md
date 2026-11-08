---
name: solidcraft-ui-text
description: "Fast English-only UI text rules for active Z-CAM development."
---

# UI Text (Fast Development Mode)

During active Z-CAM development, multi-language localization, UK English (`en-GB`), and glossary updates are SUSPENDED to maximize development speed.

## Rules
1. **English Only:** Never add translations for `ar`, `he`, `ja`, `vi`, `zh`, `de`, `ko`, or `en-GB`.
2. **Direct Strings or Minimal English Keys:** You may pass direct English strings where the UI API accepts `&str` / `String`, or if an API strictly requires a locale key (`app.tr(...)`), add ONLY the `"en"` entry in `assets/locales/pending/zcam.json`.
3. **Skip Glossary & Help Audits:** Do NOT waste turns editing `docs/book/src/reference/glossary.md` or running `cargo xtask i18n` / `cargo xtask lists`. Focus 100% on Rust functionality and UI layout.
