use std::fs::File;
use std::os::fd::AsRawFd;
use std::path::PathBuf;

use drift_fixtures::hash_tree;

use crate::harness::*;
use crate::system::System;
use crate::view::*;

const LOOKNFEEL_OURS: &str = "gaps = 5\nrounding = 2\n";
const LOOKNFEEL_THEIRS: &str = "gaps = 8\nrounding = 2\n";

fn opened(system: &System) -> Session {
    let session = system.open();
    session.wait_for_text("looknfeel.lua");
    session
}

fn plan_for_looknfeel(system: &System, key: &str) -> (Session, Screen) {
    let mut session = opened(system);
    session.click_text("looknfeel.lua");
    session.wait_for("looknfeel selected", |screen| {
        selected_is(screen, "looknfeel.lua")
    });
    session.press(key);
    let plan = session.wait_for("the plan", |screen| screen.contains("Apply plan"));
    (session, plan)
}

fn plan_for_everything(system: &System) -> (Session, Screen) {
    let mut session = opened(system);
    session.press("a");
    session.wait_for("everything marked", |screen| {
        mark_of(screen, "looknfeel.lua") == Some('x')
    });
    session.press("t");
    let plan = session.wait_for("the plan", |screen| screen.contains("Apply plan"));
    (session, plan)
}

fn backups_dir(system: &System) -> PathBuf {
    system.home().join(".local/state/drift/backups")
}

fn transactions(system: &System) -> Vec<String> {
    let mut found: Vec<String> = std::fs::read_dir(backups_dir(system))
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().to_string())
                .collect()
        })
        .unwrap_or_default();
    found.sort();
    found
}

fn backup_of(system: &System, transaction: &str, rel: &str) -> Option<String> {
    std::fs::read_to_string(backups_dir(system).join(transaction).join(rel)).ok()
}

fn held(path: PathBuf) -> File {
    let file = File::open(path).unwrap();
    let locked = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    assert_eq!(locked, 0);
    file
}

fn row_of(screen: &Screen, needle: &str) -> Option<String> {
    screen
        .rows()
        .iter()
        .find(|row| row.contains(needle))
        .cloned()
}

