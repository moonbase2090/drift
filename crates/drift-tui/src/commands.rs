use lunatui_runtime::keymap::{Command, KeyChord, Keymap};
use lunatui_term::{Event, KeyCode, KeyEvent, Modifiers};

use drift_core::Status;

const fn plain(code: KeyCode) -> KeyChord {
    KeyChord::new(code, Modifiers::new())
}

const fn key(c: char) -> KeyChord {
    plain(KeyCode::Char(c))
}

pub(crate) fn press(code: KeyCode) -> Event {
    Event::Key(KeyEvent::press(code))
}

pub fn chord_text(chord: KeyChord) -> String {
    let mut text = String::new();
    if chord.modifiers().is_control() {
        text.push_str("Ctrl+");
    }
    if chord.modifiers().is_alt() {
        text.push_str("Alt+");
    }
    if chord.modifiers().is_shift() {
        text.push_str("Shift+");
    }
    match chord.code() {
        KeyCode::Char(' ') => text.push_str("Space"),
        KeyCode::Char(c) => text.push(c),
        KeyCode::F(n) => text.push_str(&format!("F{n}")),
        KeyCode::Enter => text.push_str("Enter"),
        KeyCode::Esc => text.push_str("Esc"),
        KeyCode::Tab => text.push_str("Tab"),
        other => text.push_str(&format!("{other:?}")),
    }
    text
}

pub fn chords_text(chords: &[KeyChord]) -> String {
    chords
        .iter()
        .map(|chord| chord_text(*chord))
        .collect::<Vec<_>>()
        .join(", ")
}

macro_rules! commands {
    ($name:ident, $all:ident, $keymap:ident { $($variant:ident => ($id:expr, $title:expr, [$($chord:expr),* $(,)?])),* $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum $name {
            $($variant),*
        }

        pub const $all: &[$name] = &[$($name::$variant),*];

        impl $name {
            pub fn id(self) -> &'static str {
                match self {
                    $($name::$variant => $id),*
                }
            }

            pub fn title(self) -> &'static str {
                match self {
                    $($name::$variant => $title),*
                }
            }

            pub fn chords(self) -> &'static [KeyChord] {
                match self {
                    $($name::$variant => const { &[$($chord),*] }),*
                }
            }

            pub fn from_id(id: &str) -> Option<Self> {
                $all.iter().copied().find(|command| command.id() == id)
            }
        }

        pub fn $keymap() -> Keymap {
            let mut map = Keymap::new();
            for command in $all {
                map.register(Command::new(command.id(), command.title(), command.chords()));
            }
            map
        }
    };
}

commands!(TreeCommand, TREE_COMMANDS, tree_keymap {
    Quit => ("tree.quit", "Quit", [key('q')]),
    Rescan => ("tree.rescan", "Rescan", [key('r')]),
    Down => ("tree.down", "Move down", [key('j'), plain(KeyCode::Down)]),
    Up => ("tree.up", "Move up", [key('k'), plain(KeyCode::Up)]),
    Collapse => ("tree.collapse", "Collapse folder or go to parent", [key('h'), plain(KeyCode::Left)]),
    Expand => ("tree.expand", "Expand folder or go to first child", [key('l'), plain(KeyCode::Right)]),
    Top => ("tree.top", "Jump to the first row", [key('g'), plain(KeyCode::Home)]),
    Bottom => ("tree.bottom", "Jump to the last row", [key('G'), plain(KeyCode::End)]),
    PageDown => ("tree.page-down", "Page down", [plain(KeyCode::PageDown)]),
    PageUp => ("tree.page-up", "Page up", [plain(KeyCode::PageUp)]),
    Open => ("tree.open", "Open the diff or toggle the folder", [plain(KeyCode::Enter)]),
    Search => ("tree.search", "Search", [key('/')]),
    FilterUnchanged => ("tree.filter-unchanged", "Filter: unchanged", [key('1')]),
    FilterUpstream => ("tree.filter-upstream", "Filter: upstream changed", [key('2')]),
    FilterOverride => ("tree.filter-override", "Filter: local override", [key('3')]),
    FilterConflict => ("tree.filter-conflict", "Filter: conflict", [key('4')]),
    FilterUserOnly => ("tree.filter-user-only", "Filter: user-only", [key('5')]),
    FilterMissing => ("tree.filter-missing", "Filter: missing", [key('6')]),
    ClearFilter => ("tree.clear-filter", "Clear the status filter", [key('0')]),
    Mark => ("tree.mark", "Mark or unmark", [key(' ')]),
    MarkAll => ("tree.mark-all", "Mark everything in view", [key('a')]),
    Keep => ("tree.keep", "Keep mine", [key('o')]),
    Take => ("tree.take", "Take upstream", [key('t')]),
    Merge => ("tree.merge", "Merge", [key('m')]),
    Ignore => ("tree.ignore", "Ignore", [key('i')]),
    History => ("tree.history", "Open the history", [key('H')]),
    Back => ("tree.back", "Clear search, filter or marks", [plain(KeyCode::Esc)]),
});

