use crate::harness::*;
use crate::system::{System, CONFLICT, TWO_CONFLICTS};
use crate::view::*;

fn rules() -> System {
    System::builder().template(&TWO_CONFLICTS).build()
}

fn bindings() -> System {
    System::builder().template(&CONFLICT).build()
}

fn open_diff(system: &System, name: &str, size: (u16, u16)) -> Session {
    let mut session = system.launch(&[], size);
    session.wait_for_text(name);
    session.click_text(name);
    session.wait_for("the file selected", |screen| {
        selected(screen).contains(name)
    });
    session.press(ENTER);
    session.wait_for("the diff", |screen| screen.first().starts_with("Diff"));
    session
}

fn is_four(screen: &Screen) -> bool {
    screen.rows().get(1).is_some_and(|row| {
        row.contains("hunks")
            && row.contains("base")
            && row.contains("ours")
            && row.contains("theirs")
            && row.contains("result")
    })
}

fn is_two(screen: &Screen) -> bool {
    screen.rows().get(1).is_some_and(|row| {
        row.contains("ours")
            && row.contains("theirs")
            && !row.contains("base")
            && !row.contains("hunks")
    }) && screen.contains("result")
}

fn is_stacked(screen: &Screen) -> bool {
    screen.contains("stacked") && screen.contains("< ours") && screen.contains("> theirs")
}

#[test]
fn from_200_columns_the_diff_opens_in_five_columns() {
    let system = rules();
    let session = open_diff(&system, "rules.lua", (220, 24));
    let screen = session.wait_for("five columns", is_four);
    assert!(screen.contains("1 conflict"));
    assert!(screen.contains("2 conflict"));
    assert!(screen.contains("rule = a-ours"));
    assert!(screen.contains("rule = a-theirs"));
}

#[test]
fn from_100_columns_the_diff_opens_with_ours_and_theirs_side_by_side_and_the_result_below() {
    let system = rules();
    let session = open_diff(&system, "rules.lua", (120, 24));
    let screen = session.wait_for("two panes", is_two);
    assert!(!is_four(&screen));
    assert!(screen.contains("rule = a-ours"));
    assert!(screen.contains("rule = a-theirs"));
}

#[test]
fn below_100_columns_the_diff_stacks_in_one_column() {
    let system = rules();
    let session = open_diff(&system, "rules.lua", (80, 24));
    let screen = session.wait_for("a stacked column", is_stacked);
    assert!(screen.contains("- rule = a"));
    assert!(screen.contains("< ours rule = a-ours"));
}

#[test]
fn v_cycles_five_columns_two_panes_and_the_stacked_column() {
    let system = rules();
    let mut session = open_diff(&system, "rules.lua", (220, 24));
    session.wait_for("five columns", is_four);
    session.press("v");
    session.wait_for("two panes", is_two);
    session.press("v");
    session.wait_for("a stacked column", is_stacked);
    session.press("v");
    session.wait_for("five columns again", is_four);
}

#[test]
fn clicking_the_layout_hint_cycles_the_layout() {
    let system = rules();
    let mut session = open_diff(&system, "rules.lua", (220, 24));
    session.wait_for("five columns", is_four);
    session.click_text("[V] Layout");
    session.wait_for("two panes", is_two);
}

#[test]
fn resizing_across_a_width_boundary_follows_the_width_and_keeps_the_hunk() {
    let system = rules();
    let mut session = open_diff(&system, "rules.lua", (120, 24));
    session.wait_for("two panes", is_two);
    session.press("n");
    session.wait_for("the second hunk", |screen| {
        screen.contains("2 conflict") || screen.contains("z-ours")
    });
    session.resize(220, 24);
    let wide = session.wait_for("five columns after widening", is_four);
    assert_eq!(current_hunk(&wide), Some(2), "{}", wide.text());
    session.resize(80, 24);
    session.wait_for("a stacked column after narrowing", is_stacked);
    session.resize(220, 24);
    let again = session.wait_for("five columns again", is_four);
    assert_eq!(current_hunk(&again), Some(2));
}

#[test]
fn n_and_p_select_the_next_and_the_previous_hunk() {
    let system = rules();
    let mut session = open_diff(&system, "rules.lua", (220, 24));
    let first = session.wait_for("five columns", is_four);
    assert_eq!(current_hunk(&first), Some(1));
    session.press("n");
    session.wait_for("the second hunk", |screen| current_hunk(screen) == Some(2));
    session.press("p");
    session.wait_for("the first hunk", |screen| current_hunk(screen) == Some(1));
}

