## 2024-10-24 - Consistent UI Formatting
**Learning:** Hardcoded "Aborted." and "No changes" text lacked the standard coloring provided by the `console` crate that is used elsewhere in `kaji`.
**Action:** Ensure that standalone status messages are styled using `console::style` and match the color semantics of their accompanying `src::utils::ui` emojis (e.g. red for `ERROR`, green for `CHECK`).
