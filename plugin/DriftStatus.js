.pragma library

var WIDGET_VERSION = "0.1.1"
var VERSION_COMMAND = ["sh", "-c", "drift --version 2>/dev/null; echo \"exit:$?\""]
var STATUS_COMMAND = ["sh", "-c", "drift status --bar 2>/dev/null; echo \"exit:$?\""]

var MISSING_TEXT = "Drift missing"
var MISSING_TOOLTIP = "Drift is not installed or is not on PATH. Install it with the signed pacman repository steps in docs/install.md. This widget never installs anything."
var MISMATCH_TEXT = "Drift mismatch"
var ERROR_TEXT = "Drift error"

function splitExit(raw) {
  var text = String(raw || "")
  var found = /(?:^|\n)exit:(-?\d+)\s*$/.exec(text)
  if (!found)
    return { code: -1, output: text }
  return { code: Number(found[1]), output: text.slice(0, found.index) }
}

function parseVersion(output) {
  var found = /^drift\s+(\d+)\.(\d+)\.(\d+)/.exec(String(output || "").trim())
  if (!found)
    return null
  return { major: Number(found[1]), minor: Number(found[2]), patch: Number(found[3]), text: found[1] + "." + found[2] + "." + found[3] }
}

function compatible(widget, binary) {
  return widget.major === binary.major && widget.minor === binary.minor
}

function hidden() {
  return { visible: false, text: "", tooltip: "", clickable: false }
}

function notice(text, tooltip) {
  return { visible: true, text: text, tooltip: tooltip, clickable: false }
}

function checkBinary(raw) {
  var done = splitExit(raw)
  var exitCode = done.code
  var output = done.output
  if (exitCode === 127 || exitCode === 126 || exitCode < 0)
    return { ok: false, view: notice(MISSING_TEXT, MISSING_TOOLTIP) }
  var widget = parseVersion("drift " + WIDGET_VERSION)
  var binary = exitCode === 0 ? parseVersion(output) : null
  if (!binary) {
    return {
      ok: false,
      view: notice(MISMATCH_TEXT, "This widget is version " + WIDGET_VERSION + " but `drift --version` gave no readable version (exit " + exitCode + "). Update the omarchy-drift package, then run drift widget sync.")
    }
  }
  if (!compatible(widget, binary)) {
    var older = binary.major < widget.major || (binary.major === widget.major && binary.minor < widget.minor)
    var advice = older
      ? "The drift binary is older than this widget. Update the omarchy-drift package."
      : "The drift binary is newer than this widget. Run drift widget sync to refresh the widget."
    return {
      ok: false,
      view: notice(MISMATCH_TEXT, "This widget is version " + WIDGET_VERSION + " but drift is " + binary.text + ". " + advice)
    }
  }
  return { ok: true, view: hidden() }
}

function fromStatus(rawOutput) {
  var done = splitExit(rawOutput)
  var exitCode = done.code
  var raw = done.output
  if (exitCode !== 0 && exitCode !== 1)
    return notice(ERROR_TEXT, "drift status --bar failed (exit " + exitCode + "). Run it in a terminal to see why.")
  var parsed = null
  try {
    parsed = JSON.parse(String(raw || ""))
  } catch (error) {
    parsed = null
  }
  var text = parsed && typeof parsed.text === "string" ? parsed.text.trim() : ""
  var tooltip = parsed && typeof parsed.tooltip === "string" ? parsed.tooltip : ""
  if (text === "")
    return hidden()
  return { visible: true, text: text, tooltip: tooltip, clickable: true }
}
