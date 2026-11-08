# Fork maintenance: merging upstream/dev

This repository is a fork. `origin` is the user's fork, `upstream` is the
original repository, `personal` is the user's branch, and `dev` mirrors
`upstream/dev`. The user routes merge work through the agent. These rules are
mandatory whenever asked to fetch, merge, or cherry-pick upstream changes.

## Flow

1. `git fetch upstream`.
2. Start from a clean `personal` and snapshot it first:
   `git branch backup/personal-<date>`.
3. Optional mirror update: `git switch dev && git reset --hard upstream/dev`.
4. `git switch personal && git merge upstream/dev`.
5. Resolve conflicts under the conflict priority below.
6. Verify: `cargo check --all-targets`, `cargo test --bin hex`,
   `cargo fmt --check`, `cargo clippy --all-targets`. A merge is not finished
   until all four pass.
7. Commit the merge. Never `git push --force` and never rewrite `personal`
   history. Pushing is the user's call unless they ask for it.

For one selected upstream feature, prefer `git cherry-pick -x <sha>` over a
full merge, and apply the same rules to its conflicts.

## Conflict priority

1. **Keep both when both can work.** Prefer a resolution where upstream's change
   and the user's feature both survive. Adapt the user's code to the new
   upstream shape: thread both parameters, extend an enum, keep both fields.
2. **The user's code wins when the two cannot coexist.** If the choice is
   either/or, keep the fork's behavior and its call path.
3. **Take upstream when its change is minor and safe.** A small fix, rename,
   comment, or mechanical refactor that does not change the user's feature
   behavior may replace the fork's copy; fix the user's call sites to match.
4. **A complete upstream rework of code the user also changed loses to the
   fork.** If upstream rewrote the mechanism the fork customizes (new
   architecture, renamed subsystem, replaced data model), keep the user's
   implementation and graft only the specific improvements they asked for.
5. **When a conflict cannot be reconciled, drop the upstream hunk.** Keep the
   user's side deliberately, for example `git checkout --ours` for that file,
   or the merged shape with the upstream addition removed. Never leave conflict
   markers and never delete personal functionality to make upstream fit. Record
   each such drop in the merge commit body with the file and reason.
6. **Never discard personal work to solve a conflict.** If a personal feature
   stops compiling or its tests fail after taking an upstream hunk, revert that
   hunk first; removing the feature is not a resolution.

## Judgement calls

- Treat anything in the fork's personal history as user-owned. When unsure
  whether an upstream change is a minor fix or a rework, keep the user's
  behavior and adapt the upstream side.
- Upstream test, docs, typo, and CI-only changes are safe to take wholesale.
- Upstream changes to shared plumbing the user has extended (settings structs,
  event enums, hotkey plumbing, paste paths) are usually "keep both": adopt
  upstream's new representation and add the personal field or variant to it.
- Deleted platform code is the user's choice: do not resurrect Linux or macOS
  files unless a feature the user asked for needs them.

## Reporting

Report the files resolved by keeping both, the files where upstream was taken
as a minor change, the files where upstream was dropped with the reason, the
verification commands run, and anything left for the user.
