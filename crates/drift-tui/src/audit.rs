use lunatui_runtime::keymap::Keymap;
use lunatui_term::KeyCode;

use crate::commands::{chords_text, global_keymap, history_keymap, tree_keymap};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mouse {
    Hint(KeyCode),
    Click(&'static str),
    Palette,
}

pub(crate) struct Row {
    pub screen: &'static str,
    pub id: String,
    pub title: String,
    pub keys: String,
    pub mouse: Mouse,
    pub hints: &'static [(&'static str, KeyCode)],
}

pub(crate) const PLAN: &[(&str, &str, &str, &str)] = &[
    (
        "plan.confirm",
        "Confirm and write (Enter does it while Confirm has focus)",
        "y, Enter",
        "Click Confirm",
    ),
    (
        "plan.cancel",
        "Cancel, nothing is written (Enter does it while Cancel has focus)",
        "n, q, Esc, Enter",
        "Click Cancel",
    ),
    (
        "plan.focus",
        "Move focus between the buttons",
        "Tab, Left, Right, h, l",
        "Click a button to choose it directly",
    ),
    (
        "plan.select",
        "Select a file",
        "j, k, Up, Down, Home, End",
        "Click a file row",
    ),
    (
        "plan.expand",
        "Expand or collapse a file's diff",
        "d",
        "Click a file row",
    ),
];

pub(crate) const PRD_GAPS: &[(&str, &str, &str)] = &[
    ("Global", "u", "undo the last transaction from anywhere"),
    ("Diff", "N", "jump to the next unresolved conflict"),
    ("Diff", "P", "jump to the previous unresolved conflict"),
];

fn hints_of(context: &str) -> &'static [(&'static str, KeyCode)] {
    match context {
        "Tree" => crate::tree_screen::HINTS,
        "History" => crate::history_screen::HINTS,
        _ => crate::diff::HINTS,
    }
}

const fn hint(c: char) -> Mouse {
    Mouse::Hint(KeyCode::Char(c))
}

const COUNT: Mouse = Mouse::Click("Click the status count in the header");
const ROW: Mouse = Mouse::Click("Click a row");
const CARET: Mouse = Mouse::Click("Click the caret");
const DIVIDER: Mouse = Mouse::Click("Drag the pane divider");
const WHEEL: Mouse = Mouse::Click("Scroll the wheel");

const TARGETS: &[(&str, &str, Mouse)] = &[
    ("Everywhere", "global.help", hint('?')),
    ("Everywhere", "global.palette", hint(':')),
    ("Tree", "tree.quit", hint('q')),
    ("Tree", "tree.down", ROW),
    ("Tree", "tree.up", ROW),
    ("Tree", "tree.collapse", CARET),
    ("Tree", "tree.expand", CARET),
    ("Tree", "tree.open", Mouse::Hint(KeyCode::Enter)),
    ("Tree", "tree.search", hint('/')),
    ("Tree", "tree.filter-unchanged", COUNT),
    ("Tree", "tree.filter-upstream", COUNT),
    ("Tree", "tree.filter-override", COUNT),
    ("Tree", "tree.filter-conflict", COUNT),
    ("Tree", "tree.filter-user-only", COUNT),
    ("Tree", "tree.filter-missing", COUNT),
    (
        "Tree",
        "tree.clear-filter",
        Mouse::Click("Click the active status count again"),
    ),
    ("Tree", "tree.mark", Mouse::Click("Click the checkbox")),
    ("Tree", "tree.keep", hint('o')),
    ("Tree", "tree.take", hint('t')),
    ("Tree", "tree.merge", hint('m')),
    ("Tree", "tree.history", hint('H')),
    ("History", "history.back", hint('q')),
    ("History", "history.refresh", hint('r')),
    ("History", "history.undo", Mouse::Hint(KeyCode::Enter)),
    ("History", "history.undo-force", hint('U')),
    ("History", "history.down", ROW),
    ("History", "history.up", ROW),
    ("Diff", "diff.quit", hint('q')),
    ("Diff", "diff.next", hint('n')),
    ("Diff", "diff.previous", hint('p')),
    ("Diff", "diff.layout", hint('v')),
    ("Diff", "diff.focus-next", Mouse::Hint(KeyCode::Tab)),
    ("Diff", "diff.write", hint('w')),
    ("Diff", "diff.widen", DIVIDER),
    ("Diff", "diff.narrow", DIVIDER),
    ("Diff", "diff.down", WHEEL),
    ("Diff", "diff.up", WHEEL),
    (
        "Diff",
        "diff.mine",
        Mouse::Click("Click `[o]` in the header"),
    ),
    (
        "Diff",
        "diff.theirs",
        Mouse::Click("Click `[t]` in the header"),
    ),
    (
        "Diff",
        "diff.both",
        Mouse::Click("Click `[b]` in the header"),
    ),
    (
        "Diff",
        "diff.edit",
        Mouse::Click("Click `[e]` in the header"),
    ),
];

