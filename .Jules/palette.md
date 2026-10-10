## 2023-10-27 - Add aliases to improve discoverability
**Learning:** Adding visible aliases to clap CLI commands significantly enhances discoverability without needing extra documentation. Aliases like `push` and `deploy` on the `Apply` command mirror familiar workflows from Git and other deployment tools.
**Action:** When adding or maintaining CLI commands, consider common synonyms and workflows (e.g. `pull`/`sync` for `inspect`, `push`/`deploy` for `apply`) and add them as `visible_alias` entries in the clap `#[command(...)]` attribute.
