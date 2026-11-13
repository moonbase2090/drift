import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import vm from "node:vm";
import { fileURLToPath } from "node:url";

const root = path.join(path.dirname(fileURLToPath(import.meta.url)), "..");
const source = fs.readFileSync(path.join(root, "plugin/DriftStatus.js"), "utf8");
const context = {};
vm.runInNewContext(source.replace(/^\.pragma library\s*$/m, ""), context);
const model = context;

function stubDir(files) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "drift-stub-"));
  for (const [name, body] of Object.entries(files)) {
    const file = path.join(dir, name);
    fs.writeFileSync(file, `#!/bin/sh\n${body}\n`);
    fs.chmodSync(file, 0o755);
  }
  return dir;
}

function probe(command, dir) {
  const run = spawnSync("/bin/sh", ["-c", command[2]], {
    env: { PATH: dir },
    encoding: "utf8",
  });
  return run.stdout;
}


function binary(version, status = 'echo \'{"text":"","tooltip":""}\'; exit 0') {
  return stubDir({
    drift: `case "$1" in --version) echo "drift ${version}";; status) ${status};; esac`,
  });
}

test("a missing binary shows the install hint and nothing runs it", () => {
  const empty = stubDir({});
  const check = model.checkBinary(probe(model.VERSION_COMMAND, empty));
  assert.equal(check.ok, false);
  assert.equal(check.view.visible, true);
  assert.equal(check.view.text, "Drift missing");
  assert.match(check.view.tooltip, /not installed/);
  assert.match(check.view.tooltip, /docs\/install\.md/);
  assert.equal(check.view.clickable, false);
  assert.doesNotMatch(check.view.tooltip, /pkg aur|pacman -S/);
});

test("a matching binary passes the check and hides until something drifted", () => {
  const dir = binary("0.1.7");
  const check = model.checkBinary(probe(model.VERSION_COMMAND, dir));
  assert.equal(check.ok, true);
  const empty = model.fromStatus(probe(model.STATUS_COMMAND, dir));
  assert.deepEqual({ ...empty }, { visible: false, text: "", tooltip: "", clickable: false });
});

test("a drifted count is shown and clickable", () => {
  const dir = binary("0.1.0", 'echo \'{"text":"3","tooltip":"3 configs drifted","class":"drifted"}\'; exit 1');
  assert.equal(model.checkBinary(probe(model.VERSION_COMMAND, dir)).ok, true);
  const view = model.fromStatus(probe(model.STATUS_COMMAND, dir));
  assert.equal(view.visible, true);
  assert.equal(view.text, "3");
  assert.equal(view.tooltip, "3 configs drifted");
  assert.equal(view.clickable, true);
});

test("a binary that is too old or too new shows a mismatch message", () => {
  for (const [version, advice] of [
    ["0.0.9", /older than this widget/],
    ["0.2.0", /newer than this widget/],
    ["1.1.0", /newer than this widget/],
  ]) {
    const check = model.checkBinary(probe(model.VERSION_COMMAND, binary(version)));
    assert.equal(check.ok, false, version);
    assert.equal(check.view.text, "Drift mismatch");
    assert.match(check.view.tooltip, advice);
    assert.ok(check.view.tooltip.includes(version), version);
    assert.equal(check.view.clickable, false);
  }
});

test("a binary whose version cannot be read is a mismatch, not silence", () => {
  const garbled = stubDir({ drift: 'echo "hello"' });
  const check = model.checkBinary(probe(model.VERSION_COMMAND, garbled));
  assert.equal(check.ok, false);
  assert.equal(check.view.text, "Drift mismatch");
  const failing = stubDir({ drift: "exit 2" });
  const failed = model.checkBinary(probe(model.VERSION_COMMAND, failing));
  assert.equal(failed.view.text, "Drift mismatch");
});

test("a failing status run is shown instead of hiding the widget", () => {
  const dir = binary("0.1.0", "exit 2");
  const view = model.fromStatus(probe(model.STATUS_COMMAND, dir));
  assert.equal(view.visible, true);
  assert.equal(view.text, "Drift error");
  assert.match(view.tooltip, /exit 2/);
  const lost = model.fromStatus("");
  assert.equal(lost.text, "Drift error");
});

test("only the minor version has to match, not the patch", () => {
  for (const version of ["0.1.0", "0.1.9"]) {
    assert.equal(model.checkBinary(probe(model.VERSION_COMMAND, binary(version))).ok, true);
  }
});

test("the widget version is the manifest version and the package version", () => {
  const manifest = JSON.parse(fs.readFileSync(path.join(root, "plugin/manifest.json"), "utf8"));
  assert.equal(model.WIDGET_VERSION, manifest.version);
  const cargo = fs.readFileSync(path.join(root, "Cargo.toml"), "utf8");
  const found = /\[workspace\.package\][^[]*?version\s*=\s*"([^"]+)"/s.exec(cargo);
  assert.equal(model.WIDGET_VERSION, found[1]);
});

test("the widget never installs anything and only opens Drift when it works", () => {
  const qml = fs.readFileSync(path.join(root, "plugin/BarWidget.qml"), "utf8");
  assert.doesNotMatch(qml + source, /pacman -S|pkg aur add|\bparu\b|\byay\b/);
  assert.match(qml, /root\.clickable/);
  assert.match(qml, /Status\.checkBinary/);
  assert.match(qml, /Status\.fromStatus/);
});