fn mouse_of(screen: &str, id: &str) -> Mouse {
    TARGETS
        .iter()
        .find(|(context, command, _)| *context == screen && *command == id)
        .map_or(Mouse::Palette, |(_, _, mouse)| *mouse)
}

fn rows_of(screen: &'static str, keymap: &Keymap) -> Vec<Row> {
    keymap
        .commands()
        .iter()
        .map(|command| Row {
            screen,
            id: command.id().to_string(),
            title: command.title().to_string(),
            keys: chords_text(command.chords()),
            mouse: mouse_of(screen, command.id()),
            hints: hints_of(screen),
        })
        .collect()
}

pub(crate) fn rows() -> Vec<Row> {
    let mut rows = rows_of("Everywhere", &global_keymap());
    rows.extend(rows_of("Tree", &tree_keymap()));
    rows.extend(rows_of("History", &history_keymap()));
    rows.extend(rows_of("Diff", &crate::diff::diff_keymap()));
    rows
}

pub(crate) fn mouse_text(row: &Row) -> String {
    match row.mouse {
        Mouse::Click(text) => text.to_string(),
        Mouse::Palette => "Open the palette, click the action".to_string(),
        Mouse::Hint(code) => {
            let label = row
                .hints
                .iter()
                .find(|(_, hint)| *hint == code)
                .map_or("(missing)", |(label, _)| *label);
            if row.screen == "Everywhere" {
                format!("Click `{label}` on any screen")
            } else {
                format!("Click `{label}`")
            }
        }
    }
}

