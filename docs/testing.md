# Interaction tests

The tests in `crates/drift-cli/tests/e2e/` drive the real `drift` binary the way a person does. Each test builds a fixture system in a temporary directory (a home, the Omarchy templates, the pacman database and a state directory, all under `DRIFT_ROOT`), starts `drift` on a pseudo-terminal, sends keys or mouse clicks, and reads what the program drew. [The feature map](feature-map.md) lists every interaction and the test that covers it.

## What the test suite needs

`cargo test --workspace` runs on Linux. The terminal crate uses Linux system calls and does not build on macOS. CI uses Ubuntu, and a fresh Ubuntu 24.04 machine runs the whole suite with these packages added to the Rust toolchain:

- a C toolchain and `pkg-config` (`build-essential pkg-config`)
- `libalpm-dev`. The notify tests load `libalpm` at run time to hold a real package-manager lock, and `a_libalpm_transaction_suppresses_until_the_lock_is_removed` fails with "libalpm is not available" without it.
- `nodejs`. `the_bar_widget_model_passes_its_node_tests` runs the widget model tests with `node --test` and fails when `node` is missing. Set `DRIFT_SKIP_JS_TESTS=1` to skip it on a machine without node.

A missing prerequisite fails those tests on purpose, so a machine that lacks one cannot report a clean run.

## How the harness works

- The program runs on a pseudo-terminal of a chosen size. A reader thread feeds everything it prints to a terminal emulator (`vt100`).
- A test never sleeps. It waits on the screen: `wait_for` re-checks a condition every time a frame arrives and fails with the screen text when 30 seconds pass. The limit is a bound on failure, not a delay.
- The screen a test sees is the last complete frame. Drift wraps each frame in the synchronized-update markers (`CSI ? 2026 h` and `l`), and the harness only publishes at the end marker, so a test cannot see a half-drawn screen.
- Keys are written as the bytes a terminal sends, and a click is an SGR mouse report at a cell found by searching the screen for its text.
- A test that writes checks the files on disk afterwards, in the fixture's own directories.

## Why not Termwright

Termwright (0.2.0) does the same job, and was tried first. It pulls in an async runtime, a font renderer and an image library for screenshots, which makes `cargo tree` 372 lines long where the workspace's test dependencies make 36 and takes about three minutes to build clean, and `cargo deny check` rejects the tree (five unmaintained packages and three licences the policy does not allow). The harness here needs a pty, a terminal emulator and a wait, so it is a few hundred lines on `libc` and `vt100`; `libc` is already a dependency and `vt100` was already used by the resize test.

## Rules for a test

- It guards behaviour a user would notice, and fails when that behaviour breaks. A test that only restates the code is not accepted.
- It is deterministic: no sleeps, no timing windows, no order between tests.
- Every entry in the feature map is linked to a test or marked planned, and every test is linked from the map.

## Replay the theme, widget and notify guards

`python scripts/check-theme-widget-notify-guards.py --repeat 5` runs the PTY suite, repeats the theme/widget/notify cases with eight test threads, then checks nine controlled production mutants. Run it in an idle, clean worktree. It restores each source file in a `finally` block and requires the named test to fail behaviorally. Logs and the commit-stamped summary are written under `target/e2e-guard-proof/`. `--mutant <name>` selects one recipe for investigation. The final PTY run checks the restored source.

Theme tests inspect foreground and background colors from completed terminal frames, including contrast and live dark/light changes. Widget tests inspect installed files, hashes, preserved edits, backups and the arguments passed to fixture Omarchy commands. Notify tests inspect actual notifier arguments, persisted preferences and suppression while real fixture locks are held. All commands run in fixture homes through the same PTY harness as the tree and diff tests.

## Config and CLI diagnostic guards

`cargo test -p drift-cli --test config_and_messages` runs the real binary against isolated config, package and lock fixtures. It checks snapshot contents for XDG config precedence and fallback, error paths, dimension overflow, and lock diagnostics on commands that have nothing to change.

From a clean, idle worktree, run `python scripts/check-config-cli-guards.py`. It temporarily breaks config selection, overflow reporting and each no-op lock diagnostic, requires each named regression test to fail, restores the source and reruns the suite. Logs and the head-stamped summary go under `target/config-cli-guard-proof/`. Compiler errors do not count as caught regressions.