impl TreeCommand {
    pub fn status(self) -> Option<Status> {
        let index = match self {
            TreeCommand::FilterUnchanged => 0,
            TreeCommand::FilterUpstream => 1,
            TreeCommand::FilterOverride => 2,
            TreeCommand::FilterConflict => 3,
            TreeCommand::FilterUserOnly => 4,
            TreeCommand::FilterMissing => 5,
            _ => return None,
        };
        Status::ALL.get(index).copied()
    }
}

commands!(HistoryCommand, HISTORY_COMMANDS, history_keymap {
    Back => ("history.back", "Back to the tree", [key('q'), plain(KeyCode::Esc)]),
    Refresh => ("history.refresh", "Reload the history", [key('r')]),
    Undo => ("history.undo", "Undo the selected transaction", [plain(KeyCode::Enter)]),
    UndoForce => ("history.undo-force", "Undo and restore changed files too", [key('U')]),
    Down => ("history.down", "Move down", [key('j'), plain(KeyCode::Down)]),
    Up => ("history.up", "Move up", [key('k'), plain(KeyCode::Up)]),
    Top => ("history.top", "Jump to the newest", [key('g'), plain(KeyCode::Home)]),
    Bottom => ("history.bottom", "Jump to the oldest", [key('G'), plain(KeyCode::End)]),
    PageDown => ("history.page-down", "Page down", [plain(KeyCode::PageDown)]),
    PageUp => ("history.page-up", "Page up", [plain(KeyCode::PageUp)]),
});

commands!(GlobalCommand, GLOBAL_COMMANDS, global_keymap {
    Help => ("global.help", "Show the help", [key('?')]),
    Palette => ("global.palette", "Open the command palette", [key(':')]),
    UndoLast => ("global.undo-last", "Undo the last transaction", [key('u')]),
});

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Context {
    Tree,
    History,
    Diff,
}

pub fn context_name(context: Context) -> &'static str {
    match context {
        Context::Tree => "Tree",
        Context::History => "History",
        Context::Diff => "Diff",
    }
}

pub fn keymap_of(context: Context) -> Keymap {
    match context {
        Context::Tree => tree_keymap(),
        Context::History => history_keymap(),
        Context::Diff => crate::diff::diff_keymap(),
    }
}

pub struct Section {
    pub title: String,
    pub rows: Vec<(String, String)>,
}

fn section(title: &str, keymap: &Keymap) -> Section {
    Section {
        title: title.to_string(),
        rows: keymap
            .commands()
            .iter()
            .map(|command| (chords_text(command.chords()), command.title().to_string()))
            .collect(),
    }
}

pub fn help_sections(current: Context) -> Vec<Section> {
    let order = [Context::Tree, Context::History, Context::Diff];
    let mut sections = vec![section("Everywhere", &global_keymap())];
    let mut contexts: Vec<Context> = vec![current];
    contexts.extend(order.iter().copied().filter(|context| *context != current));
    for context in contexts {
        sections.push(section(context_name(context), &keymap_of(context)));
    }
    sections
}

