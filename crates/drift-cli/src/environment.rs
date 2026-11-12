use std::path::PathBuf;

use drift_apply::Roots;

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
        pacman_lock: PathBuf::from("/var/lib/pacman/db.lck"),
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
