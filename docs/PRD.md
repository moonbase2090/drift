# Drift: product requirements

Status: draft 0.2 · License: MIT · Target repository: `moonbase2090/drift` (public) · Stack: Rust, [Lunatui](https://github.com/moonbase2090/lunatui) v0.1.28 · Platform: Omarchy 4.5 (Arch Linux, Hyprland)

## 1. Problem

Omarchy ships starter configuration as templates inside its packages and copies them into the user's home once. The templates live in `/usr/share/omarchy/config/` (49 files at package version 4.5.0-3, owned by the `omarchy-settings` package) and map one-to-one onto `~/.config/` (for example `config/hypr/monitors.lua` becomes `~/.config/hypr/monitors.lua`, `config/omarchy/shell.json` becomes `~/.config/omarchy/shell.json`). After that copy the files belong to the user, and so do the tools that edit them: font and scale settings, the bar layout, and monitor scaling all rewrite those files in place.

When a package update changes a template, the user has no way to see it. Today's options are all-or-nothing:

- `omarchy-refresh-config <path>` copies the new template over the live file. It saves the old version as `<file>.bak.<epoch>` in the same directory and prints a plain `diff`. There is no merge, no restore command, and backups accumulate next to the config.
- `omarchy-refresh-hyprland` does that for seven Hyprland files in a row. `omarchy-refresh-shell` resets `shell.json` and the bar. On the reference laptop, 8 of the 49 templates already differ from the live file (font sizes, clock format, monitor scale, bar layout), so a refresh throws away real customization.
- Omarchy's own migrations edit files in place, and some first check whether the file still equals a known stock checksum. That is the same question Drift answers for every file, with the user in control.

Omarchy 4.5 already moved most behavior into package-owned defaults under `/usr/share/omarchy/default/`, which load live and never drift. What remains drift-prone is the set of copied starter files, and those are exactly the files users care about. Drift makes that set visible and lets the user take upstream changes selectively.

## 2. Goals

1. Show, for every shipped template, whether the live file is unchanged, a local override, behind an upstream change, or in conflict with one.
2. Offer a per-file and per-hunk 3-way resolution (keep mine, take upstream, merge) that is safer than a refresh: previewed, confirmed, backed up, undoable.
3. Be read-only by default. Opening Drift, scanning, and diffing never write to the user's config.
4. Work after an update and at any other time, including on a machine where the previous package version is gone.
5. Fit Omarchy: launch from the Omarchy menu, open as a floating Hyprland window, optionally notify after an update.
6. Stay fast and keyboard-complete, with first-class mouse support, within Lunatui's frame budget.

## 3. Non-goals

- Not a general dotfile manager, and not a replacement for chezmoi, stow, or git-tracked dotfiles. Symlinked targets are detected and left alone.
- Does not manage system files (`/etc`, boot, pacman, plymouth, login manager). Those refresh scripts need root; Drift never runs `sudo`.
- Does not run, wrap, or replace `omarchy-refresh-*`, `omarchy-update`, or migrations, and never restarts Omarchy components.
- Does not manage `/usr/share/omarchy/default/` (package-owned, loaded live) or themes.
- No network access in v1. No remote fetching of old templates.
- No format-aware (JSON, TOML, Lua) merging in v1. Merging is line-based.
- No rename tracking: a template that moved between versions appears as one removed and one added file.

## 4. Users and scenarios

**Everyday Omarchy user.** Runs `omarchy update`, sees a notification "3 configs drifted", clicks it, and lands in Drift. Two files are `upstream changed` and merge cleanly; they take upstream for both after reading the diff. One is a conflict in `hypr/looknfeel.lua`; they resolve it hunk by hunk and write once.

**Tinkerer with heavy overrides.** Wants to know which of their edits are still needed. Opens Drift any time, filters to `local override`, and reads each diff against the current template.

**Cautious user after a bad refresh.** Ran a refresh, lost a setting, and wants it back. Opens History, sees the backup, and undoes the transaction.

**Fresh install.** First update ever. The package cache holds no previous Omarchy package (see section 7.2), so Drift falls back to its recorded bases and tells the user plainly which files have an exact base and which are compared two-way.

**Scripted use.** `drift status --json` in a login script or bar widget.

## 5. Functional requirements

### Discovery and classification

- **FR-1** Enumerate templates from the installed package. Read `$OMARCHY_PATH` (default `/usr/share/omarchy`), the file list from the pacman local database for `omarchy-settings`, and verify each file's SHA-256 against that package's `mtree`. A template that fails verification is reported as a warning and still shown.
- **FR-2** Map each template to its live path through a data-driven root registry. v1 maps `config/<rel>` to `~/.config/<rel>` only. Secondary copy roots (`applications/*.desktop` to `~/.local/share/applications/`, `default/alacritty/Alacritty.desktop` to `~/.local/share/applications/`, and `default/hypr/toggles/flags.lua` to `~/.local/state/omarchy/toggles/hypr/flags.lua`) come later; they mirror the copy rules in `omarchy-refresh-applications` and `omarchy-refresh-hyprland`. Users can add roots in `~/.config/drift/config.toml`.
- **FR-3** Resolve three versions per file: base (B), ours (O, the live file), theirs (T, the installed template). Base resolution follows the precedence in section 7.2 and each file reports its base provenance (`reviewed`, `snapshot`, `cache`, or `none`).
- **FR-4** Classify each file into exactly one status:

| Status | Rule |
|---|---|
| unchanged | O equals T |
| upstream changed | O equals B, T differs from B; or both changed and the 3-way merge is clean (badge `merges cleanly`) |
| local override | O differs from B, T equals B; also O differs from T when no base exists (badge `base unknown`) |
| conflict | both changed and the 3-way merge has at least one overlapping hunk |
| user-only | file in a tracked directory with no template |
| missing | template exists, live file does not (badge `deleted by you` when B exists) |

- **FR-5** Limit `user-only` detection to files directly inside directories that hold at least one template, never recursing into directories without templates (so `~/.config/chromium` is not walked). Omarchy's `*.bak.<digits>` files are grouped under a collapsed `backups (n)` node per directory.
- **FR-6** Track the last reviewed package version in Drift state. When the installed `omarchy-settings` version differs, show an "update pending review" banner with the version change.
- **FR-7** Handle edge cases explicitly: symlink targets (shown read-only with the link destination), unreadable files, non-UTF-8 content (hash-only, no diff), files over 1 MiB (hash-only), and app-managed files marked `volatile` in the registry (v1: `chromium/Default/Preferences`), which show status with an "app rewrites this file" warning.
- **FR-8** Rescan on request (`r`) and automatically when watched files change, debounced to 200 ms.

### Viewing and resolving

- **FR-9** Tree view of paths with a status glyph, a text label, and a color per file (never color alone), a header with counts per status, status filters, and incremental search.
- **FR-10** Diff view showing base, ours, theirs, and a live result preview. Layouts adapt to width: four-pane at 160 columns and up, two-pane with a result strip at 100 and up, stacked unified view below that. Changed spans inside paired lines are highlighted.
- **FR-11** Hunk navigation and per-hunk choice: mine, theirs, both (mine then theirs), or edit in `$EDITOR`. The result preview updates immediately. Unresolved conflict hunks block writing.
- **FR-12** File-level actions: **keep mine** (writes nothing to the config; records T as reviewed so the file reads as an acknowledged local override), **take upstream** (replace with T), **merge** (open the hunk resolver; if the merge is clean, show the result for confirmation). Every file-level action also exists as a bulk action on marked files.
- **FR-13** Every write goes through a plan screen listing each file, the action, the backup location, and a before/after diff summary. Nothing is written until the user confirms.
- **FR-14** Undo the last transaction (`u`) or any earlier one from the History view. Undo is itself a transaction and can be undone.
- **FR-15** `i` ignores a file (stored in Drift state, reversible); ignored files are listed under a collapsed `ignored (n)` node.

### CLI and integration

- **FR-16** The `drift` binary exposes: `drift` (TUI), `scan`, `status [--json|--bar]`, `diff <path>`, `resolve <path> --keep|--upstream|--merge [--yes]`, `undo [txn]`, `history`, `snapshot` (used by the pacman hook), `integrate`, and `notify`. Exit codes: 0 no drift, 1 drift found, 2 error. All read commands are strictly read-only.
- **FR-17** `drift integrate` shows a diff and, on confirmation, adds an Update submenu row to `~/.config/omarchy/extensions/omarchy-menu.jsonc` and a `post-update.d/drift` hook under `~/.config/omarchy/hooks/`. Both writes use the same backup and undo path as config writes.
- **FR-18** Launch as a floating window using the app id `TUI.float`, which Omarchy's window rules already float and center, via `omarchy-launch-or-focus-tui --app-id=TUI.float drift`.
- **FR-19** After a package update, send "N configs drifted" through `omarchy-notification-send`, with a click action that opens Drift. Notifications are on by default. `drift notify off` stores the disable in Drift's state, and `drift notify on` turns them back on. Notify skips while another process holds the Omarchy update lock, and while the pacman database lock exists. The post-update hook inherits the Omarchy lock and still sends, and it still checks the pacman lock. An Omarchy lock file left behind does not suppress a later notify. Pacman removes its database lock when the transaction ends. A `bar-widget` plugin shows the count on the bar.
- **FR-20** A pacman hook snapshots the outgoing templates before every `omarchy-settings` upgrade (section 7.2).

## 6. UX and keybindings

### 6.1 Screens

1. **Tree** (home). Left: path tree. Right: file summary (status, base provenance, last reviewed version, hashes, size, mtime). Bottom: key hint bar built from the active keymap.
2. **Diff**. Hunk list on the left edge, panes as in FR-10, result preview at the bottom or right.
3. **Plan** (confirm dialog). Default focus is Cancel; the destructive confirm button is distinct.
4. **History**. Transactions with time, package version, files, and an undo action.
5. **Help** and **command palette** (`?` and `:`), both listing every action with its binding.

### 6.2 Keys

Every action has a keyboard binding. Mouse is first-class and never required.

| Context | Keys | Mouse |
|---|---|---|
| Global | `q` quit, `?` help, `:` command palette, `r` rescan, `u` undo last, `H` history, `Esc` back | Click hint-bar items; click header status counts to filter |
| Tree | `j`/`k` or arrows move, `h`/`l` collapse/expand, `g`/`G` top/bottom, `Enter` open diff, `/` search, `1`–`6` filter by status, `0` clear filter, `Space` mark, `a` mark all in filter, `i` ignore | Click row selects, double-click opens, wheel scrolls, click caret toggles, click checkbox marks |
| Per file (Tree or Diff) | `o` keep mine, `t` take upstream, `m` merge | Action buttons in the footer |
| Diff | `Tab`/`Shift-Tab` focus pane, `n`/`p` next/previous hunk, `N`/`P` next/previous unresolved conflict, `o`/`t`/`b`/`e` mine/theirs/both/edit for the hunk, `v` cycle layout, `w` write (opens Plan) | Click hunk selects, click hunk-action chips, wheel scrolls, drag the pane divider to resize |
| Plan | `y` confirm, `n`/`Esc` cancel, `Tab` between buttons, `d` expand a file's diff | Click Confirm or Cancel |
| History | `j`/`k`, `Enter` details, `u` undo selected | Click row, click Undo |

Layouts and widgets must remain usable at 80×24 (stacked unified view). The window Omarchy floats for `TUI.float` is 875×600. Stock Foot, with JetBrains Mono at 9pt and 14px of padding, measures 121×33 cells there, in the two-pane range, and users can maximize it for four panes. Drift does not ship its own window rule.

### 6.3 Lunatui widgets used

`Tree` and `TreeState` (path tree), `DiffView` (unified and side-by-side rendering of the two-way pieces), `CodeView` (base, ours, theirs, and result panes with line numbers and gutter marks), `SplitTree` with `split-keys` (draggable and keyboard-resizable panes), `Scrollbar` and `ScrollView`, `StatusBar`, `KeyHintBar`, `Dialog` (confirm and alert) with `Popup`, `ToastStack` for transient results, `SearchBox` (tree filter), `Breadcrumbs` (current path), `Tabs` (screen switch), `Table` (History), `Button`, `StatusDot` and `Badge` (status and base provenance), `Tooltip` (status explanations), `Toggle` and `RadioGroup` (settings), `CommandPalette` (opt-in Lunatui feature, enabled in Drift). `DiffView` parses a unified diff for one file and has no 3-way mode, so the four-pane view is composed by Drift from `CodeView` panes sharing one scroll offset. The 3-way view is built in Drift first and may be upstreamed to Lunatui later.

### 6.4 Theme

Follow the active Omarchy theme palette. If that palette is unavailable, fall back to Lunatui's built-in dark and light themes.

## 7. Architecture

### 7.1 Crates

| Crate | Responsibility | IO |
|---|---|---|
| `drift-core` | Types, root registry, status classification, hashing, line-based 3-way merge (diff3 over a diff library), plan/transaction data types | none, pure |
| `drift-source` | Providers for B, O, T: pacman local database and `pacman.log`, pacman cache reader (tar and zstd), snapshot store, reviewed-base store | read-only |
| `drift-apply` | The only crate allowed to write the filesystem: backup, atomic write, transaction manifest, undo, locking | write |
| `drift-tui` | Lunatui app: state, keymap, mouse hit maps, screens | none directly, talks to a worker |
| `drift-cli` | `drift` binary, subcommands, JSON output, notifications | via crates above |
| `drift-fixtures` | Golden fixture trees and a builder used by every test crate | test only |

A background worker thread runs scans and diffs and sends finished results to the UI over a channel. The render path only reads precomputed state.

Repository layout also holds `plugin/` (manifest and QML for the bar widget), `packaging/` (pacman hook, systemd user path unit, PKGBUILD), and `docs/`. The plugin directory contains no symlinks, because Omarchy's `omarchy plugin validate` rejects them. v1 ships the plugin from this same repository; whether to publish it from a separate small repository remains pending.

### 7.2 Where base, ours, and theirs come from

- **Ours (O):** the live file under the mapped path. Read only.
- **Theirs (T):** the file under `$OMARCHY_PATH/config` in the installed package, verified against the local database `mtree`. If a template is absent there, `/etc/skel/.config/<rel>` carries the same file for `config/` entries; Omarchy's own migrations use that fallback. Other registry roots are out of v1.
- **Base (B):** the merge base is "the template the user last reviewed or received". Sources in order:
  1. **Reviewed base**: Drift's own store, `~/.local/share/drift/bases/<rel>`. Seeded on the first run for every file where O equals T (verified stock), and updated to T after any resolution, including keep mine. This is what makes "keep mine" stick and gives exact bases across several updates.
  2. **Snapshot**: `/var/lib/drift/snapshots/<pkgver>/` written by the pacman hook before each `omarchy-settings` upgrade. Exact, offline, independent of cache cleaning.
  3. **Pacman cache**: `/var/cache/pacman/pkg/omarchy-settings-<prev>-x86_64.pkg.tar.zst`. The previous version comes from `pacman.log` (`upgraded omarchy-settings (A -> B)`) and Drift's recorded last-reviewed version. Extracted files are checked against the archive's own `.MTREE`.
  4. **None**: Drift degrades to two-way (ours versus theirs), labels the file `base unknown`, and offers only keep mine and take upstream.

  Why a cache-only design is not enough: `omarchy-update` runs `paccache -rk2` before upgrading, so at most two versions of each package survive. The reference laptop was installed from an image, and its cache holds no `omarchy` or `omarchy-settings` archive at all, so the very first update has no cached previous version. The `mtree` in the local database gives hashes only, and it is removed when the package is upgraded.
- The hook is a PreTransaction hook with `Operation = Upgrade`, `Type = Package`, `Target = omarchy-settings`, running `drift snapshot --pre`. It copies registry roots from `$OMARCHY_PATH` into the snapshot store (a few hundred KB) and records the version. Omarchy's own `10-omarchy-hyprland-reload-pause.hook` is the precedent for a hook on this package. The hook is the only component that writes as root, and it writes only under `/var/lib/drift`.
- Btrfs snapshots taken by `omarchy-snapshot create` contain the previous tree too, but they are root-readable only; reading them is deferred past v1, with no root helper.

### 7.3 Omarchy plugin integration

Omarchy plugins are git repositories with a `manifest.json` at the root (`schemaVersion: 1`, `id`, `name`, `version`, `kinds`, `entryPoints`). Kinds are `bar-widget`, `panel`, `overlay`, `menu`, `service`, and `bar`. Third-party ids cannot use the reserved `omarchy.` prefix, and third-party plugins run unsandboxed in the shell process. First-party example: `omarchy.reminders` is a single-kind `overlay` manifest. Third-party example: the Superwhisper plugin is a `bar-widget` with a `barWidget` block (display name, category, `defaultSection`, settings schema).

Drift ships:

- A **`bar-widget` plugin**, id `drift.status`, kinds `["bar-widget"]`, in `plugin/`. The shell loads a third-party widget unsandboxed, and the widget can run a program and read its output, so it polls `drift status --bar` itself every five minutes. It shows the JSON `text` count and hides when `text` is empty. A click opens Drift. `omarchy plugin validate ./plugin` checks the folder. `omarchy plugin add` expects the manifest at the repository root, so this folder is placed in the user plugin directory until the widget has its own repository. Place it with `omarchy bar put drift.status`.
- **Menu entry** through the user extension file `~/.config/omarchy/extensions/omarchy-menu.jsonc` (an `update.drift` row whose `action` runs the floating launcher).
- **Post-update hook** `~/.config/omarchy/hooks/post-update.d/drift` running `drift notify`. `omarchy-hook post-update` runs only at the end of `omarchy-update`, so a systemd user path unit (packaged) also watches the hook-written state file in `/var/lib/drift` and triggers `drift notify` after updates made with plain `pacman -Syu`.

The TUI and CLI do not depend on the shell plugin; the plugin is an optional front door.

## 8. Safety model

1. **Read-only by default.** Scanning, diffing, and viewing history never write to the user's config, to Omarchy files, or to anything outside Drift's own state directories. The one automatic write is Drift's own bookkeeping under `~/.local/share/drift` and `~/.local/state/drift` (seeding reviewed bases on first run, recording the last reviewed version); it is skipped silently if the directories are not writable. Write code exists only in `drift-apply`, and a CI check fails if filesystem-mutating calls appear in any other crate.
2. **Explicit and confirmed.** Every write path goes through a Plan, in the TUI via the dialog and in the CLI via interactive confirm or `--yes`. `--dry-run` prints the plan.
3. **Backup first.** Before replacing a file, Drift copies the original to `~/.local/state/drift/backups/<txn>/<rel>` with a JSON manifest (time, package version, per-file action, SHA-256 before and after, mode). The last 100 transactions are kept; older ones are pruned, and that pruning is shown in the plan.
4. **Atomic writes.** Write a temp file in the same directory, fsync, preserve mode, rename over the target. A failed step leaves the original untouched and the transaction reports which files were not applied.
5. **No stale writes.** Re-hash O immediately before writing and abort that file if it changed since the plan was made.
6. **Undo.** Restores backed-up bytes if the file still has the post-write hash; otherwise warns and offers restore-anyway, which takes a new backup first. Files Drift created are removed only if unchanged.
7. **Concurrency guards.** Take a Drift lock; refuse writes while `omarchy-update` holds `$XDG_RUNTIME_DIR/omarchy-update.lock` or `/var/lib/pacman/db.lck` exists.
8. **Hyprland reload check.** After writing under `~/.config/hypr/`, run `hyprctl configerrors` read-only and, if it reports errors, prompt to undo.
9. **Scope.** Drift writes only the user's mapped targets, its own state directories, and the two integration files from `drift integrate`. It never executes Omarchy scripts, runs `sudo`, or touches `/usr`, `/etc`, or the package cache.
10. **Symlinks and volatile files** are never written through without an explicit override per file.

## 9. Performance

- Frame ceiling: Lunatui's **8 ms worst case** per frame (paint, diff, encode), measured at 80×24 and 200×60 on the reference laptop (4 cores), with the p99 asserted in tests, not only the median.
- No filesystem or diff work on the render path. Scans and diffs run on the worker; the UI shows a spinner row for pending items.
- Cold start to first frame under 150 ms with the tree populated for 50 files. Full scan of 5,000 template files under 1 s on the reference laptop.
- Hunk layout and highlighted spans are computed once per file and cached by hash; scrolling reuses them. Cap diffed files at 1 MiB.
- Idle CPU stays near zero: no polling loop; rescans come from file watching, and the bar plugin polls at most every five minutes.

## 10. Testing and acceptance

The approach is verification, not mutation or complexity scoring. Tests are fixtures plus assertions about observable behavior.

### 10.1 Test suites

- **Golden fixture trees.** `drift-fixtures` builds temp `HOME` and `OMARCHY_PATH` trees from case directories with `base/`, `ours/`, `theirs/`, and expected output. Cases cover the full presence matrix (B, O, T each absent, same, or changed) and every status in FR-4.
- **Merge correctness.** Table tests for clean merge, overlapping conflict, adjacent edits, identical edits on both sides, edits at start and end of file, CRLF, missing trailing newline, empty files, whitespace-only changes, and large files. Invariants: merging with O equal to B yields T; with T equal to B yields O; the result of a clean merge contains every non-conflicting change from both sides.
- **Backup and undo.** Apply any sequence of actions, undo all, and require a byte- and mode-identical tree. Cover failure mid-transaction, stale-file abort, and undo after the file changed.
- **No writes in read-only mode.** Hash the full fixture `HOME` tree, excluding Drift's own state directories, plus the fixture `OMARCHY_PATH`, (content, mode, mtime) before and after every read command and TUI script; require equality. Run read commands under a read-only bind mount where available. The CI check from section 8, item 1, greps for mutating calls outside `drift-apply`.
- **Source resolution.** Fixtures for each base source and for the degradation path to `none`, including a cache archive whose `.MTREE` does not match.
- **TUI.** `lunatui-test` snapshots for each screen at 80×24, 120×36, and 200×60, with scripted key and mouse sequences, plus a rendering budget test asserting the 8 ms frame ceiling.
- **Live check on the reference laptop (Omarchy 4.5.0-3).** `drift status` must agree with a plain `cmp` over `config/` versus `~/.config` (currently 41 unchanged and 8 differing), with the machine untouched.

### 10.2 Acceptance criteria (release gates)

1. Each FR has at least one automated test or a documented manual check that passes.
2. Status classification matches the table in FR-4 for all fixture cases.
3. Round-trip undo is byte-identical in all backup tests.
4. Zero writes in every read-only test; zero mutating calls outside `drift-apply`.
5. 8 ms frame ceiling holds in the budget test.
6. Every action in section 6.2 is reachable by keyboard, and the same actions are reachable by mouse without requiring it.
7. Features ship enabled: no default-off feature flags or settings in Cargo features or config defaults.
8. No code comments by default, following Lunatui's convention; a lint script enforces it.
9. First update on an image-installed system (no cached package) still yields exact bases through the snapshot hook or reviewed bases, and says so in the UI.

## 11. Milestones

| Milestone | Scope | Exit criteria |
|---|---|---|
| **M0 Scaffold** | Workspace, crates, CI, lint and no-comment checks, fixture builder, Lunatui hello screen | CI green; empty TUI runs and quits; write-check script active |
| **M1 Read-only scan and tree** | Package and database reading, registry, base/ours/theirs providers, classification, tree screen, `scan` and `status` | FR-1 to FR-9 pass; live check matches `cmp`; no writes |
| **M2 Diff view** | 3-way merge engine, hunk model, diff screen in all layouts, `diff` command | FR-10 passes; merge correctness suite passes |
| **M3 Resolve actions, backup and undo** | Plan, transactions, atomic apply, undo, history, `resolve` and `undo` commands, Hyprland error check | FR-11 to FR-15 pass; backup/undo suite passes |
| **M4 Omarchy integration and update notification** | Pacman hook and snapshots, `integrate`, menu entry, floating launch, post-update hook, path unit, notification, bar-widget plugin | FR-17 to FR-20 pass; first-update scenario acceptance passes |
| **M5 Polish and packaging** | Help and palette polish, empty and error states, docs, AUR package, release automation | All gates in 10.2 pass; AUR package installs and uninstalls cleanly |
| **M5 stretch (post-v1)** | Format-aware merge for JSON and TOML | Optional; not a v1 release gate |

## 12. Constraints that apply to all work

- Frame time ceiling of 8 ms worst case.
- Keyboard-complete, mouse first-class, mouse never forced.
- No code comments by default, matching Lunatui.
- Features ship on by default; no default-off flags.
- Verification-focused testing as in section 10; no mutation testing and no complexity-score gates.
- Public repository: no personal names, no tracker identifiers, and no references to tooling used to produce the code, anywhere in docs, commits, or code.

## 13. Decisions

1. **Reading btrfs snapshots** as a base source needs root. **Decided:** deferred past v1. No root helper in this version.
2. **Clean-mergeable files.** **Decided:** a file where both sides changed but the merge is clean stays `upstream changed`, with a `merges cleanly` badge, as in FR-4.
3. **Remote bases.** **Decided:** no remote fetching of old templates in v1. Network stays off, as in section 3.
4. **Format-aware merge** for `shell.json` and TOML files. **Decided:** post-v1. Tracked as a milestone 5 stretch ticket; v1 merging stays line-based.
5. **3-way diff widget.** **Decided:** build it inside Drift first, then upstream to Lunatui later.
6. **Backup retention.** **Decided:** keep the last 100 transactions. Pruning is shown in the plan.
7. **Theme.** **Decided:** follow the active Omarchy theme palette, falling back to Lunatui's built-in dark and light themes.
8. **Floating window size.** **Decided:** 875×600 is enough. Stock Foot, with JetBrains Mono at 9pt and 14px of padding, measures 121×33 cells in that window, which uses the two-pane layout. Drift does not ship its own window rule. Maximizing the window reaches four panes.
9. **Registry breadth.** **Decided:** v1 covers `config/` templates mapped to `~/.config` only. Secondary copy roots (applications, the Alacritty desktop file, Hyprland toggle flags) come later.
10. **License and name.** **Decided:** MIT, to match Lunatui. Whether `drift` is free as a package name on the AUR is checked in the packaging ticket.
11. **Plugin install path.** **Pending:** the owner's answer on whether the plugin should live in a separate small repository. For now, one repository.
12. **Third-party plugin capabilities.** **Decided:** a third-party bar widget runs `drift status --bar` itself. The shell loads it unsandboxed, so it can start the program and read the output. The bar `run` helper only starts a command and does not return output, so the widget does not use it for the count. The command bar module is not the implementation.