pub fn help_lines(current: Context) -> Vec<String> {
    let mut lines = Vec::new();
    for section in help_sections(current) {
        if !lines.is_empty() {
            lines.push(String::new());
        }
        lines.push(section.title);
        for (keys, title) in section.rows {
            lines.push(format!("  {keys:<22}{title}"));
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_keymaps() -> Vec<(&'static str, Keymap)> {
        vec![
            ("tree", tree_keymap()),
            ("history", history_keymap()),
            ("global", global_keymap()),
            ("diff", crate::diff::diff_keymap()),
        ]
    }

    #[test]
    fn every_action_of_every_screen_has_a_binding() {
        for command in TREE_COMMANDS {
            assert!(!command.chords().is_empty(), "{}", command.id());
        }
        for command in HISTORY_COMMANDS {
            assert!(!command.chords().is_empty(), "{}", command.id());
        }
        for command in GLOBAL_COMMANDS {
            assert!(!command.chords().is_empty(), "{}", command.id());
        }
        for (name, keymap) in all_keymaps() {
            for command in keymap.commands() {
                assert!(!command.chords().is_empty(), "{name}: {}", command.id());
                assert!(!command.title().is_empty(), "{name}: {}", command.id());
            }
        }
    }

    #[test]
    fn the_keymaps_hold_exactly_the_actions_of_their_enums_in_order() {
        let ids = |keymap: &Keymap| -> Vec<&'static str> {
            keymap.commands().iter().map(Command::id).collect()
        };
        assert_eq!(
            ids(&tree_keymap()),
            TREE_COMMANDS.iter().map(|c| c.id()).collect::<Vec<_>>()
        );
        assert_eq!(
            ids(&history_keymap()),
            HISTORY_COMMANDS.iter().map(|c| c.id()).collect::<Vec<_>>()
        );
        assert_eq!(
            ids(&global_keymap()),
            GLOBAL_COMMANDS.iter().map(|c| c.id()).collect::<Vec<_>>()
        );
        for command in TREE_COMMANDS {
            assert_eq!(TreeCommand::from_id(command.id()), Some(*command));
        }
        for command in HISTORY_COMMANDS {
            assert_eq!(HistoryCommand::from_id(command.id()), Some(*command));
        }
        assert_eq!(TreeCommand::from_id("tree.nope"), None);
        assert_eq!(GlobalCommand::from_id("history.back"), None);
    }

    #[test]
    fn ids_and_titles_are_unique_and_no_key_runs_two_actions() {
        let mut ids = Vec::new();
        for (_, keymap) in all_keymaps() {
            for command in keymap.commands() {
                assert!(!ids.contains(&command.id()), "{}", command.id());
                ids.push(command.id());
            }
        }
        for (name, keymap) in all_keymaps() {
            let mut chords: Vec<KeyChord> = Vec::new();
            for command in keymap.commands() {
                for chord in command.chords() {
                    assert!(!chords.contains(chord), "{name}: {}", chord_text(*chord));
                    chords.push(*chord);
                }
            }
            let mut titles: Vec<&str> = keymap.commands().iter().map(Command::title).collect();
            titles.sort_unstable();
            let count = titles.len();
            titles.dedup();
            assert_eq!(titles.len(), count, "{name}");
        }
        let globals = global_keymap();
        for (name, keymap) in all_keymaps()
            .into_iter()
            .filter(|(name, _)| *name != "global")
        {
            for command in keymap.commands() {
                for chord in command.chords() {
                    let event =
                        Event::Key(KeyEvent::press(chord.code()).modifiers(chord.modifiers()));
                    assert_eq!(
                        globals.dispatch(&event),
                        None,
                        "{name}: {}",
                        chord_text(*chord)
                    );
                }
            }
        }
    }

    #[test]
    fn chords_read_as_the_keys_a_person_presses() {
        assert_eq!(chord_text(key(' ')), "Space");
        assert_eq!(chord_text(plain(KeyCode::Enter)), "Enter");
        assert_eq!(chord_text(plain(KeyCode::Esc)), "Esc");
        assert_eq!(chord_text(key('G')), "G");
        assert_eq!(chord_text(key('g')), "g");
        assert_eq!(chord_text(plain(KeyCode::PageDown)), "PageDown");
        assert_eq!(
            chord_text(KeyChord::new(KeyCode::Tab, Modifiers::new().shift())),
            "Shift+Tab"
        );
        assert_eq!(chords_text(TreeCommand::Down.chords()), "j, Down");
    }

    #[test]
    fn the_help_is_only_what_the_keymaps_say() {
        for current in [Context::Tree, Context::History, Context::Diff] {
            let sections = help_sections(current);
            assert_eq!(sections[0].title, "Everywhere");
            assert_eq!(sections[1].title, context_name(current));
            let mut expected = vec![("Everywhere".to_string(), global_keymap())];
            for context in [Context::Tree, Context::History, Context::Diff] {
                expected.push((context_name(context).to_string(), keymap_of(context)));
            }
            for (title, keymap) in &expected {
                let section = sections.iter().find(|found| &found.title == title).unwrap();
                let rows: Vec<(String, String)> = keymap
                    .commands()
                    .iter()
                    .map(|command| (chords_text(command.chords()), command.title().to_string()))
                    .collect();
                assert_eq!(section.rows, rows, "{title}");
            }
            let total: usize = expected
                .iter()
                .map(|(_, keymap)| keymap.commands().len())
                .sum();
            let rows: usize = sections.iter().map(|section| section.rows.len()).sum();
            assert_eq!(rows, total);
            let lines = help_lines(current);
            for (_, keymap) in &expected {
                for command in keymap.commands() {
                    assert!(
                        lines.iter().any(|line| line.contains(command.title())
                            && line.contains(&chords_text(command.chords()))),
                        "{current:?} {}",
                        command.id()
                    );
                }
            }
        }
    }

    #[test]
    fn the_status_filters_cover_every_status_once() {
        let statuses: Vec<Status> = TREE_COMMANDS.iter().filter_map(|c| c.status()).collect();
        assert_eq!(statuses, Status::ALL.to_vec());
    }
}
