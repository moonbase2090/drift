# Command reference

Every command below is run by the documentation test against a fixture system, so what you read here is what the program does. Exit codes: `0` nothing drifted (or success), `1` drift was found, `2` an error. Commands that write ask first, and without a terminal they write nothing unless you pass `--yes`.

## Scanning and status

```sh
drift scan
drift status
drift status --json
drift status --bar
```

`scan` lists every tracked file with its status. `status` prints a summary: counts per status and the package version, as JSON with `--json`. `status --bar` prints one JSON object for a status bar: `text` is the drifted count (empty when nothing drifted), `tooltip` names the count and `class` is `clean` or `drifted`. Local overrides and user-only files count as no drift. These commands write nothing.

## Diff

```sh
drift diff .config/hypr/hyprland.conf
drift diff "$HOME/.config/hypr/hyprland.conf" --json
```

`diff` takes one tracked live path, relative to your home or absolute under it. It prints the base provenance, the status, the base, ours and theirs contents, the hunk kinds and the automatic merge result; conflicts have no automatic result. `--json` includes each hunk's three sides. Missing and unreadable sides are stated, and non-UTF-8 files and files over 1 MiB show a hash and size instead of content. The exit code is `1` when that file drifted and `2` for an unknown path.

## Resolving files

```sh
drift resolve .config/hypr/monitors.lua --keep
drift resolve .config/hypr/hyprland.conf .config/waybar/style.css --upstream --dry-run
drift resolve "$HOME/.config/hypr/looknfeel.lua" --merge --yes
```

`resolve` shows a plan (each file, its action, the backup location and a diff summary) and writes only after you answer `y`. `--yes` skips the question and `--dry-run` only prints the plan. All the files of one call go into one transaction.

- `--keep` (keep mine) writes no configuration. It records the shipped template as the reviewed base, so the file reads as an acknowledged local override and stays that way after a rescan.
- `--upstream` (take upstream) replaces the file with the template, or creates it when it is missing, after backing up the original.
- `--merge` applies a three-way merge when it has no conflicting hunks. Files with conflicts, no known base, non-text content or only local changes are skipped and named.

Symlinks, files an app rewrites, files without a template and files already equal to upstream are skipped with a reason. A write under `~/.config/hypr` is followed by `hyprctl configerrors`, and Drift offers to undo the transaction if Hyprland reports errors.

## History and undo

```sh
drift history
drift history --json
drift undo --dry-run
drift undo <transaction> --yes
drift undo --force .config/waybar/config.jsonc --dry-run
```

`history` lists the transactions Drift has written, newest first: id, time, package version, file count and what was done. `undo` previews what undoing a transaction (the newest by default; name one with `<transaction>`, an id from `history`) would restore or remove, and writes only after you answer `y`. Undoing puts back the reviewed bases the transaction replaced, so an undone take upstream reads as upstream changed again. The undo is a transaction of its own and can be undone in turn. A file that changed after the transaction is left alone and named; `--force <path>` restores it anyway.

## Upgrade snapshot and notice

```sh
drift snapshot --pre
drift notify
drift notify off
drift notify on
```

`snapshot --pre` copies the installed template roots into `/var/lib/drift/snapshots/<version>/` and writes only there. Pacman runs it from a PreTransaction hook before an `omarchy-settings` upgrade; a failure exits `2` and does not stop the upgrade.

`notify` scans and, when configuration has drifted, sends a notification through `omarchy-notification-send`: `1 config drifted` or `N configs drifted`, normal urgency, and a click opens Drift. Nothing is sent when nothing drifted, while another process holds the Omarchy update lock, while pacman's database lock exists, or after `drift notify off`. `notify on` turns notices back on (they start on); the choice is kept in Drift's state. `notify` exits `0` when it sends or skips and `2` when the scan or the notifier fails.

## Integrating with Omarchy

```sh
drift integrate --dry-run
drift integrate --yes
```

`integrate` adds a Drift row to the Omarchy menu (`~/.config/omarchy/extensions/omarchy-menu.jsonc`) and a hook (`~/.config/omarchy/hooks/post-update.d/drift`) that runs `drift notify` after `omarchy-update`. It shows the change as a diff, asks before writing, and is itself a transaction you can undo. `--dry-run` only shows the diff; run it again after a change and it says there is nothing to do.

## Bar widget

```sh
drift widget install --dry-run
drift widget install
drift widget sync
drift widget remove --yes
```

`widget` manages the [bar widget](widget.md). `install` copies the packaged plugin folder (from `/usr/share/omarchy-drift/plugin`, or `DRIFT_PLUGIN_DIR` or `--source <folder>`, or the repository's `plugin/` folder when running from a checkout) into `~/.config/omarchy/plugins/drift.status/`, runs `omarchy-shell shell rescanPlugins`, enables it with `omarchy plugin enable drift.status` and puts it on the bar with `omarchy bar put drift.status`; `--no-place` leaves the bar alone. Running it again changes no file. `sync` refreshes the copy after a package upgrade, but only when your copy still matches what Drift installed. `remove` asks, disables the widget and deletes your copy; the packaged files and the binary stay.

A file you edited is never overwritten without asking. `--yes` does not allow it: the command stops with exit `1` and tells you to use `--force`. Answering `y` at the prompt, or `--force`, replaces it after saving your edited copy as `.drift.status.bak.<time>` beside the plugin. Install and sync also refuse to write through a symlink. Without `omarchy` and `omarchy-shell` on `PATH` the commands say so and write nothing. `--dry-run` prints the files and the exact commands and does nothing.

## Other

```sh
drift --version
drift --help
```

With no arguments `drift` opens the [review screen](guide.md#the-review-screen). `drift --snapshot` prints one frame of it and does not take the terminal.
