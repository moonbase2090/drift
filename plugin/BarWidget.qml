import QtQuick
import Quickshell.Io
import qs.Ui

BarWidget {
  id: root

  property string countText: ""
  property string countTooltip: ""

  function showCount(raw) {
    var parsed = null
    try {
      parsed = JSON.parse(String(raw || ""))
    } catch (error) {
      parsed = null
    }
    var text = parsed && typeof parsed.text === "string" ? parsed.text.trim() : ""
    var tooltip = parsed && typeof parsed.tooltip === "string" ? parsed.tooltip : ""
    countText = text
    countTooltip = text === "" ? "" : tooltip
  }

  function refresh() {
    if (!statusProc.running)
      statusProc.running = true
  }

  visible: countText !== ""
  implicitWidth: button.implicitWidth
  implicitHeight: button.implicitHeight

  Process {
    id: statusProc
    command: ["drift", "status", "--bar"]
    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: root.showCount(text)
    }
    onExited: function(exitCode) {
      if (exitCode !== 0 && exitCode !== 1)
        root.showCount("")
    }
  }

  Timer {
    interval: 300000
    running: true
    repeat: true
    triggeredOnStart: true
    onTriggered: root.refresh()
  }

  WidgetButton {
    id: button
    anchors.fill: parent
    bar: root.bar
    text: root.countText
    tooltipText: root.countTooltip
    fontSize: 12
    horizontalMargin: 7.5
    onPressed: function(pressed) {
      if (pressed === Qt.LeftButton && root.bar)
        root.bar.run("omarchy-launch-or-focus-tui --app-id=TUI.float drift")
    }
  }
}
