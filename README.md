# Drift

Drift is an Omarchy plugin, built with [lunatui](https://github.com/moonbase2090/lunatui), that shows how local Hyprland and Omarchy configuration compares to the templates shipped in the installed `omarchy` package.

After an Omarchy update, or any time, it reports which files under `~/.config/hypr` and `~/.config/omarchy` still match those templates, which are local overrides, and which conflict with a newly shipped upstream change. It is meant to replace the all-or-nothing `omarchy-refresh-*` commands with a file-by-file review.

**Status:** read-only scan, status and file diff commands are available. The review screen scans on a worker thread and shows the result as a tree. The product brief is `docs/PRD.md`.

## Review screen

`drift` opens a screen with a header, a path tree, and the key hints taken from the keymap. While a scan is running the tree shows a scanning row, and the rest of the screen still takes input. Press `r` to scan again. A change under a tracked root starts one follow-up scan after a short pause, including when many changes arrive together. Press `q`, or click Quit, to leave. The terminal is restored when the screen closes.

![Empty Drift screen](docs/media/empty-screen.gif)

![Scanning](docs/media/scanning.gif)

`drift --snapshot` prints one 80×24 frame and does not take the terminal. Pass `--width` and `--height` together to print another size.

A changed file opens in five columns once the window is at least 160 cells wide: the hunk list, then base, ours, theirs, and the result. The four text panes share one scroll position. `n` and `p` move between hunks, Tab moves focus, and `[` / `]` or a drag resizes the focused pane.

![Four-pane diff](docs/media/four-pane.gif)

Below 160 columns the same file uses fewer panes. From 100 columns it shows ours and theirs side by side, with the result underneath. Narrower than that, the rows stack in one column. A removed line keeps a minus mark, an added line keeps a plus mark, and a conflict shows ours and theirs. `v` cycles those layouts. A resize that crosses a width boundary follows the width again and keeps the selected hunk. A file with no base stays on two panes, and the header says the base is unknown.

![Narrower diff layouts](docs/media/layout-cycle.gif)

## Plan dialog

Before anything is written, the plan dialog lists every file with its action, the backup location and a diff summary, and the old transactions that will be pruned. Focus starts on Cancel. `d` expands the selected file's diff, Tab moves between the buttons, `y` confirms, `n` or Escape cancels, and the mouse works on the buttons and rows. The capture below is the real program: `cargo run -p drift-tui --example plan_dialog` opens the dialog on a sample plan and writes nothing; here it expands the first diff, the second diff, moves focus to Confirm and confirms.

![Plan dialog](docs/media/plan-dialog.gif)

## Tree screen

`drift` scans the installed package against your files and shows the result as a path tree. Every file has a glyph, its status label and a colour, so no status depends on colour alone; the header counts each status and doubles as the filter buttons. `j` and `k` move, `h` and `l` collapse and expand, `1`-`6` filter by status and `0` clears, `/` searches as you type, `Space` marks and `a` marks everything in the filter. The mouse selects, double-clicks to open, scrolls, toggles carets and checkboxes, and presses the header counts and the hint bar. The capture below is the real program: `cargo run -p drift-tui --example tree_screen` opens the screen on a sample report. It moves to a file (the summary pane shows its template and base), marks it, filters to upstream changes, then searches for `way`.

![Tree screen](docs/media/tree-screen.gif)

## Safety model

- The first view is always a **read-only preview**. Opening Drift never writes configuration.
- A per-file **three-way diff** offers *keep*, *take upstream*, or *merge*. Those actions are explicit: nothing is applied until it is confirmed.
- Before any write, Drift takes a **backup** of the file it is about to change. An **undo** path restores that backup.
- New behavior ships **on by default**. A setting that is off by default is used only when the owner asks for one for that feature.

## Using it

Launch Drift from the Omarchy menu, or as a floating Hyprland window, through the [Omarchy shell-plugin](https://github.com/basecamp/omarchy/blob/quattro/manual/32-shell-plugins.md) mechanism. It is built for Omarchy 4.5 (Quattro). Package templates are read from `/usr/share/omarchy`.

The interface is a path tree plus the per-file diff. Classification uses the installed package file list and, when available, a cache of the previous package version, so "matches template," "local override," and "conflicts with a new upstream change" stay distinct.

## Read-only commands

```sh
drift scan
drift status --json
drift diff .config/hypr/hyprland.conf
drift diff "$HOME/.config/hypr/hyprland.conf" --json
```

`diff` accepts one tracked live path, relative to home or absolute under home.
It prints base provenance, status, base/ours/theirs contents, hunk kinds and the
automatic merge result. JSON includes each hunk's three sides. Conflicts have no
automatic result. Missing or unreadable sides are explicit; non-UTF-8 files and
files over 1 MiB show their hash and size instead of content.

Exit codes are 0 for no drift in the selected file, 1 for drift and 2 for an error,
including an unknown path. Local overrides and user-only files count as no drift,
matching `status`. These commands write no configuration or state.

## Resolving files

```sh
drift resolve .config/hypr/monitors.lua --keep
drift resolve .config/hypr/a.lua .config/waybar/style.css --upstream --dry-run
drift resolve "$HOME/.config/hypr/looknfeel.lua" --merge --yes
```

`resolve` shows a plan (each file, its action, the backup location and a diff summary) and writes only after you answer `y`; `--yes` skips the question, `--dry-run` only prints the plan, and without a terminal nothing is written unless `--yes` is given. All files in one call go into one transaction.

- `--keep` (keep mine) writes no configuration. It records the shipped template as the reviewed base, so the file reads as an acknowledged local override and stays that way after a rescan.
- `--upstream` (take upstream) replaces the file with the template, or creates it when it is missing, after backing up the original.
- `--merge` applies a three-way merge when it has no conflicting hunks. Files with conflicts, no known base, non-text content or only local changes are skipped and named.

Symlinks, files an app rewrites, files without a template and files already equal to upstream are skipped with a reason. A write under `~/.config/hypr` is followed by `hyprctl configerrors`, and Drift offers to undo the transaction if Hyprland reports errors.

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
