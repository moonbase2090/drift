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
fn q_quits_and_exits_cleanly() {
    let (_system, mut session) = opened();
    session.press("q");
    assert_eq!(session.wait_exit(), 0);
}

#[test]
fn clicking_the_quit_hint_quits() {
    let (_system, mut session) = opened();
    session.click_text("[Q] Quit");
    assert_eq!(session.wait_exit(), 0);
}

#[test]
fn j_and_down_move_the_selection_down() {
    let (_system, mut session) = opened();
    assert!(selected_is(&session.screen(), ".config/"));
    session.press("j");
    session.wait_for("hypr/ selected", |screen| selected_is(screen, "hypr/"));
    session.press(DOWN);
    session.wait_for("autostart selected", |screen| {
        selected_is(screen, "autostart.lua")
    });
}

#[test]
fn k_and_up_move_the_selection_up() {
    let (_system, mut session) = opened();
    for _ in 0..3 {
        session.press("j");
    }
    session.wait_for("bindings selected", |screen| {
        selected_is(screen, "bindings.lua")
    });
    session.press("k");
    session.wait_for("autostart selected", |screen| {
        selected_is(screen, "autostart.lua")
    });
    session.press(UP);
    session.wait_for("hypr/ selected", |screen| selected_is(screen, "hypr/"));
}

#[test]
fn h_and_left_collapse_a_folder_and_l_and_right_expand_it() {
    let (_system, mut session) = opened();
    session.press("j");
    session.wait_for("hypr/ selected", |screen| selected_is(screen, "hypr/"));
    session.press("h");
    let collapsed = session.wait_for("hypr collapsed", |screen| !has_row(screen, "autostart.lua"));
    assert!(has_row(&collapsed, "▸ [ ] hypr/"));
    session.press("l");
    session.wait_for("hypr expanded", |screen| has_row(screen, "autostart.lua"));
    session.press(LEFT);
    session.wait_for("hypr collapsed again", |screen| {
        !has_row(screen, "autostart.lua")
    });
    session.press(RIGHT);
    session.wait_for("hypr expanded again", |screen| {
        has_row(screen, "autostart.lua")
    });
}

#[test]
fn h_on_a_file_goes_to_its_folder_and_l_on_an_open_folder_goes_to_its_first_child() {
    let (_system, mut session) = opened();
    for _ in 0..3 {
        session.press("j");
    }
    session.wait_for("bindings selected", |screen| {
        selected_is(screen, "bindings.lua")
    });
    session.press("h");
    session.wait_for("hypr/ selected", |screen| selected_is(screen, "hypr/"));
    session.press("l");
    session.wait_for("first child selected", |screen| {
        selected_is(screen, "autostart.lua")
    });
}

#[test]
fn g_and_home_jump_to_the_first_row_and_capital_g_and_end_to_the_last() {
    let (_system, mut session) = opened();
    session.press("G");
    session.wait_for("last row selected", |screen| selected_is(screen, "waybar/"));
    session.press("g");
    session.wait_for("first row selected", |screen| {
        selected_is(screen, ".config/")
    });
    session.press(END);
    session.wait_for("last row selected by End", |screen| {
        selected_is(screen, "waybar/")
    });
    session.press(HOME);
    session.wait_for("first row selected by Home", |screen| {
        selected_is(screen, ".config/")
    });
}

#[test]
fn page_down_and_page_up_move_by_a_screenful() {
    let system = System::world();
    let mut session = system.launch(&[], (120, 12));
    session.wait_for_text("looknfeel.lua");
    session.press(PAGE_DOWN);
    let moved = session.wait_for("selection to leave the first row", |screen| {
        !selected_is(screen, ".config/")
    });
    assert!(!selected(&moved).is_empty());
    session.press(PAGE_UP);
    session.wait_for("selection back on the first row", |screen| {
        selected_is(screen, ".config/")
    });
}

#[test]
fn enter_opens_the_diff_of_a_file_and_toggles_a_folder() {
    let (_system, mut session) = opened();
    session.press("j");
    session.wait_for("hypr/ selected", |screen| selected_is(screen, "hypr/"));
    session.press(ENTER);
    session.wait_for("hypr collapsed", |screen| !has_row(screen, "autostart.lua"));
    session.press(ENTER);
    session.wait_for("hypr expanded", |screen| has_row(screen, "autostart.lua"));
    session.press("j");
    session.press(ENTER);
    let diff = session.wait_for("the diff", |screen| screen.first().contains("Diff"));
    assert!(diff.contains("autostart.lua"));
}

#[test]
fn clicking_a_row_selects_it_and_clicking_the_caret_toggles_the_folder() {
    let (_system, mut session) = opened();
    session.click_text("bindings.lua");
    session.wait_for("bindings selected", |screen| {
        selected_is(screen, "bindings.lua")
    });
    let screen = session.screen();
    let (column, row) = screen.position("▾ [ ] hypr/").unwrap();
    session.click(column, row);
    session.wait_for("hypr collapsed", |screen| !has_row(screen, "autostart.lua"));
}
