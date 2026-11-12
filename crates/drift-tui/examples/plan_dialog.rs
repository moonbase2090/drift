use drift_core::{Action, FileContents, Plan, PlanView, PlannedFile};
use drift_tui::{review_plan, Decision};
use sha2::{Digest, Sha256};

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

struct Sample {
    path: &'static str,
    action: Action,
    before: Option<&'static str>,
    after: &'static str,
}

fn main() -> std::io::Result<()> {
    let samples = [
        Sample {
            path: "config/hypr/looknfeel.lua",
            action: Action::Merge,
            before: Some("gaps_in = 5\ngaps_out = 10\nrounding = 2\nblur = true\nshadow = false\n"),
            after: "gaps_in = 8\ngaps_out = 10\nrounding = 2\nblur = true\nshadow = true\nanimations = true\n",
        },
        Sample {
            path: "config/waybar/style.css",
            action: Action::TakeUpstream,
            before: None,
            after: "* {\n  font-size: 13px;\n}\n",
        },
        Sample {
            path: "config/hypr/monitors.lua",
            action: Action::KeepMine,
            before: Some("scale = 2\n"),
            after: "scale = 2\n",
        },
    ];
    let files: Vec<PlannedFile> = samples
        .iter()
        .map(|sample| {
            PlannedFile::new(
                sample.path,
                sample.action,
                sample.before.map(|text| digest(text.as_bytes())).as_deref(),
                Some(&digest(sample.after.as_bytes())),
                Some(0o644),
                None,
            )
            .expect("sample file")
        })
        .collect();
    let plan = Plan::new(
        "txn-1731412200",
        "2026-11-12T10:30:00Z",
        "4.5.0-3",
        files,
        &["txn-1722000000"],
    )
    .expect("sample plan");
    let contents: Vec<FileContents> = samples
        .iter()
        .map(|sample| FileContents {
            path: sample.path.to_string(),
            before: sample.before.map(|text| text.as_bytes().to_vec()),
            after: Some(sample.after.as_bytes().to_vec()),
        })
        .collect();
    let view = PlanView::build(&plan, "~/.local/state/drift", &contents);
    let decision = review_plan(view)?;
    match decision {
        Decision::Confirm => println!("decision: confirm (the demo writes nothing)"),
        Decision::Cancel => println!("decision: cancel (nothing was written)"),
    }
    Ok(())
}
