# Merge upstream/dev into personal: resolve Windows conflicts

Scope: single goal. All steps unchecked below are in scope.

## Definition of Done

The merge of `upstream/dev` (8d6ef0f0) into `personal` resolves all 8 conflicted files
without losing either feature set: the personal Rewrite-selection / paste-to-start-window /
excluded-apps guard features and upstream's DictationFlavour / Terminal paste features.

Gates:
- Gate A (mechanical): no conflict markers anywhere; `cargo check` green; `cargo test` green
  for the touched crates; `cargo clippy` without new warnings on touched files; `cargo fmt --check`.
- Gate B (semantic): independent verifier confirms each personal feature's call path still
  reaches its code and each upstream change survives.

## Steps

- [x] 1. Extend `DictationFlavour` with a `Rewrite` variant in `src/dictation.rs`
  check: `rg "Rewrite" src/dictation.rs` shows the variant and its docs.

- [x] 2. Resolve the 20 conflicts in `src/windows/input.rs` (adopt `flavour`, keep both hotkey
      plumbing paths, fix test literals)
  check: `rg -c "^<<<<<<<|^>>>>>>>" src/windows/input.rs` returns 0 matches.

- [x] 3. Resolve the 22 conflicts in `src/windows/dictation.rs` (Job/Finishing keep `flavour`
      plus `target` and `rewrite_selection`; config keeps both hotkeys)
  check: `rg -c "^<<<<<<<|^>>>>>>>" src/windows/dictation.rs` returns 0 matches.

- [x] 4. Resolve the 2 conflicts in `src/windows/paste.rs` (`pub(crate)` snapshot + `shift_paste`,
      `paste()` takes both `target` and `shift`)
  check: `rg -c "^<<<<<<<|^>>>>>>>" src/windows/paste.rs` returns 0 matches.

- [x] 5. Resolve the 1 conflict in `src/windows/rune.rs` (keep both `ShortcutTarget` variants)
  check: `rg -c "^<<<<<<<|^>>>>>>>" src/windows/rune.rs` returns 0 matches.

- [x] 6. Thread the rewrite hotkey through `src/windows/dictation/shortcuts.rs` and fix all
      compile fallout at call sites outside the conflicted regions
  check: `cargo check` exits 0 from the repo root.

- [x] 7. Review and stage the four pre-merged files (`french.rs`, `portuguese.rs`, `russian.rs`,
      `rune_ui/general.rs`), stage the resolved files, and commit the merge
  check: `git status --porcelain` shows no unmerged paths and `git log -1 --format=%s` is the
  merge commit.

- [x] 8. Gates: `cargo test` for touched areas and `cargo fmt --check`, then independent verifier
  check: verifier returned PASS; unit-test summary 1605 passed, 0 failed.
