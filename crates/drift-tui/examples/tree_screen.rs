use drift_core::{BackupSummary, Badge, BaseProvenance, ScanEntry, ScanReport, Status};

fn entry(path: &str, status: Status, base: BaseProvenance, badge: Option<Badge>) -> ScanEntry {
    ScanEntry {
        path: path.to_string(),
        template: Some(format!("config/{}", path.trim_start_matches(".config/"))),
        status,
        badge,
        base,
        volatile: false,
        note: String::new(),
    }
}

fn main() -> std::io::Result<()> {
    let none = BaseProvenance::None;
    let mut entries = vec![
        entry(
            ".config/hypr/monitors.lua",
            Status::LocalOverride,
            BaseProvenance::Reviewed,
            None,
        ),
        entry(
            ".config/hypr/looknfeel.lua",
            Status::UpstreamChanged,
            BaseProvenance::Reviewed,
            Some(Badge::MergesCleanly),
        ),
        entry(
            ".config/hypr/input.lua",
            Status::Conflict,
            BaseProvenance::Snapshot,
            None,
        ),
        entry(".config/hypr/bindings.lua", Status::Unchanged, none, None),
        entry(".config/hypr/autostart.lua", Status::Unchanged, none, None),
        entry(
            ".config/waybar/style.css",
            Status::LocalOverride,
            none,
            Some(Badge::BaseUnknown),
        ),
        entry(".config/waybar/config.jsonc", Status::Unchanged, none, None),
        entry(
            ".config/alacritty/alacritty.toml",
            Status::UpstreamChanged,
            BaseProvenance::Cache,
            None,
        ),
        entry(
            ".config/btop/btop.conf",
            Status::Missing,
            BaseProvenance::Reviewed,
            Some(Badge::DeletedByYou),
        ),
        entry(
            ".config/chromium/Default/Preferences",
            Status::LocalOverride,
            none,
            Some(Badge::BaseUnknown),
        ),
        entry(".config/git/config", Status::Unchanged, none, None),
        entry(".config/kitty/kitty.conf", Status::Unchanged, none, None),
        entry(".config/hypr/scratch.lua", Status::UserOnly, none, None),
    ];
    entries[9].volatile = true;
    entries[9].note = "app rewrites this file".to_string();
    let report = ScanReport {
        package_version: "4.5.0-3".to_string(),
        reviewed_version: Some("4.4.0-1".to_string()),
        entries,
        backups: vec![BackupSummary {
            dir: ".config/hypr".to_string(),
            count: 2,
        }],
        ignored: 0,
    };
    drift_tui::launch_with(&report)
}
