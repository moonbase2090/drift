# Drift user guide

Drift compares your Hyprland and Omarchy configuration with the templates the installed `omarchy-settings` package ships, shows the difference file by file, and lets you keep your version, take upstream's, or merge the two. It never writes anything until you confirm a plan, and every write can be undone. Drift is built for Omarchy 4.5 and reads the templates from `/usr/share/omarchy`.

This guide covers what is in the program today. The [command reference](cli.md) lists every command, the [bar widget](widget.md) page covers the status-bar module, and [input-map.md](input-map.md) lists every key and mouse target.

## What Drift tracks

Omarchy copies starter files from the package into your home once; after that they are yours. Drift maps each template to its live file (`config/<path>` to `~/.config/<path>`) and puts every file in exactly one state:

| Status | Meaning |
|---|---|
| unchanged | Your file equals the template. |
| upstream changed | You have not touched the file, but the template moved on, or both changed and the changes merge cleanly (the row says `merges cleanly`). |
| local override | You changed the file and the template did not. Without a known base the row says `base unknown`. |
| conflict | Both changed and at least one hunk overlaps. |
| user-only | A file of yours in a directory that holds templates, with no template of its own. |
| missing | The template exists and your file does not (`deleted by you` when Drift knows you once had it). |

Every row carries a glyph and a word as well as a colour, so no state depends on colour alone. Only directories that contain at least one template are looked at; the rest of `~/.config` is never walked. Omarchy's `*.bak.<digits>` backups are grouped under one `backups (n)` row per directory. Symlinks are shown read-only, unreadable files are named, files that are not UTF-8 or are larger than 1 MiB show a hash instead of a diff, and files an app rewrites on its own (Chromium's `Default/Preferences`) carry a warning.

## How bases are found

A three-way comparison needs the **base**: the template as it was when you last looked. Drift looks for it in this order and tells you which one it used (the base provenance in `drift diff`, the tree summary and the diff header):

1. **reviewed**: the template as it was when you last resolved the file with Drift (keep mine records it for exactly this reason).
2. **snapshot**: the copy `drift snapshot --pre` takes of the template roots before each `omarchy-settings` upgrade.
3. **cache**: the previous package, read from pacman's package cache.
4. **none**: no base could be found.

Without a base, Drift compares your file with the template in two panes and cannot tell a change of yours from a change of upstream's. The row says `base unknown`, the diff opens as two panes, and merging is not offered for the file.

