# Drift

Drift is an Omarchy plugin, built with [lunatui](https://github.com/moonbase2090/lunatui), that shows how local Hyprland and Omarchy configuration compares to the templates shipped in the installed `omarchy` package.

After an Omarchy update, or any time, it reports which files under `~/.config/hypr` and `~/.config/omarchy` still match those templates, which are local overrides, and which conflict with a newly shipped upstream change. It is meant to replace the all-or-nothing `omarchy-refresh-*` commands with a file-by-file review.

**Status:** the review screen opens. The tree is empty until scanning lands. The product brief is `docs/PRD.md`.

## Review screen

`drift` opens a screen with a header, an empty path tree, and the key hints taken from the keymap. Press `q`, or click Quit, to leave. The terminal is restored when the screen closes.

![Empty Drift screen](docs/media/empty-screen.gif)

`drift --snapshot` prints one 80×24 frame and does not take the terminal. Pass `--width` and `--height` together to print another size.

## Plan dialog

Before anything is written, the plan dialog lists every file with its action, the backup location and a diff summary, and the old transactions that will be pruned. Focus starts on Cancel. `d` expands the selected file's diff, Tab moves between the buttons, `y` confirms, `n` or Escape cancels, and the mouse works on the buttons and rows. The capture below is the real program: `cargo run -p drift-tui --example plan_dialog` opens the dialog on a sample plan and writes nothing; here it expands the first diff, the second diff, moves focus to Confirm and confirms.

![Plan dialog](docs/media/plan-dialog.gif)

## Safety model

- The first view is always a **read-only preview**. Opening Drift never writes configuration.
- A per-file **three-way diff** offers *keep*, *take upstream*, or *merge*. Those actions are explicit: nothing is applied until it is confirmed.
- Before any write, Drift takes a **backup** of the file it is about to change. An **undo** path restores that backup.
- New behavior ships **on by default**. A setting that is off by default is used only when the owner asks for one for that feature.

## Using it

Launch Drift from the Omarchy menu, or as a floating Hyprland window, through the [Omarchy shell-plugin](https://github.com/basecamp/omarchy/blob/quattro/manual/32-shell-plugins.md) mechanism. It is built for Omarchy 4.5 (Quattro). Package templates are read from `/usr/share/omarchy`.

The interface is a path tree plus the per-file diff. Classification uses the installed package file list and, when available, a cache of the previous package version, so "matches template," "local override," and "conflicts with a new upstream change" stay distinct.

## Development

Rust 1.99.1, pinned by `rust-toolchain.toml`. From this repository:

```sh
cargo test
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo xtask check
cargo deny check
```

See the [review policy](REVIEW_POLICY.md) for PR classification, proof, and merge requirements. Pull requests that add or change a visible widget or TUI view must include a README GIF or WebP captured from real program output.
