use crate::harness::*;
use crate::system::System;
use crate::view::*;

fn opened() -> (System, Session) {
    let system = System::world();
    let session = system.open();
    session.wait_for_text("looknfeel.lua");
    (system, session)
}

fn open_palette(session: &mut Session) -> usize {
    session.press(":");
    let screen = session.wait_for("the palette", |screen| {
        screen.contains("Command palette") && palette_count(screen).is_some()
    });
    palette_count(&screen).unwrap()
}

#[test]
fn question_mark_opens_the_help_with_every_section_and_escape_closes_it() {
    let (_system, mut session) = opened();
    session.press("?");
    let help = session.wait_for("the help", |screen| screen.first().contains("Help"));
    for section in ["Everywhere", "Tree"] {
        assert!(help.contains(section), "{}", help.text());
    }
    assert!(help.contains("Show the help"));
    assert!(help.contains("[Esc] Close"));
    session.press(ESC);
    session.wait_for("the tree again", |screen| {
        screen.contains("8 of 8 files") && !screen.first().contains("Help")
    });
}

#[test]
fn clicking_the_help_hint_opens_the_help() {
    let (_system, mut session) = opened();
    session.click_text("[?] Help");
    session.wait_for("the help", |screen| screen.first().contains("Help"));
}

#[test]
fn the_help_scrolls_with_j_and_k_through_every_section() {
    let system = System::world();
    let mut session = system.launch(&[], (110, 14));
    session.wait_for_text("looknfeel.lua");
    session.press("?");
    let top = session.wait_for("the help", |screen| screen.first().contains("Help"));
    assert!(top.contains("Show the help") && !top.contains("Layout"));
    for _ in 0..70 {
        session.press("j");
    }
    session.wait_for("a later section", |screen| {
        screen.contains("Layout") && !screen.contains("Show the help")
    });
    for _ in 0..140 {
        session.press("k");
    }
    session.wait_for("the top again", |screen| {
        screen.contains("Show the help") && !screen.contains("Layout")
    });
}

#[test]
fn the_help_lists_every_action_of_the_key_map_document() {
    let system = System::world();
    let mut session = system.launch(&[], (110, 120));
    session.wait_for_text("looknfeel.lua");
    session.press("?");
    let help = session.wait_for("the whole help", |screen| {
        screen.first().contains("Help") && screen.contains("Show the help")
    });
    let document = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/input-map.md"),
    )
    .unwrap();
    let section = document
        .split("\n## ")
        .find(|part| part.starts_with("Tree"))
        .unwrap();
    let titles: Vec<String> = section
        .lines()
        .filter(|line| line.starts_with("| ") && line.contains("`tree."))
        .map(|line| line.split('|').nth(1).unwrap().trim().to_string())
        .collect();
    assert!(titles.len() > 20);
    for title in titles {
        assert!(help.contains(&title), "{title} is not in the help");
    }
}

#[test]
fn colon_opens_the_palette_listing_the_commands_and_escape_closes_it() {
    let (_system, mut session) = opened();
    let all = open_palette(&mut session);
    assert!(all > 20, "{all}");
    let screen = session.screen();
    assert!(screen.contains("Quit"));
    assert!(screen.contains("Rescan"));
    session.press(ESC);
    session.wait_for("the palette closed", |screen| {
        !screen.contains("Command palette") && selected_is(screen, ".config/")
    });
}

#[test]
fn clicking_the_palette_hint_opens_it() {
    let (_system, mut session) = opened();
    session.click_text("[:] Palette");
    session.wait_for_text("Command palette");
}

#[test]
fn typing_narrows_the_palette_and_backspace_widens_it() {
    let (_system, mut session) = opened();
    let all = open_palette(&mut session);
    for letter in ["l", "a", "s", "t"] {
        session.press(letter);
    }
    let narrow = session.wait_for("a narrower list", |screen| {
        palette_count(screen).is_some_and(|count| count < all)
    });
    assert!(narrow.contains("Jump to the last row"));
    assert!(!narrow.contains("Rescan"));
    for _ in 0..4 {
        session.press("\x7f");
    }
    session.wait_for("the full list again", |screen| {
        palette_count(screen) == Some(all)
    });
}

#[test]
fn enter_runs_the_first_match_and_closes_the_palette() {
    let (_system, mut session) = opened();
    open_palette(&mut session);
    for letter in "last row".chars() {
        session.press(&letter.to_string());
    }
    session.wait_for("the last-row command first", |screen| {
        palette_first(screen).is_some_and(|row| row.starts_with("Jump to the last row"))
    });
    session.press(ENTER);
    session.wait_for("the last row selected and the palette closed", |screen| {
        !screen.contains("Command palette") && selected_is(screen, "waybar/")
    });
}

#[test]
fn down_chooses_the_next_match_before_enter() {
    let (_system, mut session) = opened();
    session.press("G");
    session.wait_for("the last row", |screen| selected_is(screen, "waybar/"));
    open_palette(&mut session);
    for letter in "jump".chars() {
        session.press(&letter.to_string());
    }
    session.wait_for("two matches", |screen| palette_count(screen) == Some(2));
    session.press(ENTER);
    session.wait_for("the first row from the first match", |screen| {
        selected_is(screen, ".config/") && !screen.contains("Command palette")
    });
    session.press("G");
    session.wait_for("the last row", |screen| selected_is(screen, "waybar/"));
    open_palette(&mut session);
    for letter in "jump".chars() {
        session.press(&letter.to_string());
    }
    session.wait_for("two matches", |screen| palette_count(screen) == Some(2));
    session.press(DOWN);
    session.press(ENTER);
    session.wait_for("the palette closed", |screen| {
        !screen.contains("Command palette")
    });
    assert!(selected_is(&session.screen(), "waybar/"));
}

#[test]
fn clicking_a_palette_row_runs_that_command() {
    let (_system, mut session) = opened();
    open_palette(&mut session);
    session.click_text("Jump to the last row");
    session.wait_for("the last row selected and the palette closed", |screen| {
        !screen.contains("Command palette") && selected_is(screen, "waybar/")
    });
}

#[test]
fn a_palette_search_with_no_match_runs_nothing_and_stays_open() {
    let (_system, mut session) = opened();
    open_palette(&mut session);
    for letter in ["z", "z", "z"] {
        session.press(letter);
    }
    session.wait_for("no commands", |screen| palette_count(screen) == Some(0));
    session.press(ENTER);
    let after = session.screen();
    assert!(after.contains("Command palette"));
    assert_eq!(palette_count(&after), Some(0));
    session.press(ESC);
    session.wait_for("closed", |screen| !screen.contains("Command palette"));
}

#[test]
fn a_command_with_no_key_of_its_own_is_reachable_from_the_palette() {
    let (_system, mut session) = opened();
    open_palette(&mut session);
    for letter in "mark every".chars() {
        session.press(&letter.to_string());
    }
    session.wait_for("the mark-everything command first", |screen| {
        palette_first(screen).is_some_and(|row| row.starts_with("Mark everything in view"))
    });
    session.press(ENTER);
    session.wait_for("everything marked", |screen| screen.contains("8 marked"));
}
