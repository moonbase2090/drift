# Bar widget

The widget in `plugin/` has the id `drift.status`. It runs `drift status --bar` every five minutes and shows the drifted count in the Omarchy bar. The pill hides when nothing has drifted, its tooltip names the count, and a click opens Drift in the floating window (`omarchy-launch-or-focus-tui --app-id=TUI.float drift`), or focuses it if it is already open.

To use it, copy the `plugin/` folder into your Omarchy plugin directory as `~/.config/omarchy/plugins/drift.status`, then put it on the bar:

```text
omarchy plugin validate ./plugin
omarchy bar put drift.status
```

`omarchy plugin validate` checks the folder against Omarchy's manifest schema. To take it off the bar again, run `omarchy plugin disable drift.status` and delete the folder.

The count is the same one `drift status --bar` prints, so you can try the widget's input in a terminal:

```sh
drift status --bar
```
