use std::process::ExitCode;

const USAGE: &str = "Usage: drift [--version] [--help]";

fn run(args: &[String]) -> Result<String, String> {
    match args.first().map(String::as_str) {
        None | Some("-h") | Some("--help") => Ok(USAGE.to_string()),
        Some("-V") | Some("--version") => Ok(format!("drift {}", env!("CARGO_PKG_VERSION"))),
        Some(other) => Err(format!("drift: unknown argument '{other}'\n{USAGE}")),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(text) => {
            println!("{text}");
            ExitCode::SUCCESS
        }
        Err(text) => {
            eprintln!("{text}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn version_flag_prints_version() {
        let out = run(&args(&["--version"])).unwrap();
        assert_eq!(out, format!("drift {}", env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn no_arguments_prints_usage() {
        assert_eq!(run(&[]).unwrap(), USAGE);
    }

    #[test]
    fn unknown_argument_is_an_error() {
        assert!(run(&args(&["bogus"])).is_err());
    }
}