#[test]
fn clicking_the_next_and_previous_hunk_hints_moves_between_hunks() {
    let system = rules();
    let mut session = open_diff(&system, "rules.lua", (220, 24));
    session.wait_for("five columns", is_four);
    session.click_text("[N] Next hunk");
    session.wait_for("the second hunk", |screen| current_hunk(screen) == Some(2));
    session.click_text("[P] Prev hunk");
    session.wait_for("the first hunk", |screen| current_hunk(screen) == Some(1));
}

#[test]
fn capital_n_jumps_to_the_next_unresolved_conflict_and_says_when_there_is_none() {
    let system = rules();
    let mut session = open_diff(&system, "rules.lua", (220, 24));
    session.wait_for("five columns", is_four);
    session.press("t");
    session.wait_for("the first hunk resolved", |screen| {
        !screen.contains("● 1 unresolved")
    });
    session.press("N");
    session.wait_for("the second hunk", |screen| current_hunk(screen) == Some(2));
    session.press("t");
    session.press("N");
    let note = session.wait_for("a note that nothing is left", |screen| {
        screen
            .first()
            .contains("no unresolved conflict after this hunk")
    });
    assert_eq!(current_hunk(&note), Some(2));
}

#[test]
fn capital_p_jumps_to_the_previous_unresolved_conflict_and_says_when_there_is_none() {
    let system = rules();
    let mut session = open_diff(&system, "rules.lua", (220, 24));
    session.wait_for("five columns", is_four);
    session.press("n");
    session.wait_for("the second hunk", |screen| current_hunk(screen) == Some(2));
    session.press("P");
    session.wait_for("the first hunk", |screen| current_hunk(screen) == Some(1));
    session.press("t");
    session.press("n");
    session.wait_for("the second hunk", |screen| current_hunk(screen) == Some(2));
    session.press("P");
    let note = session.wait_for("a note that nothing is left", |screen| {
        screen
            .first()
            .contains("no unresolved conflict before this hunk")
    });
    assert_eq!(current_hunk(&note), Some(2));
}

#[test]
fn moving_to_a_hunk_scrolls_the_panes_to_it() {
    let system = rules();
    let mut session = open_diff(&system, "rules.lua", (220, 12));
    session.wait_for("the first line", |screen| screen.contains("rule = a-ours"));
    session.press("n");
    let down = session.wait_for("the last hunk in view", |screen| {
        screen.contains("rule = z-ours")
    });
    assert!(!down.contains("rule = a-ours"));
    session.press("p");
    session.wait_for("the first hunk in view", |screen| {
        screen.contains("rule = a-ours")
    });
}

#[test]
fn down_and_up_scroll_every_pane_together() {
    let system = rules();
    let mut session = open_diff(&system, "rules.lua", (220, 12));
    session.wait_for("the top", |screen| screen.contains("rule = a-ours"));
    for _ in 0..6 {
        session.press(DOWN);
    }
    let scrolled = session.wait_for("the panes scrolled", |screen| {
        !screen.contains("rule = a-ours")
    });
    let base = column_of(&scrolled, "rule = g", "rule = g");
    assert!(base.is_some());
    assert_eq!(
        scrolled
            .rows()
            .iter()
            .filter(|row| row.contains("rule = g"))
            .count(),
        1
    );
    let row = scrolled
        .rows()
        .iter()
        .find(|row| row.contains("rule = g"))
        .unwrap();
    assert_eq!(row.matches("rule = g").count(), 4, "{row}");
    for _ in 0..6 {
        session.press(UP);
    }
    session.wait_for("back at the top", |screen| screen.contains("rule = a-ours"));
}

#[test]
fn the_mouse_wheel_scrolls_the_panes() {
    let system = rules();
    let mut session = open_diff(&system, "rules.lua", (220, 12));
    session.wait_for("the top", |screen| screen.contains("rule = a-ours"));
    for _ in 0..6 {
        session.wheel_down(60, 5);
    }
    session.wait_for("scrolled by the wheel", |screen| {
        !screen.contains("rule = a-ours")
    });
    for _ in 0..6 {
        session.wheel_up(60, 5);
    }
    session.wait_for("back at the top", |screen| screen.contains("rule = a-ours"));
}