fn squash(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn status_row(screen: &Screen, status: &str, name: &str) -> bool {
    screen
        .rows()
        .iter()
        .any(|row| squash(row).contains(&format!("{status} {name}")))
}

fn plan_selected(screen: &Screen, needle: &str) -> bool {
    screen
        .rows()
        .iter()
        .any(|row| row.contains("> [") && row.contains(needle))
}

fn confirm_and_wait(session: &mut Session) -> Screen {
    session.press("y");
    session.wait_for("the write finished", |screen| {
        !screen.contains("Apply plan") && screen.contains("Applied ")
    })
}

#[test]
fn the_plan_for_an_existing_file_names_the_action_the_real_path_and_the_backup() {
    let system = System::world();
    let (_session, plan) = plan_for_looknfeel(&system, "t");
    assert!(
        plan.contains("1 of 1 files will be written"),
        "{}",
        plan.text()
    );
    assert!(
        plan.contains("[+] take upstream .config/hypr/looknfeel.lua  (+1 -1)"),
        "{}",
        plan.text()
    );
    let backup = row_of(&plan, "backup:").unwrap();
    assert!(backup.contains("/backups/resolve-"), "{backup}");
    assert!(backup.contains(".config/hypr/looknfeel.lua"), "{backup}");
    assert!(!plan.contains("none (new file)"), "{}", plan.text());
    assert!(!plan.contains(" config/hypr"), "{}", plan.text());
}

#[test]
fn the_plan_for_a_missing_file_reads_new_file_and_has_no_backup() {
    let system = System::world();
    let mut session = opened(&system);
    session.press("G");
    session.wait_for("waybar selected", |screen| selected_is(screen, "waybar/"));
    session.press("t");
    let plan = session.wait_for("the plan", |screen| screen.contains("Apply plan"));
    assert!(
        plan.contains("take upstream .config/waybar/config.jsonc"),
        "{}",
        plan.text()
    );
    assert!(plan.contains("(new file, 1 lines)"), "{}", plan.text());
    assert!(plan.contains("backup: none (new file)"), "{}", plan.text());
}

#[test]
fn y_confirms_and_writes_the_file_a_backup_and_one_transaction() {
    let system = System::world();
    let (mut session, _plan) = plan_for_looknfeel(&system, "t");
    assert_eq!(
        system.read_live("hypr/looknfeel.lua").as_deref(),
        Some(LOOKNFEEL_OURS)
    );
    assert!(transactions(&system).is_empty());
    let done = confirm_and_wait(&mut session);
    assert!(
        done.contains("Applied 1 file(s) as transaction resolve-"),
        "{}",
        done.text()
    );
    assert_eq!(
        system.read_live("hypr/looknfeel.lua").as_deref(),
        Some(LOOKNFEEL_THEIRS)
    );
    let made = transactions(&system);
    assert_eq!(made.len(), 1, "{made:?}");
    assert_eq!(
        backup_of(&system, &made[0], ".config/hypr/looknfeel.lua").as_deref(),
        Some(LOOKNFEEL_OURS)
    );
    assert!(done.contains(&made[0]), "{}", done.text());
}

#[test]
fn after_a_confirmed_write_the_list_refreshes_in_place_and_keeps_the_selection() {
    let system = System::world();
    let (mut session, _plan) = plan_for_looknfeel(&system, "t");
    let mark = session.mark();
    session.press("y");
    let done = session.wait_for("the row reads unchanged", |screen| {
        status_row(screen, "✓ unchanged", "looknfeel.lua")
    });
    assert!(selected_is(&done, "looknfeel.lua"), "{}", done.text());
    assert!(has_row(&done, "autostart.lua"));
    assert!(has_row(&done, "bindings.lua"));
    for frame in session.frames_since(mark) {
        if frame.contains("Apply plan") || frame.contains(" Drift ") {
            continue;
        }
        assert!(
            has_row(&frame, "autostart.lua") || frame.contains("Scanning"),
            "{}",
            frame.text()
        );
    }
}

#[test]
fn enter_on_the_confirm_button_writes_after_tab_moves_the_focus_to_it() {
    let system = System::world();
    let (mut session, plan) = plan_for_looknfeel(&system, "t");
    assert!(plan.contains("[>Cancel<]"), "{}", plan.text());
    session.press(TAB);
    session.wait_for("confirm focused", |screen| screen.contains("[>Confirm<]"));
    session.press(ENTER);
    session.wait_for("the write finished", |screen| screen.contains("Applied "));
    assert_eq!(
        system.read_live("hypr/looknfeel.lua").as_deref(),
        Some(LOOKNFEEL_THEIRS)
    );
    assert_eq!(transactions(&system).len(), 1);
}

#[test]
fn clicking_the_confirm_button_writes() {
    let system = System::world();
    let (mut session, _plan) = plan_for_looknfeel(&system, "t");
    session.click_text("[ Confirm ]");
    session.wait_for("the write finished", |screen| screen.contains("Applied "));
    assert_eq!(
        system.read_live("hypr/looknfeel.lua").as_deref(),
        Some(LOOKNFEEL_THEIRS)
    );
    assert_eq!(transactions(&system).len(), 1);
}

#[test]
fn a_plan_for_everything_writes_every_listed_file_and_creates_the_missing_one() {
    let system = System::world();
    let (mut session, plan) = plan_for_everything(&system);
    assert!(
        plan.contains("6 of 6 files will be written"),
        "{}",
        plan.text()
    );
    confirm_and_wait(&mut session);
    for (rel, theirs) in [
        ("hypr/autostart.lua", crate::system::CLEAN_MERGE.theirs),
        ("hypr/bindings.lua", crate::system::CONFLICT.theirs),
        ("hypr/looknfeel.lua", LOOKNFEEL_THEIRS),
        ("hypr/monitors.lua", crate::system::OVERRIDE.theirs),
        ("hypr/windows.lua", crate::system::NO_BASE.theirs),
        ("waybar/config.jsonc", crate::system::MISSING.theirs),
    ] {
        assert_eq!(system.read_live(rel).as_deref(), Some(theirs), "{rel}");
    }
    assert_eq!(
        system.read_live("hypr/mine.lua").as_deref(),
        Some("-- only mine\n")
    );
    assert_eq!(
        system.read_live("hypr/input.lua").as_deref(),
        Some(crate::system::UNCHANGED.ours.unwrap())
    );
    let made = transactions(&system);
    assert_eq!(made.len(), 1, "{made:?}");
    assert_eq!(
        backup_of(&system, &made[0], ".config/hypr/monitors.lua").as_deref(),
        crate::system::OVERRIDE.ours
    );
    assert_eq!(
        backup_of(&system, &made[0], ".config/waybar/config.jsonc"),
        None
    );
}

fn cancelled(how: &str) {
    let system = System::world();
    let (mut session, _plan) = plan_for_looknfeel(&system, "t");
    let before = hash_tree(system.root()).unwrap();
    match how {
        "n" => session.press("n"),
        "q" => session.press("q"),
        "esc" => session.press(ESC),
        "enter" => session.press(ENTER),
        _ => session.click_text("[>Cancel<]"),
    }
    let after = session.wait_for("the plan closed", |screen| !screen.contains("Apply plan"));
    assert!(
        after.contains("Cancelled. Nothing was written."),
        "{how}: {}",
        after.text()
    );
    assert_eq!(hash_tree(system.root()).unwrap(), before, "{how}");
    assert_eq!(
        system.read_live("hypr/looknfeel.lua").as_deref(),
        Some(LOOKNFEEL_OURS)
    );
    assert!(transactions(&system).is_empty(), "{how}");
    session.press("q");
    assert_eq!(session.wait_exit(), 0, "{how}");
}

#[test]
fn n_cancels_and_changes_nothing() {
    cancelled("n");
}

#[test]
fn q_cancels_the_plan_instead_of_quitting_and_changes_nothing() {
    cancelled("q");
}

#[test]
fn escape_cancels_and_changes_nothing() {
    cancelled("esc");
}

#[test]
fn enter_on_the_default_cancel_button_changes_nothing() {
    cancelled("enter");
}

#[test]
fn clicking_the_cancel_button_changes_nothing() {
    cancelled("click");
}

#[test]
fn tab_arrows_h_and_l_move_between_the_buttons() {
    let system = System::world();
    let (mut session, plan) = plan_for_looknfeel(&system, "t");
    assert!(plan.contains("[>Cancel<]"));
    for (key, focused) in [
        (TAB, "[>Confirm<]"),
        (TAB, "[>Cancel<]"),
        (RIGHT, "[>Confirm<]"),
        (LEFT, "[>Cancel<]"),
        ("l", "[>Confirm<]"),
        ("h", "[>Cancel<]"),
        (SHIFT_TAB, "[>Confirm<]"),
    ] {
        session.press(key);
        session.wait_for(focused, |screen| screen.contains(focused));
    }
    assert!(transactions(&system).is_empty());
    assert_eq!(
        system.read_live("hypr/looknfeel.lua").as_deref(),
        Some(LOOKNFEEL_OURS)
    );
}

#[test]
fn j_k_arrows_home_and_end_select_a_file_in_the_plan() {
    let system = System::world();
    let (mut session, plan) = plan_for_everything(&system);
    assert!(plan_selected(&plan, "autostart.lua"), "{}", plan.text());
    session.press("j");
    session.wait_for("bindings", |screen| plan_selected(screen, "bindings.lua"));
    session.press(DOWN);
    session.wait_for("looknfeel", |screen| plan_selected(screen, "looknfeel.lua"));
    session.press("k");
    session.wait_for("bindings again", |screen| {
        plan_selected(screen, "bindings.lua")
    });
    session.press(UP);
    session.wait_for("autostart again", |screen| {
        plan_selected(screen, "autostart.lua")
    });
    session.press(END);
    session.wait_for("last", |screen| {
        plan_selected(screen, "waybar/config.jsonc")
    });
    session.press(HOME);
    session.wait_for("first", |screen| plan_selected(screen, "autostart.lua"));
}

#[test]
fn clicking_a_file_in_the_plan_selects_it() {
    let system = System::world();
    let (mut session, _plan) = plan_for_everything(&system);
    session.click_text("take upstream .config/hypr/monitors.lua");
    session.wait_for("monitors selected", |screen| {
        plan_selected(screen, "monitors.lua")
    });
}

#[test]
fn d_expands_the_selected_files_diff_and_a_second_d_collapses_it() {
    let system = System::world();
    let (mut session, _plan) = plan_for_everything(&system);
    session.press("j");
    session.wait_for("bindings", |screen| plan_selected(screen, "bindings.lua"));
    session.press("d");
    let open = session.wait_for("the diff is expanded", |screen| {
        screen.contains("+bind = SUPER, A, nine")
    });
    assert!(open.contains("-bind = SUPER, A, seven"), "{}", open.text());
    assert!(open.contains("> [-] take upstream .config/hypr/bindings.lua"));
    assert!(!open.contains("-rounding"));
    session.press("d");
    session.wait_for("collapsed", |screen| {
        !screen.contains("+bind = SUPER, A, nine")
    });
}

#[test]
fn a_held_update_lock_refuses_the_write_and_changes_nothing() {
    let system = System::world();
    system.write("runtime/omarchy-update.lock", b"");
    let lock = held(system.root().join("runtime/omarchy-update.lock"));
    let (mut session, _plan) = plan_for_looknfeel(&system, "t");
    let before = hash_tree(system.root()).unwrap();
    session.press("y");
    let refused = session.wait_for("the refusal", |screen| {
        screen.contains("omarchy update lock is held")
    });
    assert!(!refused.contains("Applied "), "{}", refused.text());
    assert!(!refused.contains("Apply plan"), "{}", refused.text());
    assert_eq!(hash_tree(system.root()).unwrap(), before);
    assert_eq!(
        system.read_live("hypr/looknfeel.lua").as_deref(),
        Some(LOOKNFEEL_OURS)
    );
    assert!(transactions(&system).is_empty());
    drop(lock);
}

#[test]
fn a_stale_update_lock_file_that_nobody_holds_does_not_refuse_the_write() {
    let system = System::world();
    system.write("runtime/omarchy-update.lock", b"");
    let (mut session, _plan) = plan_for_looknfeel(&system, "t");
    let done = confirm_and_wait(&mut session);
    assert!(!done.contains("lock is held"), "{}", done.text());
    assert_eq!(
        system.read_live("hypr/looknfeel.lua").as_deref(),
        Some(LOOKNFEEL_THEIRS)
    );
    assert_eq!(transactions(&system).len(), 1);
}

#[test]
fn pacmans_database_lock_refuses_the_write_and_changes_nothing() {
    let system = System::world();
    system.write("var/lib/pacman/db.lck", b"");
    let (mut session, _plan) = plan_for_looknfeel(&system, "t");
    let before = hash_tree(system.root()).unwrap();
    session.press("y");
    let refused = session.wait_for("the refusal", |screen| {
        screen.contains("pacman database lock is held")
    });
    assert!(!refused.contains("Applied "), "{}", refused.text());
    assert_eq!(hash_tree(system.root()).unwrap(), before);
    assert!(transactions(&system).is_empty());
}

#[test]
fn a_file_changed_after_the_plan_was_made_is_refused_and_left_as_it_is() {
    let system = System::world();
    let (mut session, _plan) = plan_for_looknfeel(&system, "t");
    system.write("home/.config/hypr/looknfeel.lua", "gaps = 99\n");
    session.press("y");
    let refused = session.wait_for("the refusal", |screen| {
        !screen.contains("Apply plan") && !screen.contains("Cancelled")
    });
    assert!(!refused.contains("Applied "), "{}", refused.text());
    assert_eq!(
        system.read_live("hypr/looknfeel.lua").as_deref(),
        Some("gaps = 99\n")
    );
    assert!(
        refused.contains("stale hash for .config/hypr/looknfeel.lua"),
        "{}",
        refused.text()
    );
    assert_eq!(transactions(&system), Vec::<String>::new());
    session.press("H");
    let history = session.wait_for("the empty history", |screen| {
        screen.contains("No transactions yet.")
    });
    assert!(!history.contains("could not be read"), "{}", history.text());
}

fn base_of(system: &System, rel: &str) -> Option<String> {
    std::fs::read_to_string(
        system
            .home()
            .join(".local/share/drift/bases/config")
            .join(rel),
    )
    .ok()
}

fn after_a_take(system: &System) -> Session {
    let (mut session, _plan) = plan_for_looknfeel(system, "t");
    confirm_and_wait(&mut session);
    assert_eq!(
        system.read_live("hypr/looknfeel.lua").as_deref(),
        Some(LOOKNFEEL_THEIRS)
    );
    assert_eq!(
        base_of(system, "hypr/looknfeel.lua").as_deref(),
        Some(LOOKNFEEL_THEIRS)
    );
    session
}

fn history_of(session: &mut Session) -> Screen {
    session.press("H");
    session.wait_for("the history", |screen| screen.contains("Undo and force"))
}

#[test]
fn enter_in_the_history_undoes_a_transaction_and_restores_the_saved_bytes() {
    let system = System::world();
    let mut session = after_a_take(&system);
    history_of(&mut session);
    session.press(ENTER);
    let plan = session.wait_for("the undo plan", |screen| {
        screen.contains("Apply plan undo-")
    });
    assert!(
        squash(&plan.text())
            .contains("undo .config/hypr/looknfeel.lua (restore the saved original)"),
        "{}",
        plan.text()
    );
    assert!(
        plan.contains("2 of 2 files will be written"),
        "{}",
        plan.text()
    );
    session.press("y");
    session.wait_for("two transactions", |screen| {
        screen.contains("2 transactions")
    });
    assert_eq!(
        system.read_live("hypr/looknfeel.lua").as_deref(),
        Some(LOOKNFEEL_OURS)
    );
    assert_eq!(
        base_of(&system, "hypr/looknfeel.lua").as_deref(),
        Some(LOOKNFEEL_OURS)
    );
    assert_eq!(transactions(&system).len(), 2);
}

#[test]
fn u_in_the_tree_opens_the_undo_plan_for_the_newest_transaction() {
    let system = System::world();
    let mut session = after_a_take(&system);
    session.press("u");
    let plan = session.wait_for("the undo plan", |screen| {
        screen.contains("Apply plan undo-")
    });
    assert!(
        squash(&plan.text()).contains(".config/hypr/looknfeel.lua (restore the saved original)"),
        "{}",
        plan.text()
    );
    session.press("y");
    session.wait_for("the row reads upstream changed again", |screen| {
        status_row(screen, "↓ upstream changed", "looknfeel.lua")
    });
    assert_eq!(
        system.read_live("hypr/looknfeel.lua").as_deref(),
        Some(LOOKNFEEL_OURS)
    );
}

#[test]
fn undo_refuses_a_file_changed_since_and_capital_u_restores_it_anyway() {
    let system = System::world();
    let mut session = after_a_take(&system);
    system.write("home/.config/hypr/looknfeel.lua", "gaps = 77\n");
    history_of(&mut session);
    session.press(ENTER);
    session.wait_for("the undo plan", |screen| {
        screen.contains("Apply plan undo-")
    });
    session.press("y");
    let refused = session.wait_for("the refusal", |screen| {
        screen.contains("changed since") && !screen.contains("Apply plan")
    });
    assert!(
        refused.contains("nothing was written"),
        "{}",
        refused.text()
    );
    assert_eq!(
        system.read_live("hypr/looknfeel.lua").as_deref(),
        Some("gaps = 77\n")
    );
    session.press("U");
    let plan = session.wait_for("the forced plan", |screen| {
        screen.contains("Apply plan undo-")
    });
    assert!(
        plan.contains(".config/hypr/looknfeel.lua"),
        "{}",
        plan.text()
    );
    session.press("y");
    session.wait_for("two transactions", |screen| {
        screen.contains("2 transactions")
    });
    assert_eq!(
        system.read_live("hypr/looknfeel.lua").as_deref(),
        Some(LOOKNFEEL_OURS)
    );
}

#[test]
fn o_keeps_mine_writes_nothing_and_records_the_template_as_reviewed() {
    let system = System::world();
    let (mut session, plan) = plan_for_looknfeel(&system, "o");
    assert!(
        plan.contains("0 of 1 files will be written"),
        "{}",
        plan.text()
    );
    assert!(plan.contains("keep mine"), "{}", plan.text());
    assert!(
        plan.contains("backup: none (nothing is written)"),
        "{}",
        plan.text()
    );
    session.press("y");
    session.wait_for("the row reads local override", |screen| {
        status_row(screen, "≠ local override", "looknfeel.lua")
    });
    assert_eq!(
        system.read_live("hypr/looknfeel.lua").as_deref(),
        Some(LOOKNFEEL_OURS)
    );
    assert_eq!(
        base_of(&system, "hypr/looknfeel.lua").as_deref(),
        Some(LOOKNFEEL_THEIRS)
    );
}

#[test]
fn m_merges_a_clean_merge_and_writes_the_combined_text_with_a_backup() {
    let system = System::world();
    let mut session = opened(&system);
    session.click_text("autostart.lua");
    session.wait_for("autostart selected", |screen| {
        selected_is(screen, "autostart.lua")
    });
    session.press("m");
    let plan = session.wait_for("the plan", |screen| screen.contains("Apply plan"));
    assert!(
        squash(&plan.text()).contains("merge .config/hypr/autostart.lua"),
        "{}",
        plan.text()
    );
    confirm_and_wait(&mut session);
    assert_eq!(
        system.read_live("hypr/autostart.lua").as_deref(),
        Some("exec A\nexec b\nexec c\nexec d\nexec e\nexec f\nexec g\nexec H\n")
    );
    let made = transactions(&system);
    assert_eq!(made.len(), 1, "{made:?}");
    assert_eq!(
        backup_of(&system, &made[0], ".config/hypr/autostart.lua").as_deref(),
        crate::system::CLEAN_MERGE.ours
    );
}
