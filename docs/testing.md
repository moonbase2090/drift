# Interaction tests

The tests in `crates/drift-cli/tests/e2e/` drive the real `drift` binary the way a person does. Each test builds a fixture system in a temporary directory (a home, the Omarchy templates, the pacman database and a state directory, all under `DRIFT_ROOT`), starts `drift` on a pseudo-terminal, sends keys or mouse clicks, and reads what the program drew. [The feature map](feature-map.md) lists every interaction and the test that covers it.

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
- Every entry in the feature map has a test, and every test is linked from the map.
