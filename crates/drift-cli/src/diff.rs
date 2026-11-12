use drift_core::RelPath;
use drift_source::build_file_diff;
use drift_source::ScanPaths;
use std::{collections::BTreeSet, io::Write, path::Path};

pub struct DiffArgs {
    pub path: String,
    pub json: bool,
}

pub fn parse(args: &[String]) -> Result<DiffArgs, String> {
    let mut path = None;
    let mut json = false;
    for arg in args {
        match arg.as_str() {
            "--json" if !json => json = true,
            value if !value.starts_with('-') && path.is_none() => path = Some(value.to_string()),
            other => {
                return Err(format!(
                    "unexpected argument '{other}'; usage: drift diff <path> [--json]"
                ))
            }
        }
    }
    Ok(DiffArgs {
        path: path.ok_or("usage: drift diff <path> [--json]")?,
        json,
    })
}

pub fn run(paths: &ScanPaths, args: &DiffArgs, out: &mut dyn Write) -> Result<u8, String> {
    let supplied = Path::new(&args.path);
    let relative = if supplied.is_absolute() {
        supplied
            .strip_prefix(&paths.source.home)
            .map_err(|_| "path is outside the home directory".to_string())?
    } else {
        supplied
    };
    let path = RelPath::new(relative.to_str().ok_or("path must be UTF-8")?)
        .map_err(|error| error.to_string())?;
    let registry = crate::scan::registry(paths)?;
    let diff = build_file_diff(&registry, paths, &BTreeSet::new(), &path)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("unknown tracked path '{}'", args.path))?;
    let text = if args.json {
        diff.to_json()
    } else {
        diff.plain_text()
    };
    out.write_all(text.as_bytes())
        .and_then(|()| out.flush())
        .map_err(|error| error.to_string())?;
    Ok(diff.exit_code())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn a_diff_takes_one_path_and_an_optional_json_flag() {
        let parsed = parse(&args(&[".config/hypr/a.lua"])).unwrap();
        assert_eq!(parsed.path, ".config/hypr/a.lua");
        assert!(!parsed.json);
        assert!(parse(&args(&["--json", ".config/a"])).unwrap().json);
        assert!(parse(&args(&[".config/a", "--json"])).unwrap().json);
        for invalid in [
            vec![],
            vec!["--json"],
            vec!["a", "b"],
            vec!["a", "--bar"],
            vec!["a", "--json", "--json"],
        ] {
            assert!(parse(&args(&invalid)).is_err(), "{invalid:?}");
        }
    }
}
