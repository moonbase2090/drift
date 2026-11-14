use std::path::PathBuf;

use drift_apply::snapshot_templates;
use drift_source::{read_package, ReadPaths};

pub fn parse(args: &[String]) -> Result<(), String> {
    if args.len() == 1 && args[0] == "--pre" {
        Ok(())
    } else {
        Err("usage: drift snapshot --pre".to_string())
    }
}

pub fn run() -> Result<(), String> {
    let paths = crate::scan::paths_from_env();
    let registry = crate::scan::load_registry(&paths)?;
    let root = env_path("DRIFT_PACMAN_ROOT").unwrap_or_else(drift_source::system_root);
    let package = ReadPaths::new(root, &paths.source.package.omarchy_path);
    let installed = read_package(&package).map_err(|error| error.to_string())?;
    let snapshots = env_path("DRIFT_SNAPSHOT_DIR")
        .unwrap_or_else(|| drift_source::system_path("/var/lib/drift/snapshots"));
    snapshot_templates(
        &registry,
        &paths.source.package.omarchy_path,
        &snapshots,
        &installed.version,
    )
    .map(|_| ())
    .map_err(|error| error.to_string())
}

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}
