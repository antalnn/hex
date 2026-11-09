# Hex UI sandbox

A standalone crate for Hex's settings window: the real pane code drawn over a
fixture host, so UI work rebuilds in seconds without audio, hotkeys, the
service pipeline, or the single-instance event.

The main crate excludes this folder from its workspace (`Cargo.toml`), so

```
cd ui-sandbox
cargo run
```

builds this crate alone. It is not a member and nothing here links into Hex.

## What is real, what is a fixture

Real copies, edited here and moved back once green:

- `src/rune_ui/general.rs`, `general/search.rs`, `general/shortcuts.rs`
- `src/rune_ui/controls.rs`, `icons.rs`, `lists.rs`, `list_detail.rs`
- `src/desktop/i18n.rs` and the language tables

Fixtures stand in for the host the panes read: `src/app_settings.rs` (the same
field names and shapes, no persistence), `src/desktop/host.rs` (the snapshot
types), `src/transcription/models.rs` (a two-model catalog), and the small
modules beside them. `src/rune_ui.rs` is the window shell: the `Ui` state, the
pane it draws, and the fixture service that records requests instead of
sending them.

The i18n check scans the sources listed in `src/desktop/i18n.rs`
(`SOURCES`); the sandbox lists only the files it carries, and its floor is
150 strings rather than the app's 300.

## Checks

```
cargo fmt --all -- --check
cargo check --all-targets
cargo test
```

`cargo test general_reads_the_hardware_then_the_keys_then_the_rest` runs the
General pane's order and grouping test headlessly, the same one the app runs
on Windows (`src/rune_ui/accessibility_tests.rs`).