#[test]
fn q_leaves_the_diff_for_the_tree_and_the_quit_hint_does_the_same() {
    let system = rules();
    let mut session = open_diff(&system, "rules.lua", (120, 24));
    session.press("q");
    session.wait_for("the tree", |screen| screen.contains("1 of 1 files"));
    session.press(ENTER);
    session.wait_for("the diff", |screen| screen.first().starts_with("Diff"));
    session.click_text("[Q] Quit");
    session.wait_for("the tree", |screen| screen.contains("1 of 1 files"));
}

#[test]
fn bracket_keys_widen_and_narrow_the_focused_pane() {
    let system = rules();
    let mut session = open_diff(&system, "rules.lua", (220, 24));
    let before = session.wait_for("five columns", is_four);
    let theirs = column_of(&before, "hunks", "theirs").unwrap();
    session.press(TAB);
    session.press("]");
    let wider = session.wait_for("the ours pane wider", |screen| {
        column_of(screen, "hunks", "theirs").is_some_and(|column| column > theirs)
    });
    let widened = column_of(&wider, "hunks", "theirs").unwrap();
    session.press("[");
    session.press("[");
    session.wait_for("the ours pane narrower", |screen| {
        column_of(screen, "hunks", "theirs").is_some_and(|column| column < widened)
    });
}

#[test]
fn dragging_a_divider_resizes_the_panes() {
    let system = rules();
    let mut session = open_diff(&system, "rules.lua", (220, 24));
    let before = session.wait_for("five columns", is_four);
    let theirs = column_of(&before, "hunks", "theirs").unwrap();
    session.drag((theirs - 1, 5), (theirs + 11, 5));
    session.wait_for("the divider moved right", |screen| {
        column_of(screen, "hunks", "theirs").is_some_and(|column| column > theirs + 5)
    });
}

#[test]
fn tab_and_shift_tab_move_the_pane_focus() {
    let system = rules();
    let mut session = open_diff(&system, "rules.lua", (220, 24));
    let before = session.wait_for("five columns", is_four);
    let theirs = column_of(&before, "hunks", "theirs").unwrap();
    let result = column_of(&before, "hunks", "result").unwrap();
    session.press(TAB);
    session.press(TAB);
    session.press(TAB);
    session.press("]");
    let third = session.wait_for("the theirs pane widened", |screen| {
        column_of(screen, "hunks", "result").is_some_and(|column| column > result)
    });
    assert_eq!(column_of(&third, "hunks", "theirs"), Some(theirs));
    session.press(SHIFT_TAB);
    session.press("]");
    session.wait_for("the ours pane widened", |screen| {
        column_of(screen, "hunks", "theirs").is_some_and(|column| column > theirs)
    });
}

fn resolve_and_write(session: &mut Session, key: &str) {
    session.press(key);
    session.wait_for("the choice shown in the result", |screen| {
        !screen.contains("● 1 unresolved") && !screen.contains("●1 unresolved")
    });
    session.press("w");
    session.wait_for("the plan", |screen| screen.contains("Apply plan"));
    session.press("y");
    session.wait_for("the write finished", |screen| {
        !screen.contains("Apply plan")
    });
}

fn backups(system: &System) -> Vec<String> {
    let dir = system.root().join("home/.local/state/drift/backups");
    let mut found = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            found.push(entry.file_name().to_string_lossy().to_string());
        }
    }
    found
}

const MINE: &str = "bind = SUPER, A, seven\nbind = SUPER, B, two\nbind = SUPER, C, three\n";
const THEIRS: &str = "bind = SUPER, A, nine\nbind = SUPER, B, two\nbind = SUPER, C, three\n";

#[test]
fn t_takes_theirs_for_the_hunk_and_write_puts_it_on_disk_with_a_backup_of_the_original() {
    let system = bindings();
    let mut session = open_diff(&system, "bindings.lua", (120, 24));
    session.press("t");
    let result = session.wait_for("theirs in the result", |screen| {
        screen.contains("+1 bind = SUPER, A, nine")
    });
    assert!(!result.contains("●"));
    assert_eq!(system.read_live("hypr/bindings.lua").as_deref(), Some(MINE));
    resolve_and_write(&mut session, "t");
    assert_eq!(
        system.read_live("hypr/bindings.lua").as_deref(),
        Some(THEIRS)
    );
    let made = backups(&system);
    assert_eq!(made.len(), 1, "{made:?}");
    let saved = system
        .root()
        .join("home/.local/state/drift/backups")
        .join(&made[0])
        .join(".config/hypr/bindings.lua");
    assert_eq!(std::fs::read_to_string(saved).unwrap(), MINE);
}