pub fn input_map() -> String {
    let mut out = String::new();
    out.push_str("# Actions, keys and mouse targets\n\n");
    out.push_str(
        "Generated by the `the_input_map_document_is_current` test from the keymaps and the mouse\n\
         targets the screens declare. Every action has a key. An action without a direct mouse\n\
         target is reached through the command palette (`:`), which every screen opens by click.\n",
    );
    let mut current = "";
    for row in rows() {
        if row.screen != current {
            current = row.screen;
            out.push_str(&format!(
                "\n## {current}\n\n| Action | Id | Keys | Mouse |\n|---|---|---|---|\n"
            ));
        }
        out.push_str(&format!(
            "| {} | `{}` | `{}` | {} |\n",
            row.title,
            row.id,
            row.keys.replace(", ", "`, `"),
            mouse_text(&row)
        ));
    }
    out.push_str("\n## Plan dialog\n\n| Action | Id | Keys | Mouse |\n|---|---|---|---|\n");
    for (id, title, keys, mouse) in PLAN {
        out.push_str(&format!(
            "| {title} | `{id}` | `{}` | {mouse} |\n",
            keys.replace(", ", "`, `")
        ));
    }
    out.push_str(
        "\n## Keys the PRD lists that are not built yet\n\n| Context | Key | Action |\n|---|---|---|\n",
    );
    for (context, key, what) in PRD_GAPS {
        out.push_str(&format!("| {context} | `{key}` | {what} |\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{keymap_of, press, Context};

    #[test]
    fn every_action_has_a_key_and_every_direct_mouse_target_is_one_of_its_keys() {
        let rows = rows();
        assert!(rows.len() >= 50, "{}", rows.len());
        for row in &rows {
            assert!(!row.keys.is_empty(), "{}", row.id);
            if let Mouse::Hint(code) = row.mouse {
                let keymap = match row.screen {
                    "Tree" => tree_keymap(),
                    "History" => history_keymap(),
                    "Diff" => crate::diff::diff_keymap(),
                    _ => global_keymap(),
                };
                let found = keymap.dispatch(&press(code));
                assert_eq!(found, Some(row.id.as_str()), "{} via {code:?}", row.id);
                if row.screen == "Everywhere" {
                    for screen in ["Tree", "History", "Diff"] {
                        assert!(
                            hints_of(screen).iter().any(|(_, hint)| *hint == code),
                            "{} missing from the {screen} hint bar",
                            row.id
                        );
                    }
                } else {
                    assert!(
                        row.hints.iter().any(|(_, hint)| *hint == code),
                        "{} is not on the {} hint bar",
                        row.id,
                        row.screen
                    );
                }
            }
        }
    }

    #[test]
    fn every_declared_mouse_target_names_a_real_action_once() {
        let rows = rows();
        for (screen, id, _) in TARGETS {
            let hits = rows
                .iter()
                .filter(|row| row.screen == *screen && row.id == *id)
                .count();
            assert_eq!(hits, 1, "{screen} {id}");
        }
    }

    #[test]
    fn every_screen_opens_the_palette_by_click_and_the_palette_lists_every_action() {
        for screen in ["Tree", "History", "Diff"] {
            assert!(
                hints_of(screen)
                    .iter()
                    .any(|(_, code)| *code == KeyCode::Char(':')),
                "{screen}"
            );
        }
        for context in [Context::Tree, Context::History, Context::Diff] {
            let palette = crate::overlay::Palette::new(context);
            let titles: Vec<String> = palette.titles().iter().map(ToString::to_string).collect();
            for command in keymap_of(context).commands() {
                assert!(
                    titles.iter().any(|title| title == command.title()),
                    "{context:?}: {}",
                    command.title()
                );
            }
        }
    }

    fn implemented(context: &str) -> Vec<String> {
        let chords = |keymap: Keymap| -> Vec<String> {
            keymap
                .commands()
                .iter()
                .flat_map(|command| {
                    command
                        .chords()
                        .iter()
                        .map(|chord| crate::commands::chord_text(*chord))
                })
                .collect()
        };
        let mut found = match context {
            "Tree" | "Global" => chords(tree_keymap()),
            "History" => chords(history_keymap()),
            "Diff" => chords(crate::diff::diff_keymap()),
            "Per file (Tree or Diff)" => {
                let mut both = chords(tree_keymap());
                both.extend(chords(crate::diff::diff_keymap()));
                both
            }
            _ => PLAN
                .iter()
                .flat_map(|(_, _, keys, _)| keys.split(", ").map(str::to_string))
                .collect(),
        };
        if context != "Plan" {
            found.extend(chords(global_keymap()));
        }
        found
    }

    fn prd_keys() -> Vec<(String, String)> {
        let prd = include_str!("../../../docs/PRD.md");
        let start = prd.find("### 6.2 Keys").expect("section 6.2");
        let section = &prd[start..];
        let end = section.find("Layouts and widgets").expect("end of 6.2");
        let mut found = Vec::new();
        for line in section[..end].lines().filter(|line| line.starts_with("| ")) {
            let cells: Vec<&str> = line.split('|').map(str::trim).collect();
            if cells.len() < 4 || cells[1] == "Context" {
                continue;
            }
            for (at, piece) in cells[2].split('`').enumerate() {
                if at % 2 == 1 {
                    found.push((cells[1].to_string(), piece.replace('-', "+")));
                }
            }
        }
        found
    }

    #[test]
    fn every_key_the_prd_lists_is_built_or_a_named_gap() {
        let wanted = prd_keys();
        assert!(wanted.len() >= 40, "{}", wanted.len());
        let mut missing: Vec<(String, String)> = Vec::new();
        for (context, token) in &wanted {
            if !implemented(context).contains(token) {
                missing.push((context.clone(), token.clone()));
            }
        }
        let mut named: Vec<(String, String)> = PRD_GAPS
            .iter()
            .map(|(context, key, _)| (context.to_string(), key.to_string()))
            .collect();
        missing.sort();
        named.sort();
        assert_eq!(missing, named);
    }

    #[test]
    fn the_input_map_document_is_current() {
        let generated = input_map();
        let committed = include_str!("../../../docs/input-map.md");
        assert_eq!(
            generated, committed,
            "docs/input-map.md is stale: replace it with the generated text above"
        );
    }
}
