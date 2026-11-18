use crate::harness::*;
use crate::system::System;
use crate::view::*;

fn ignored_file(system: &System) -> Option<String> {
    std::fs::read_to_string(system.home().join(".local/state/drift/ignored")).ok()
}

fn opened(system: &System) -> Session {
    let session = system.open();
    session.wait_for_text("looknfeel.lua");
    session
}

#[test]
fn i_ignores_the_selected_file_and_records_it_in_the_state_folder() {
    let system = System::world();
    let mut session = opened(&system);
    session.click_text("looknfeel.lua");
    session.wait_for("selected", |screen| selected_is(screen, "looknfeel.lua"));
    session.press("i");
    let after = session.wait_for("the file left out", |screen| {
        screen.contains("Ignored 1 file(s).") && !has_row(screen, "looknfeel.lua")
    });
    assert!(has_row(&after, "autostart.lua"), "{}", after.text());
    assert_eq!(
        ignored_file(&system).as_deref(),
        Some(".config/hypr/looknfeel.lua\n")
    );
    assert!(system.read_live("hypr/looknfeel.lua").is_some());
}

#[test]
fn i_ignores_every_marked_file_and_not_the_selected_one() {
    let system = System::world();
    let mut session = opened(&system);
    session.click_text("autostart.lua");
    session.wait_for("selected", |screen| selected_is(screen, "autostart.lua"));
    session.press(" ");
    session.click_text("monitors.lua");
    session.wait_for("selected", |screen| selected_is(screen, "monitors.lua"));
    session.press(" ");
    session.click_text("input.lua");
    session.wait_for("two marked and another selected", |screen| {
        screen.contains("2 marked") && selected_is(screen, "input.lua")
    });
    session.press("i");
    let after = session.wait_for("the marked files left out", |screen| {
        screen.contains("Ignored 2 file(s).")
            && !screen.contains("scanning")
            && !has_row(screen, "autostart.lua")
    });
    assert!(!has_row(&after, "autostart.lua"), "{}", after.text());
    assert!(!has_row(&after, "monitors.lua"));
    assert!(has_row(&after, "input.lua"));
    assert_eq!(
        ignored_file(&system).as_deref(),
        Some(".config/hypr/autostart.lua\n.config/hypr/monitors.lua\n")
    );
}

#[test]
fn an_ignored_file_stays_out_of_the_list_in_the_next_session() {
    let system = System::world();
    let mut first = opened(&system);
    first.click_text("looknfeel.lua");
    first.wait_for("selected", |screen| selected_is(screen, "looknfeel.lua"));
    first.press("i");
    first.wait_for("ignored", |screen| screen.contains("Ignored 1 file(s)."));
    drop(first);
    let second = system.open();
    second.wait_for("the list without it", |screen| {
        has_row(screen, "autostart.lua") && !has_row(screen, "looknfeel.lua")
    });
}

#[test]
fn the_palette_ignore_command_ignores_the_selected_file() {
    let system = System::world();
    let mut session = opened(&system);
    session.click_text("looknfeel.lua");
    session.wait_for("selected", |screen| selected_is(screen, "looknfeel.lua"));
    session.press(":");
    session.wait_for("the palette", |screen| palette_count(screen).is_some());
    for letter in "ignore".chars() {
        session.press(&letter.to_string());
    }
    session.wait_for("the ignore command first", |screen| {
        palette_first(screen).is_some_and(|row| row.starts_with("Ignore"))
    });
    session.press(ENTER);
    session.wait_for("ignored", |screen| {
        screen.contains("Ignored 1 file(s).") && !has_row(screen, "looknfeel.lua")
    });
    assert_eq!(
        ignored_file(&system).as_deref(),
        Some(".config/hypr/looknfeel.lua\n")
    );
}