#[test]
fn o_keeps_mine_for_the_hunk_and_the_plan_writes_nothing() {
    let system = bindings();
    let mut session = open_diff(&system, "bindings.lua", (120, 24));
    session.press("o");
    session.wait_for("mine in the result", |screen| {
        screen.contains("bind = SUPER, A, seven") && !screen.contains("●")
    });
    session.press("w");
    let plan = session.wait_for("the plan", |screen| screen.contains("Apply plan"));
    assert!(
        plan.contains("0 of 1 files will be written"),
        "{}",
        plan.text()
    );
    assert!(plan.contains("keep mine"));
    assert!(plan.contains("(not written)"));
    session.press("y");
    session.wait_for("the plan closed", |screen| !screen.contains("Apply plan"));
    assert_eq!(system.read_live("hypr/bindings.lua").as_deref(), Some(MINE));
    for made in backups(&system) {
        let copy = system
            .root()
            .join("home/.local/state/drift/backups")
            .join(made)
            .join(".config/hypr/bindings.lua");
        assert!(!copy.exists(), "{}", copy.display());
    }
}

#[test]
fn b_keeps_both_sides_in_the_result_and_write_puts_both_on_disk() {
    let system = bindings();
    let mut session = open_diff(&system, "bindings.lua", (120, 24));
    session.press("b");
    let result = session.wait_for("both sides in the result", |screen| {
        screen.contains("+1 bind = SUPER, A, seven") && screen.contains("+2 bind = SUPER, A, nine")
    });
    assert!(!result.contains("●"));
    session.press("w");
    session.wait_for("the plan", |screen| screen.contains("Apply plan"));
    session.press("y");
    session.wait_for("the write finished", |screen| {
        !screen.contains("Apply plan")
    });
    let disk = system.read_live("hypr/bindings.lua").unwrap();
    assert_eq!(
        disk,
        "bind = SUPER, A, seven\nbind = SUPER, A, nine\nbind = SUPER, B, two\nbind = SUPER, C, three\n"
    );
}

#[test]
fn e_hands_the_hunk_to_the_editor_and_write_puts_what_it_saved_on_disk() {
    let system = bindings();
    let mut session = open_diff(&system, "bindings.lua", (120, 24));
    session.press("e");
    session.wait_for("the edited text in the result", |screen| {
        screen.contains("bind = SUPER, A, edited")
    });
    session.press("w");
    session.wait_for("the plan", |screen| screen.contains("Apply plan"));
    session.press("y");
    session.wait_for("the write finished", |screen| {
        !screen.contains("Apply plan")
    });
    assert_eq!(
        system.read_live("hypr/bindings.lua").as_deref(),
        Some("bind = SUPER, A, edited\nbind = SUPER, B, two\nbind = SUPER, C, three\n")
    );
}

#[test]
fn the_choice_keys_have_click_targets_in_the_header() {
    let system = bindings();
    let mut session = open_diff(&system, "bindings.lua", (120, 24));
    session.click_text("[t] theirs");
    session.wait_for("theirs in the result", |screen| {
        screen.contains("+1 bind = SUPER, A, nine")
    });
    session.click_text("[o] mine");
    session.wait_for("mine in the result", |screen| {
        screen.contains("bind = SUPER, A, seven") && !screen.contains("+1 bind = SUPER, A, nine")
    });
}

#[test]
fn the_write_hint_opens_the_plan_for_the_result() {
    let system = bindings();
    let mut session = open_diff(&system, "bindings.lua", (120, 24));
    session.press("t");
    session.wait_for("theirs in the result", |screen| {
        screen.contains("+1 bind = SUPER, A, nine")
    });
    session.click_text("[W] Write");
    let plan = session.wait_for("the plan", |screen| screen.contains("Apply plan"));
    assert!(plan.contains(".config/hypr/bindings.lua"));
}
