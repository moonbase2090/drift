use crate::harness::*;
use crate::system::System;
use crate::view::*;

fn opened() -> (System, Session) {
    let system = System::world();
    let session = system.open();
    session.wait_for_text("looknfeel.lua");
    (system, session)
}

#[test]
fn slash_opens_a_search_that_narrows_the_tree_as_you_type() {
    let (_system, mut session) = opened();
    session.press("/");
    session.wait_for_text("search: _");
    for letter in ["b", "i", "n", "d"] {
        session.press(letter);
    }
    let found = session.wait_for("only bindings left", |screen| {
        screen.contains("search: bind_") && screen.contains("1 of 8 files")
    });
    assert!(has_row(&found, "bindings.lua"));
    assert!(!has_row(&found, "looknfeel.lua"));
}

#[test]
fn backspace_widens_the_search_again() {
    let (_system, mut session) = opened();
    session.press("/");
    session.press("b");
    session.press("i");
    session.press("n");
    session.wait_for_text("1 of 8 files");
    session.press("\x7f");
    session.press("\x7f");
    session.press("\x7f");
    let found = session.wait_for("every file back", |screen| {
        screen.contains("8 of 8 files") && screen.contains("search: _")
    });
    assert!(has_row(&found, "looknfeel.lua"));
}

#[test]
fn enter_keeps_the_search_and_leaves_the_input_and_escape_clears_it() {
    let (_system, mut session) = opened();
    session.press("/");
    session.press("l");
    session.wait_for_text("7 of 8 files");
    session.press(ENTER);
    let kept = session.wait_for("search kept without the cursor", |screen| {
        screen.contains("search: l ") && !screen.contains("search: l_")
    });
    assert!(kept.contains("7 of 8 files"));
    session.press(ESC);
    session.wait_for("search cleared", |screen| {
        !screen.contains("search:") && screen.contains("8 of 8 files")
    });
}

#[test]
fn a_search_with_no_match_says_so_and_escape_recovers() {
    let (_system, mut session) = opened();
    session.press("/");
    for letter in ["z", "z", "z"] {
        session.press(letter);
    }
    session.wait_for_text("0 of 8 files");
    session.press(ESC);
    session.wait_for("every file back", |screen| {
        screen.contains("8 of 8 files") && has_row(screen, "looknfeel.lua")
    });
}

#[test]
fn clicking_the_search_hint_opens_the_search() {
    let (_system, mut session) = opened();
    session.click_text("[/] Search");
    session.wait_for_text("search: _");
}

fn only(screen: &Screen, status: &str, others: &[&str]) -> bool {
    has_row(screen, status) && others.iter().all(|other| !has_row(screen, other))
}

#[test]
fn the_number_keys_filter_the_tree_by_status_and_zero_clears_the_filter() {
    let (_system, mut session) = opened();
    let cases = [
        ("1", "unchanged", "filter: unchanged", "input.lua"),
        (
            "2",
            "upstream changed",
            "filter: upstream changed",
            "looknfeel.lua",
        ),
        (
            "3",
            "local override",
            "filter: local override",
            "monitors.lua",
        ),
        ("4", "conflict", "filter: conflict", "bindings.lua"),
        ("5", "user-only", "filter: user-only", "mine.lua"),
        ("6", "missing", "filter: missing", "config.jsonc"),
    ];
    for (key, _status, banner, file) in cases {
        session.press(key);
        let screen = session.wait_for(banner, |screen| {
            screen.contains(banner) && has_row(screen, file)
        });
        let others: Vec<&str> = cases
            .iter()
            .filter(|case| case.3 != file)
            .map(|case| case.3)
            .collect();
        assert!(only(&screen, file, &others), "{}", screen.text());
    }
    session.press("0");
    session.wait_for("filter cleared", |screen| {
        !screen.contains("filter:") && screen.contains("8 of 8 files")
    });
}

#[test]
fn clicking_a_status_count_filters_and_clicking_it_again_clears() {
    let (_system, mut session) = opened();
    session.click_text("4 ✗ conflict");
    session.wait_for("conflict filter", |screen| {
        screen.contains("filter: conflict") && has_row(screen, "bindings.lua")
    });
    session.click_text("4 ✗ conflict");
    session.wait_for("filter cleared", |screen| !screen.contains("filter:"));
}

#[test]
fn space_marks_the_selected_folder_and_everything_under_it() {
    let (_system, mut session) = opened();
    session.press("j");
    session.press(SPACE);
    let marked = session.wait_for("7 marked", |screen| screen.contains("7 marked"));
    assert_eq!(mark_of(&marked, ".config/"), Some('-'));
    assert_eq!(mark_of(&marked, "hypr/"), Some('x'));
    assert_eq!(mark_of(&marked, "bindings.lua"), Some('x'));
    assert_eq!(mark_of(&marked, "waybar/"), Some(' '));
    session.press(SPACE);
    session.wait_for("marks removed", |screen| !screen.contains("marked"));
}

#[test]
fn a_marks_everything_in_view_and_escape_clears_the_marks() {
    let (_system, mut session) = opened();
    session.press("a");
    let all = session.wait_for("8 marked", |screen| screen.contains("8 marked"));
    for name in [".config/", "hypr/", "waybar/", "bindings.lua", "mine.lua"] {
        assert_eq!(mark_of(&all, name), Some('x'), "{name}");
    }
    session.press(ESC);
    session.wait_for("marks cleared", |screen| !screen.contains("marked"));
}

#[test]
fn a_marks_only_what_the_filter_shows() {
    let (_system, mut session) = opened();
    session.press("4");
    session.wait_for_text("filter: conflict");
    session.press("a");
    session.wait_for_text("1 marked");
}

#[test]
fn clicking_a_checkbox_marks_that_row() {
    let (_system, mut session) = opened();
    let screen = session.screen();
    let (column, row) = screen.position("[ ] ✗ conflict").unwrap();
    session.click(column + 1, row);
    let marked = session.wait_for("one marked", |screen| screen.contains("1 marked"));
    assert_eq!(mark_of(&marked, "bindings.lua"), Some('x'));
    assert_eq!(mark_of(&marked, "input.lua"), Some(' '));
}

#[test]
fn escape_clears_the_search_the_filter_and_the_marks_one_after_another() {
    let (_system, mut session) = opened();
    session.press("a");
    session.press("4");
    session.wait_for("filter and marks", |screen| {
        screen.contains("filter: conflict") && screen.contains("marked")
    });
    session.press(ESC);
    session.wait_for("something cleared", |screen| {
        !screen.contains("filter: conflict") || !screen.contains("marked")
    });
    session.press(ESC);
    session.wait_for("everything cleared", |screen| {
        !screen.contains("filter:") && !screen.contains("marked")
    });
}
