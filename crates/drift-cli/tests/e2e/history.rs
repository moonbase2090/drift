use crate::harness::*;
use crate::system::System;
use crate::view::*;

const FILES: [&str; 3] = ["looknfeel.lua", "autostart.lua", "monitors.lua"];

fn take_in_a_new_session(system: &System, file: &str) {
    let mut session = system.open();
    session.wait_for_text(file);
    session.click_text(file);
    session.wait_for("the file selected", |screen| selected_is(screen, file));
    session.press("t");
    session.wait_for("the plan", |screen| screen.contains("Apply plan"));
    session.press("y");
    session.wait_for("the write finished", |screen| {
        !screen.contains("Apply plan") && screen.contains("Applied ")
    });
}

fn after_three_takes(system: &System, size: (u16, u16)) -> Session {
    for file in FILES {
        take_in_a_new_session(system, file);
    }
    let session = system.launch(&[], size);
    session.wait_for_text("8 of 8 files");
    session
}

fn history_after_three_takes(system: &System, size: (u16, u16)) -> Session {
    let mut session = after_three_takes(system, size);
    session.press("H");
    session.wait_for("the history", |screen| {
        screen.contains("Undo and force") && screen.contains("3 transactions")
    });
    session
}

fn ids(screen: &Screen) -> Vec<String> {
    screen
        .rows()
        .iter()
        .filter_map(|row| {
            let row = row.trim_start_matches('>').trim_start();
            row.starts_with("resolve-")
                .then(|| row.split_whitespace().next().unwrap().to_string())
        })
        .collect()
}

fn selected_id(screen: &Screen) -> Option<String> {
    screen
        .rows()
        .iter()
        .find(|row| row.starts_with(">resolve-"))
        .map(|row| row[1..].split_whitespace().next().unwrap().to_string())
}

fn is_selected(session: &Session, id: &str) -> impl Fn(&Screen) -> bool {
    let id = id.to_string();
    let _ = session;
    move |screen| selected_id(screen).as_deref() == Some(id.as_str())
}

#[test]
fn h_opens_the_history_listing_every_transaction_newest_first() {
    let system = System::world();
    let session = history_after_three_takes(&system, (100, 30));
    let screen = session.screen();
    let listed = ids(&screen);
    assert_eq!(listed.len(), 3, "{}", screen.text());
    let mut newest_first = listed.clone();
    newest_first.sort();
    newest_first.reverse();
    assert_eq!(listed, newest_first);
    assert_eq!(selected_id(&screen), Some(listed[0].clone()));
}

#[test]
fn j_and_down_move_down_and_k_and_up_move_up() {
    let system = System::world();
    let mut session = history_after_three_takes(&system, (100, 30));
    let listed = ids(&session.screen());
    session.press("j");
    session.wait_for("the second", is_selected(&session, &listed[1]));
    session.press(DOWN);
    session.wait_for("the third", is_selected(&session, &listed[2]));
    session.press(DOWN);
    session.wait_for("still the third", is_selected(&session, &listed[2]));
    session.press("k");
    session.wait_for("the second again", is_selected(&session, &listed[1]));
    session.press(UP);
    session.wait_for("the first again", is_selected(&session, &listed[0]));
    session.press("k");
    session.wait_for("still the first", is_selected(&session, &listed[0]));
}

#[test]
fn g_and_home_select_the_newest_and_capital_g_and_end_the_oldest() {
    let system = System::world();
    let mut session = history_after_three_takes(&system, (100, 30));
    let listed = ids(&session.screen());
    session.press("G");
    session.wait_for("the oldest", is_selected(&session, &listed[2]));
    session.press("g");
    session.wait_for("the newest", is_selected(&session, &listed[0]));
    session.press(END);
    session.wait_for("the oldest by End", is_selected(&session, &listed[2]));
    session.press(HOME);
    session.wait_for("the newest by Home", is_selected(&session, &listed[0]));
}

#[test]
fn page_down_and_page_up_move_by_half_the_history_pane() {
    let system = System::world();
    let mut session = history_after_three_takes(&system, (100, 8));
    let listed = ids(&session.screen());
    session.press(PAGE_DOWN);
    session.wait_for("two rows down", is_selected(&session, &listed[2]));
    session.press(PAGE_UP);
    session.wait_for("two rows up", is_selected(&session, &listed[0]));
}

#[test]
fn clicking_a_row_selects_it() {
    let system = System::world();
    let mut session = history_after_three_takes(&system, (100, 30));
    let listed = ids(&session.screen());
    session.click_text(&listed[2]);
    session.wait_for("the clicked row", is_selected(&session, &listed[2]));
}

#[test]
fn q_and_escape_return_to_the_tree() {
    let system = System::world();
    let mut session = history_after_three_takes(&system, (100, 30));
    session.press("q");
    session.wait_for("the tree", |screen| {
        screen.contains("[Q] Quit") && !screen.contains("Undo and force")
    });
    session.press("H");
    session.wait_for("the history", |screen| screen.contains("Undo and force"));
    session.press(ESC);
    session.wait_for("the tree again", |screen| {
        screen.contains("[Q] Quit") && !screen.contains("Undo and force")
    });
}

#[test]
fn r_reloads_the_list_and_shows_what_appeared_on_disk() {
    let system = System::world();
    let mut session = history_after_three_takes(&system, (100, 30));
    assert!(!session.screen().contains("could not be read"));
    let listed = ids(&session.screen());
    system.write(
        &format!(
            "home/.local/state/drift/backups/{}/manifest.json",
            listed[2]
        ),
        "not a manifest",
    );
    session.press("r");
    session.wait_for("the unreadable entry", |screen| {
        screen.contains(&format!(
            "{}  (backups/{}/manifest.json could not be read)",
            listed[2], listed[2]
        ))
    });
}
