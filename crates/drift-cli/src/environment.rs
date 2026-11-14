use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::time::Duration;

use drift_apply::{IntegrateMode, IntegrateOutcome, Roots};

pub fn run_integrate(
    roots: &Roots,
    elapsed: Duration,
    mode: IntegrateMode,
    interactive: bool,
    input: &mut dyn BufRead,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<(), String> {
    let id = format!("integrate-{}", elapsed.as_millis());
    let time = utc_timestamp(elapsed.as_secs());
    let mut ask = || {
        if !interactive {
            return false;
        }
        let mut line = String::new();
        input.read_line(&mut line).is_ok() && matches!(line.trim(), "y" | "Y" | "yes")
    };
    let outcome = drift_apply::integrate(roots, &id, &time, mode, out, &mut ask)?;
    if outcome == IntegrateOutcome::Declined && !interactive {
        let note = "drift: not a terminal, so nothing was written; pass --yes to apply\n";
        err.write_all(note.as_bytes())
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

pub fn roots_from_env() -> Result<Roots, String> {
    roots_from(&|name| std::env::var_os(name))
}

fn roots_from(lookup: &dyn Fn(&str) -> Option<std::ffi::OsString>) -> Result<Roots, String> {
    let var = |name: &str| lookup(name).filter(|value| !value.is_empty());
    let home = var("HOME").map(PathBuf::from);
    let config = match (var("XDG_CONFIG_HOME"), &home) {
        (Some(dir), _) => PathBuf::from(dir),
        (None, Some(home)) => home.join(".config"),
        (None, None) => return Err("HOME is not set".to_string()),
    };
    let state = match (var("XDG_STATE_HOME"), &home) {
        (Some(dir), _) => PathBuf::from(dir),
        (None, Some(home)) => home.join(".local/state"),
        (None, None) => return Err("HOME is not set".to_string()),
    };
    let runtime = var("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/run"));
    Ok(Roots {
        live: config.join("omarchy"),
        state: state.join("drift"),
        omarchy_lock: runtime.join("omarchy-update.lock"),
        pacman_lock: drift_source::system_path("/var/lib/pacman/db.lck"),
    })
}

pub fn utc_timestamp(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rest = secs % 86_400;
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3_600,
        rest % 3_600 / 60,
        rest % 60
    )
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::*;

    fn absent_roots() -> Roots {
        Roots {
            live: PathBuf::from("/nonexistent/drift-test/live"),
            state: PathBuf::from("/nonexistent/drift-test/state"),
            omarchy_lock: PathBuf::from("/nonexistent/drift-test/lock"),
            pacman_lock: PathBuf::from("/nonexistent/drift-test/db.lck"),
        }
    }

    fn drive(
        mode: IntegrateMode,
        interactive: bool,
        input: &str,
    ) -> (Result<(), String>, String, String) {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let result = run_integrate(
            &absent_roots(),
            Duration::from_millis(1_700_000_000_123),
            mode,
            interactive,
            &mut input.as_bytes(),
            &mut out,
            &mut err,
        );
        (
            result,
            String::from_utf8(out).unwrap(),
            String::from_utf8(err).unwrap(),
        )
    }

    #[test]
    fn a_dry_run_shows_the_diff_and_writes_nothing() {
        let (result, out, err) = drive(IntegrateMode::DryRun, false, "");
        result.unwrap();
        assert!(out.contains("+++ b/hooks/post-update.d/drift"), "{out}");
        assert!(out.contains("Dry run"), "{out}");
        assert!(err.is_empty(), "{err}");
    }

    #[test]
    fn without_a_terminal_nothing_is_written_and_the_user_is_told() {
        let (result, out, err) = drive(IntegrateMode::Confirm, false, "y\n");
        result.unwrap();
        assert!(out.contains("Nothing was written"), "{out}");
        assert!(err.contains("pass --yes"), "{err}");
    }

    #[test]
    fn only_a_yes_answer_goes_on_to_write() {
        for answer in ["n\n", "\n", "maybe\n", ""] {
            let (result, out, err) = drive(IntegrateMode::Confirm, true, answer);
            result.unwrap();
            assert!(out.contains("Nothing was written"), "{answer:?} {out}");
            assert!(err.is_empty(), "{err}");
        }
        for answer in ["y\n", "Y\n", "yes\n", "  yes  \n"] {
            let (result, _, _) = drive(IntegrateMode::Confirm, true, answer);
            let error = result.unwrap_err();
            assert!(
                error.contains("/nonexistent/drift-test"),
                "{answer:?} {error}"
            );
        }
    }

    #[test]
    fn timestamps_are_utc_iso_8601() {
        assert_eq!(utc_timestamp(0), "1970-01-01T00:00:00Z");
        assert_eq!(utc_timestamp(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(utc_timestamp(1_700_000_000), "2023-11-14T22:13:20Z");
        assert_eq!(utc_timestamp(1_709_251_199), "2024-02-29T23:59:59Z");
        assert_eq!(utc_timestamp(1_709_251_200), "2024-03-01T00:00:00Z");
    }

    #[test]
    fn roots_follow_the_xdg_variables_with_home_fallbacks() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| OsString::from(value))
            }
        };
        let roots = roots_from(&env(&[("HOME", "/h")])).unwrap();
        assert_eq!(roots.live, PathBuf::from("/h/.config/omarchy"));
        assert_eq!(roots.state, PathBuf::from("/h/.local/state/drift"));
        assert_eq!(
            roots.omarchy_lock,
            PathBuf::from("/run/omarchy-update.lock")
        );
        assert_eq!(roots.pacman_lock, PathBuf::from("/var/lib/pacman/db.lck"));
        let roots = roots_from(&env(&[
            ("HOME", "/h"),
            ("XDG_CONFIG_HOME", "/c"),
            ("XDG_STATE_HOME", "/s"),
            ("XDG_RUNTIME_DIR", "/r"),
        ]))
        .unwrap();
        assert_eq!(roots.live, PathBuf::from("/c/omarchy"));
        assert_eq!(roots.state, PathBuf::from("/s/drift"));
        assert_eq!(roots.omarchy_lock, PathBuf::from("/r/omarchy-update.lock"));
        let roots =
            roots_from(&env(&[("XDG_CONFIG_HOME", "/c"), ("XDG_STATE_HOME", "/s")])).unwrap();
        assert_eq!(roots.live, PathBuf::from("/c/omarchy"));
        assert!(roots_from(&env(&[])).is_err());
        assert!(roots_from(&env(&[("HOME", "")])).is_err());
    }
}