**The first update.** On a fresh install there is no snapshot and the package cache holds no previous package, so most files start at `none`. The first time you resolve a file Drift records the template as its reviewed base, so from then on that file has an exact base. Install the pacman hooks (see [Package snapshot](../README.md#package-snapshot)) before the next upgrade and every tracked file gets a snapshot base at the upgrade after that.

## The review screen

`drift` with no arguments opens the review screen. It is read-only: opening Drift never writes configuration. The header counts every status and doubles as the filter buttons, the tree lists the files that need review (it opens only as far as the first one), and a summary pane shows the selected file's template, base, hashes and state.

| Key | Action |
|---|---|
| `j` `k` / arrows | move |
| `h` `l` | collapse or expand |
| `Enter` | open the diff |
| `/` | search as you type |
| `1`–`6`, `0` | filter by status, clear the filter |
| `Space`, `a` | mark a file, mark everything in view |
| `o` `t` `m` | keep mine, take upstream, merge (act on the marked files or the selection) |
| `H` | history |
| `u` | undo the newest transaction |
| `r` | rescan |
| `?` | help for the current screen |
| `:` | command palette: every action by name |
| `q` | quit |

Everything on the screen can be done with the mouse as well: click a row, double-click to open, click a status count to filter, click the hint bar at the bottom. [input-map.md](input-map.md) lists every action with its keys and its mouse target. The screen follows the terminal when you resize it.

Two states replace the tree: **Nothing needs review** when no file drifted (local overrides and user-only files are not drift), and **Scan failed** with the reason spelled out. Both still offer rescan and history.

## The diff screen

`Enter` on a file opens its diff. The layout follows the window width: five columns (hunk list, base, ours, theirs, result) from 200 columns, ours and theirs side by side with the result underneath from 100, and one stacked column below that. `v` cycles the layouts. Added, removed and changed lines are tinted across the whole row and marked in the gutter with `+`, `-` and `~`, in the colours of your Omarchy theme.

| Key | Action |
|---|---|
| `n` `p` | next, previous hunk |
| `N` `P` | next, previous conflict that is still unresolved |
| `o` `t` `b` `e` | for the selected hunk: mine, theirs, both, or edit in `$EDITOR` |
| `Tab` / `Shift+Tab` | move focus between panes |
| `[` `]` | narrow or widen the focused pane (a drag on the divider does the same) |
| `w` | write: opens the plan dialog |
| `Esc` | back to the tree |

The result pane updates as you choose. A conflict hunk with no choice blocks writing and the screen says so.

## Resolving a conflict

1. Find the file. Press `4` to filter to conflicts, or `/` and type part of the path.
2. Press `Enter`. Press `N` to jump to the first unresolved conflict. The selected hunk is highlighted in every pane.
3. Read ours and theirs next to the base. Choose with `o` (keep your side of this hunk), `t` (take upstream's), `b` (both, yours first) or `e` (edit by hand in `$EDITOR`; Drift hands the terminal over and takes it back).
4. Press `N` again for the next unresolved conflict and repeat until none is left.
5. Press `w`. The plan dialog lists the file, the action, where the backup will go and a diff summary. Focus starts on Cancel. Press `d` to expand the diff, `Tab` to move focus to Confirm and `Enter` (or `y`) to write.
6. Drift writes the file atomically, after copying the original into its backup store. If the file is under `~/.config/hypr` Drift then runs `hyprctl configerrors` and offers to undo if Hyprland reports errors.

A file with no conflicts can skip the hunks: from the tree press `m` and read the merge in the plan dialog, or press `t` to take upstream outright, or `o` to keep yours (which writes nothing and records the template as reviewed, so the file reads as an acknowledged local override). `d` on a file you chose to keep shows how the template differs from yours, so you can see what you are declining.

## Safety model

- Opening Drift, scanning, `status`, `diff`, `scan` and `history` never write configuration or state.
- Every write goes through a plan that lists each file, its action, its backup location and a diff summary. Nothing is written until you confirm. Without a terminal, commands write nothing unless you pass `--yes`.
- Before a file is changed its original is copied into the backup store (`~/.local/state/drift/backups/<transaction>/`). A transaction records every file it touched with the hashes before and after, in `manifest.json`.
- Files are replaced atomically. Symlinks, files an app rewrites on its own, files without a template and files already equal to upstream are skipped with a reason.
- Every transaction can be undone, and an undo is a transaction that can be undone in turn. A file that changed after the transaction is left alone unless you name it with `--force`.
- Nothing is sent over the network. The only things Drift runs besides itself are `hyprctl configerrors` after a Hyprland write, `omarchy-notification-send` for the upgrade notice, and `$EDITOR` when you ask to edit a hunk.

## Recovering from backups

Drift keeps everything it needs to put a file back.

- **Undo a transaction.** `drift history` lists them, newest first. `drift undo` previews restoring the newest and asks; `drift undo <id>` picks another. See the [command reference](cli.md#history-and-undo). The same list is on the review screen under `H`.
- **A file changed since.** Undo names it and leaves it alone. `drift undo --force <path>` restores the backup over it.
- **By hand.** Each transaction is a plain directory: `~/.local/state/drift/backups/<transaction>/` holds the original files at their paths relative to your home (for example `.config/hypr/monitors.lua`) next to a `manifest.json` that names each action and its hashes. If Drift itself will not start, copy the file back with `cp`. Drift keeps the newest 100 transactions; the plan dialog lists any older ones it will prune with the write.
- **A configuration error after a Hyprland write.** Drift offers to undo straight away. If you decline, `drift undo --yes` does it later.
- **Base records.** Keep mine and take upstream record the template as the file's reviewed base; undo puts the previous base back, so an undone take upstream reads as upstream changed again.

## Uninstalling

1. Remove the hook Drift added to Omarchy: delete `~/.config/omarchy/hooks/post-update.d/drift`, and the `drift` row in `~/.config/omarchy/extensions/omarchy-menu.jsonc` (both were added by `drift integrate`, which showed you the change first).
2. Remove the bar widget: `drift widget remove` disables it and deletes your copy (do this before removing the binary).
3. Stop the upgrade notice: `systemctl --user disable --now drift-notify.path` if you enabled the packaged user unit.
4. Remove the program. With the package, `sudo pacman -Rns omarchy-drift` removes the binary, the pacman hooks, the user units and the packaged plugin copy. If you installed by hand, delete the binary and whatever you copied, including the two hooks in `/usr/share/libalpm/hooks/` (`10-drift-snapshot.hook`, `20-drift-updated.hook`) if you added them. Either way, `/var/lib/drift/` (the template snapshots and the upgrade stamp) belongs to neither the package nor your home, so removing the package leaves it behind: delete it yourself with `sudo rm -r /var/lib/drift`.
5. Optionally delete Drift's own data: `~/.local/state/drift` (backups, transaction records, notification setting) and `~/.local/share/drift` (recorded bases). Do this last: it is what undo reads, so after it Drift can no longer put files back.

None of this touches your configuration files; they stay exactly as they are now.
