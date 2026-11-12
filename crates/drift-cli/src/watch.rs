use std::path::{Path, PathBuf};

use drift_core::Registry;
use drift_tui::ChangeSink;
use notify::event::EventKind;
use notify::{recommended_watcher, RecommendedWatcher, RecursiveMode, Watcher};

pub(crate) fn dirs(registry: &Registry, home: &Path, omarchy: &Path) -> Vec<PathBuf> {
    registry
        .roots()
        .iter()
        .flat_map(|root| [root.live().under(home), root.template().under(omarchy)])
        .collect()
}

pub(crate) fn arm(paths: &[PathBuf], sink: ChangeSink) -> Option<RecommendedWatcher> {
    let mut watcher = recommended_watcher(move |result: Result<notify::Event, notify::Error>| {
        let Ok(event) = result else {
            return;
        };
        if relevant(event.kind) {
            sink.changed();
        }
    })
    .ok()?;
    let mut watching = false;
    for path in paths {
        if watcher.watch(path, RecursiveMode::Recursive).is_ok() {
            watching = true;
        }
    }
    watching.then_some(watcher)
}

fn relevant(kind: EventKind) -> bool {
    !matches!(kind, EventKind::Access(_) | EventKind::Other)
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use drift_core::Registry;

    use super::dirs;

    #[test]
    fn tracked_roots_watch_the_live_tree_and_the_package_templates() {
        let registry = Registry::v1();
        let watched = dirs(
            &registry,
            Path::new("/home/user"),
            Path::new("/usr/share/omarchy"),
        );
        assert_eq!(
            watched,
            vec![
                PathBuf::from("/home/user/.config"),
                PathBuf::from("/usr/share/omarchy/config"),
            ]
        );
    }
}
