# The first-update scenario

On a fresh install Drift has no previous copy of the templates, so it cannot tell a change of yours from a change of upstream's. [`scripts/first-update.sh`](../scripts/first-update.sh) plays the first upgrade end to end in a scratch root and checks that Drift ends up with an exact base for every file, says so, and tells you.

`cargo test` runs it (`crates/drift-cli/tests/first_update.rs`); you can run it by hand:

```bash
cargo build -p drift-cli
DRIFT_BIN=target/debug/drift scripts/first-update.sh
```

The scratch root stands in for `/` through `DRIFT_ROOT`, so the script never touches your configuration, your pacman database or your home. It builds an `omarchy-settings` package database with templates and an `mtree`, a home with five tracked files (two untouched, three edited by the user), no package cache, no snapshots and no recorded bases. Then:

1. **Fresh install.** `drift scan` shows every file with `(base: none)` and the edited ones as `local override [base unknown]`.
2. **Control.** The same package upgrade without Drift's hooks leaves every file at `(base: none)`: an untouched file that upstream changed reads as a local override, which is the confusion the hooks exist to prevent.
3. **Upgrade with the hooks.** The scenario runs the `Exec` line of `packaging/10-drift-snapshot.hook` before the package changes (it copies the template roots to `/var/lib/drift/snapshots/4.5.0-2`), swaps the package, writes the upgrade into `pacman.log` as pacman does, then runs the `PostTransaction` hook (the upgrade stamp) and the notice helper the user unit starts.
4. **Exact bases.** `drift scan` now reads `(base: snapshot)` for every file that differs from its template, with the right states: one clean merge, one conflict, one plain upstream change, one local override. `drift diff` and `drift diff --json` name the snapshot as the base source (`base_provenance`).
5. **Notification.** The fake `omarchy-notification-send` received `3 configs drifted`, normal urgency, with the click action that opens Drift. The control system received nothing.
6. **Keeping a file** records its template as a reviewed base, and the file then reads `(base: reviewed)`.

The same base source is shown per file in the review screen's summary pane (`base: snapshot`).
