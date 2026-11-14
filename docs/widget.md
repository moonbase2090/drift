# Bar widget

The widget in `plugin/` has the id `drift.status`. It runs `drift status --bar` every five minutes and shows the drifted count in the Omarchy bar. The pill hides when nothing has drifted, its tooltip names the count, and a click opens Drift in the floating window (`omarchy-launch-or-focus-tui --app-id=TUI.float drift`), or focuses it if it is already open.

To use it:

```sh
drift widget install --dry-run
drift widget install
```

`install` copies the `plugin/` folder to `~/.config/omarchy/plugins/drift.status`, rescans the plugins, enables the widget and puts it on the bar. `--dry-run` shows the files and the commands first, and `--no-place` skips the bar. After a package upgrade `drift widget sync` refreshes the copy; if you edited it, Drift asks (`--force` to replace it, with a backup) instead of overwriting. `drift widget remove` disables the widget and deletes your copy. All of this is in the [command reference](cli.md#bar-widget).

If `drift` is missing, the widget says "Drift missing" with a pointer to the install steps; if its version does not match the widget it says "Drift mismatch" and what to update; if `drift status --bar` fails it says "Drift error". It never installs anything.

Doing it by hand works too: copy the folder, then run `omarchy plugin validate` on it and put it on the bar.

```text
omarchy plugin validate ./plugin
omarchy bar put drift.status
```

The count is the same one `drift status --bar` prints, so you can try the widget's input in a terminal:

```sh
drift status --bar
```
