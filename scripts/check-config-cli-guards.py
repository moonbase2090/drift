#!/usr/bin/env python3
"""Replay config/diagnostic regressions and prove each guard rejects a broken path."""
import fcntl
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]


def git(*args):
    return subprocess.check_output(["git", *args], cwd=ROOT, text=True).strip()


def main():
    with open(ROOT / git("rev-parse", "--git-path", "config-cli-guards.lock"), "w") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        if git("status", "--porcelain"):
            raise SystemExit("Use a clean, idle worktree.")
        head = git("rev-parse", "HEAD")
        logs = ROOT / "target/config-cli-guard-proof"
        logs.mkdir(parents=True, exist_ok=True)
        summary = {"head": head, "mutants": [], "restored": False}
        command = ["cargo", "test", "-p", "drift-cli", "--test", "config_and_messages"]

        def run(name, test=None):
            args = command + ([test, "--", "--exact"] if test else [])
            result = subprocess.run(args, cwd=ROOT, capture_output=True, text=True, timeout=600)
            text = result.stdout + result.stderr
            (logs / f"{name}.log").write_text(text)
            if test:
                assert result.returncode != 0 and f"test {test} ... FAILED" in text and "test result: FAILED" in text, name
            else:
                assert result.returncode == 0 and "7 passed; 0 failed" in text, name

        run("baseline")
        recipes = [
            ("config", "scan.rs", "let path = &paths.user_config;", 'let path = &paths.source.home.join(".config/drift/config.toml");', "snapshot_uses_xdg_config_instead_of_a_broken_home_config"),
            ("range", "main.rs", "must be between 1 and 65535", "needs a number", "dimensions_report_numeric_overflow_as_a_range_error"),
            ("integrate-lock", "environment.rs", "report_update_lock(roots, err)?;", "let _ = (roots, &mut *err);", "an_already_integrated_install_reports_the_held_lock_without_writing"),
            ("resolve-lock", "resolve.rs", "crate::environment::report_update_lock(env.roots, err)?;", "let _ = (env.roots, &mut *err);", "resolving_an_unchanged_file_reports_the_held_lock_without_writing"),
        ]
        for name, file, before, after, test in recipes:
            path = ROOT / "crates/drift-cli/src" / file
            original = path.read_bytes()
            assert original.count(before.encode()) == 1, name
            try:
                path.write_bytes(original.replace(before.encode(), after.encode()))
                run(name, test)
                summary["mutants"].append({"name": name, "killed": True})
                print(f"{name}: killed", flush=True)
            finally:
                path.write_bytes(original)
        run("restored")
        assert not git("status", "--porcelain")
        summary["restored"] = True
        (logs / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
        print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
