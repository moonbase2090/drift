#!/usr/bin/env python3
"""Replay PTY test guards in an idle, clean worktree; always restore mutated sources."""
import argparse
import fcntl
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]
RECIPES = [
    ("theme-mode", "crates/drift-tui/src/diff_palette.rs",
     ".map(|source| variant_from_colors(&source))", ".map(|_| Variant::Dark)",
     "theme::dark_and_light_theme_switches_repaint_legible_statuses_without_resetting_selection"),
    ("theme-hues", "crates/drift-tui/src/screen.rs",
     "screen.with_palette(crate::diff_palette::DiffPalette::active())",
     "screen.with_palette(crate::diff_palette::DiffPalette::fallback())",
     "theme::the_diff_uses_omarchy_hues_and_repaints_its_cached_frame_when_mode_changes"),
    ("theme-cache", "crates/drift-tui/src/diff.rs",
     "                && painted.variant == variant\n", "",
     "theme::the_diff_uses_omarchy_hues_and_repaints_its_cached_frame_when_mode_changes"),
    ("widget-enable", "crates/drift-apply/src/widget.rs",
     '    if enable {\n        steps.push(cmd(&["omarchy", "plugin", "enable", id]));',
     '    if false && enable {\n        steps.push(cmd(&["omarchy", "plugin", "enable", id]));',
     "widget::widget_install_copies_records_enables_and_places_the_plugin"),
    ("widget-edit-guard", "crates/drift-apply/src/widget.rs",
     "if edited == 0 || options.force {", "if true || edited == 0 || options.force {",
     "widget::widget_sync_updates_shipped_files_and_yes_cannot_overwrite_local_edits_but_force_backs_them_up"),
    ("widget-remove", "crates/drift-apply/src/widget.rs",
     "fs::remove_dir_all(&dest_dir)", "fs::metadata(&dest_dir).map(|_| ())",
     "widget::widget_remove_decline_keeps_the_copy_then_yes_removes_it_and_preserves_edits"),
    ("notify-delivery", "crates/drift-cli/src/notify.rs",
     "                deliver(&notice)?;", "                let _ = &notice;",
     "notify::notify_delivers_singular_and_plural_counts_with_the_click_command"),
    ("notify-disabled", "crates/drift-cli/src/notify.rs",
     "            if !enabled {", "            if false && !enabled {",
     "notify::notify_on_and_off_persist_across_processes_and_control_delivery"),
    ("notify-lock", "crates/drift-cli/src/notify.rs",
     "            if update_running(omarchy_lock, pacman_lock) {",
     "            if false && update_running(omarchy_lock, pacman_lock) {",
     "notify::notify_skips_zero_drift_and_real_update_locks_but_allows_unlocked_leftovers"),
]


def git(*args):
    return subprocess.check_output(["git", *args], cwd=ROOT, text=True).strip()


def run(output, name, args):
    result = subprocess.run(
        ["cargo", "test", "-p", "drift-cli", "--test", "e2e", *args],
        cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=600,
    )
    (output / f"{name}.log").write_text(result.stdout)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repeat", type=int, default=3)
    parser.add_argument("--mutant", action="append", choices=[r[0] for r in RECIPES])
    args = parser.parse_args()
    if not 1 <= args.repeat <= 20:
        parser.error("--repeat must be between 1 and 20")
    if git("status", "--porcelain"):
        parser.error("commit or stash changes before running the guard proof")
    lock = Path(git("rev-parse", "--git-path", "e2e-guard.lock"))
    if not lock.is_absolute():
        lock = ROOT / lock
    with lock.open("w") as held:
        fcntl.flock(held, fcntl.LOCK_EX | fcntl.LOCK_NB)
        output = ROOT / "target/e2e-guard-proof"
        output.mkdir(parents=True, exist_ok=True)
        summary = {"head": git("rev-parse", "HEAD"), "baseline": False, "stress_runs": 0, "mutants": []}
        (output / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
        baseline = run(output, "baseline", ["--", "--test-threads=8"])
        if baseline.returncode:
            raise SystemExit(f"baseline failed: {output / 'baseline.log'}")
        summary["baseline"] = True
        for iteration in range(args.repeat):
            result = run(output, f"stress-{iteration + 1}", [
                "--", "--test-threads=8", "--skip", "tree::", "--skip", "filter::",
                "--skip", "diff::", "--skip", "map::",
            ])
            if result.returncode:
                raise SystemExit(f"stress run failed: {output / f'stress-{iteration + 1}.log'}")
            summary["stress_runs"] += 1
        for name, relative, old, new, test in RECIPES:
            if args.mutant and name not in args.mutant:
                continue
            path = ROOT / relative
            original = path.read_bytes()
            text = original.decode()
            if text.count(old) != 1:
                raise SystemExit(f"{name}: recipe needs updating; expected exactly one source match")
            try:
                path.write_text(text.replace(old, new, 1))
                result = run(output, name, [test, "--", "--exact", "--test-threads=1"])
                killed = (result.returncode != 0 and f"test {test} ... FAILED" in result.stdout
                          and "test result: FAILED" in result.stdout and "could not compile" not in result.stdout)
                summary["mutants"].append({"name": name, "test": test, "killed": killed})
                (output / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
                print(f"{name}: {'killed' if killed else 'FAILED guard proof'}", flush=True)
                if not killed:
                    raise SystemExit(f"{name}: guard did not reject the mutant behaviorally; see {output / (name + '.log')}")
            finally:
                path.write_bytes(original)
        restored = run(output, "restored", ["--", "--test-threads=8"])
        summary["restored"] = restored.returncode == 0 and not git("status", "--porcelain")
        (output / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
        if not summary["restored"]:
            raise SystemExit(f"restored source validation failed: {output / 'restored.log'}")
        print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
