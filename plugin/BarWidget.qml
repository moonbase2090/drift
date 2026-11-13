import QtQuick
import Quickshell.Io
import qs.Ui
import "DriftStatus.js" as Status

BarWidget {
  id: root

  property string countText: ""
  property string countTooltip: ""
  property bool shown: false
  property bool clickable: false

  function apply(view) {
    countText = view.text
    countTooltip = view.visible ? view.tooltip : ""
    shown = view.visible
    clickable = view.clickable
  }

  function refresh() {
    if (!versionProc.running && !statusProc.running)
      versionProc.running = true
  }

  visible: shown
  implicitWidth: button.implicitWidth
  implicitHeight: button.implicitHeight

  Process {
    id: versionProc
    command: Status.VERSION_COMMAND
    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: {
        var check = Status.checkBinary(text)
        if (check.ok)
          statusProc.running = true
        else
          root.apply(check.view)
      }
    }
  }

  Process {
    id: statusProc
    command: Status.STATUS_COMMAND
    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: root.apply(Status.fromStatus(text))
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
      if (pressed === Qt.LeftButton && root.clickable && root.bar)
        root.bar.run("omarchy-launch-or-focus-tui --app-id=TUI.float drift")
    }
  }
}
