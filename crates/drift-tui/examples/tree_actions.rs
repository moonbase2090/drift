use drift_core::{
    prepare, Action, BackupSummary, Badge, BaseProvenance, Plan, PlanView, RelPath, ResolveInput,
    ScanEntry, ScanReport, Status,
};
use drift_tui::{Proposal, Resolver, Screen};
use lunatui::{run, RunOptions};

const FILES: [(&str, Status, &str, &str, &str); 4] = [
    (
        ".config/hypr/looknfeel.lua",
        Status::UpstreamChanged,
        "gaps_in = 5\nrounding = 2\nblur = true\nshadow = false\n",
        "gaps_in = 5\nrounding = 2\nblur = true\nshadow = false\n",
        "gaps_in = 8\nrounding = 2\nblur = true\nshadow = true\nanimations = true\n",
    ),
    (
        ".config/hypr/monitors.lua",
        Status::LocalOverride,
        "scale = 1\n",
        "scale = 2\n",
        "scale = 1\n",
    ),
    (
        ".config/waybar/style.css",
        Status::UpstreamChanged,
        "* {\n  font-size: 12px;\n}\n",
        "* {\n  font-size: 12px;\n}\n",
        "* {\n  font-size: 13px;\n}\n",
    ),
    (
        ".config/waybar/config.jsonc",
        Status::Unchanged,
        "{}\n",
        "{}\n",
        "{}\n",
    ),
];

struct Demo;

impl Resolver for Demo {
    fn propose(&mut self, action: Action, paths: &[String]) -> Result<Proposal, String> {
        let inputs: Vec<ResolveInput> = FILES
            .iter()
            .filter(|(path, ..)| paths.iter().any(|wanted| wanted == path))
            .map(|(path, _, base, ours, theirs)| ResolveInput {
                path: (*path).to_string(),
                root: "config".to_string(),
                rel: RelPath::new(path.trim_start_matches(".config/")).expect("sample path"),
                volatile: false,
                link: false,
                unreadable: false,
                base: Some(base.as_bytes().to_vec()),
                ours: Some(ours.as_bytes().to_vec()),
                theirs: Some(theirs.as_bytes().to_vec()),
            })
            .collect();
        let prepared = prepare(
            action,
            &inputs,
            "resolve-1731412200",
            "2026-11-12T10:30:00Z",
            "4.5.0-3",
            &[],
        )
        .map_err(|error| error.to_string())?;
        let skipped = prepared
            .skipped
            .iter()
            .map(|skip| format!("{} {}", skip.path, skip.reason.label()))
            .collect();
        let state = "~/.local/state/drift";
        let view = match prepared.plan.as_ref() {
            Some(plan) => PlanView::build(plan, state, &prepared.contents),
            None => PlanView::build(
                &Plan::new("none", "-", "4.5.0-3", Vec::new(), &[]).map_err(|e| e.to_string())?,
                state,
                &[],
            ),
        };
        Ok(Proposal {
            prepared,
            view,
            skipped,
        })
    }

    fn apply(&mut self, proposal: &Proposal) -> Result<String, String> {
        Ok(format!(
            "Demo: would apply {} file(s) as transaction {}; nothing was written.",
            proposal.prepared.writes.len(),
            proposal.view.id
        ))
    }
}

fn main() -> std::io::Result<()> {
    let entries = FILES
        .iter()
        .map(|(path, status, ..)| ScanEntry {
            path: (*path).to_string(),
            template: Some(format!("config/{}", path.trim_start_matches(".config/"))),
            status: *status,
            badge: (*status == Status::UpstreamChanged).then_some(Badge::MergesCleanly),
            base: BaseProvenance::Reviewed,
            volatile: false,
            note: String::new(),
        })
        .collect();
    let report = ScanReport {
        package_version: "4.5.0-3".to_string(),
        reviewed_version: Some("4.4.0-1".to_string()),
        entries,
        backups: vec![BackupSummary {
            dir: ".config/hypr".to_string(),
            count: 1,
        }],
        ignored: 0,
    };
    let screen = Screen::with_report(&report).with_resolver(Box::new(Demo));
    let (columns, rows) = terminal_size::terminal_size()
        .map(|(c, r)| (c.0, r.0))
        .unwrap_or((100, 28));
    run(screen, RunOptions::new(columns, rows).pixels(false))
}
