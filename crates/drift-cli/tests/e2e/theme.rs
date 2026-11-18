use std::collections::BTreeSet;

use crate::harness::{Launch, Session, ENTER, ESC};
use crate::system::{System, UPSTREAM};
use crate::view::{mark_of, selected_is};

fn colors(system: &System, mode: &str, green: &str) {
    system.put(
        "home/.local/state/omarchy/current/theme/colors.toml",
        format!(
            "mode = \"{mode}\"\ngreen = \"{green}\"\nred = \"#ff0000\"\nyellow = \"#ffff00\"\n"
        ),
    );
}

fn open(system: &System) -> Session {
    let mut env = system.env();
    env.push(("COLORTERM".into(), "truecolor".into()));
    Session::start(Launch {
        program: env!("CARGO_BIN_EXE_drift"),
        args: vec![],
        env,
        size: (200, 40),
    })
}

fn rgb(color: vt100::Color) -> [u8; 3] {
    match color {
        vt100::Color::Rgb(r, g, b) => [r, g, b],
        other => panic!("expected truecolor from the terminal, got {other:?}"),
    }
}

fn luminance([r, g, b]: [u8; 3]) -> f64 {
    [r, g, b]
        .into_iter()
        .zip([0.2126, 0.7152, 0.0722])
        .map(|(c, weight)| {
            let value = f64::from(c) / 255.0;
            weight
                * if value <= 0.04045 {
                    value / 12.92
                } else {
                    ((value + 0.055) / 1.055).powf(2.4)
                }
        })
        .sum()
}

fn mode_is(screen: &vt100::Screen, light: bool) -> bool {
    matches!(screen.cell(0, 0).map(|cell| cell.bgcolor()), Some(vt100::Color::Rgb(r,g,b)) if if light { luminance([r,g,b]) > 0.6 } else { luminance([r,g,b]) < 0.1 })
}

fn cell_at_text<'a>(screen: &'a vt100::Screen, needle: &str) -> Option<&'a vt100::Cell> {
    screen
        .rows(0, screen.size().1)
        .enumerate()
        .find_map(|(row, text)| {
            let at = text.find(needle)?;
            screen.cell(row as u16, text[..at].chars().count() as u16)
        })
}

fn legible_statuses(screen: &vt100::Screen) {
    let mut distinct = BTreeSet::new();
    for label in [
        "unchanged",
        "upstream changed",
        "local override",
        "conflict",
        "user-only",
        "missing",
    ] {
        let cell = cell_at_text(screen, label)
            .unwrap_or_else(|| panic!("missing {label}: {}", screen.contents()));
        let foreground = rgb(cell.fgcolor());
        let a = luminance(foreground);
        let b = luminance(rgb(cell.bgcolor()));
        let contrast = (a.max(b) + 0.05) / (a.min(b) + 0.05);
        assert!(contrast >= 4.5, "{label}: contrast {contrast}, {cell:?}");
        if ["unchanged", "upstream changed", "conflict"].contains(&label) {
            distinct.insert(foreground);
        }
    }
    assert_eq!(
        distinct.len(),
        3,
        "success, upstream change and conflict need distinct colours"
    );
}

#[test]
fn dark_and_light_theme_switches_repaint_legible_statuses_without_resetting_selection() {
    let system = System::world();
    colors(&system, "dark", "#00ff00");
    let mut session = open(&system);
    session.wait_for_text("looknfeel.lua");
    session.press("jjj");
    session.wait_for("bindings selected before changing theme", |screen| {
        selected_is(screen, "bindings.lua")
    });
    session.press(" ");
    session.wait_for("bindings marked before changing theme", |screen| {
        mark_of(screen, "bindings.lua") == Some('x')
    });
    legible_statuses(
        &session.wait_for_colors("dark status colours", |screen| mode_is(screen, false)),
    );
    for (mode, light) in [("light", true), ("dark", false)] {
        colors(&system, mode, "#00ff00");
        session.press("x");
        legible_statuses(
            &session.wait_for_colors(&format!("{mode} status colours"), |screen| {
                mode_is(screen, light)
            }),
        );
        let visible = session.screen();
        assert!(
            selected_is(&visible, "bindings.lua"),
            "{mode}: {}",
            visible.text()
        );
        assert_eq!(
            mark_of(&visible, "bindings.lua"),
            Some('x'),
            "{mode}: {}",
            visible.text()
        );
    }
    session.press("q");
    assert_eq!(session.wait_exit(), 0);
}

fn hue_is(screen: &vt100::Screen, magenta: bool) -> bool {
    let Some(cell) = cell_at_text(screen, "gaps = 8") else {
        return false;
    };
    let vt100::Color::Rgb(r, g, b) = cell.bgcolor() else {
        return false;
    };
    if magenta {
        i16::from(r) - i16::from(g) > 40 && i16::from(b) - i16::from(g) > 40
    } else {
        i16::from(g) - i16::from(r) > 40 && i16::from(g) - i16::from(b) > 30
    }
}

#[test]
fn the_diff_uses_omarchy_hues_and_repaints_its_cached_frame_when_mode_changes() {
    let system = System::builder().template(&UPSTREAM).build();
    colors(&system, "dark", "#00ff00");
    let mut session = open(&system);
    session.wait_for_text("looknfeel.lua");
    session.press("jj");
    session.wait_for("file selected", |screen| {
        selected_is(screen, "looknfeel.lua")
    });
    session.press(ENTER);
    session.wait_for_colors("custom green added row", |screen| {
        mode_is(screen, false) && hue_is(screen, false)
    });
    colors(&system, "light", "#00ff00");
    session.press("x");
    let light = session.wait_for_colors("cached diff repainted light", |screen| {
        mode_is(screen, true) && hue_is(screen, false)
    });
    assert!(light.contents().contains("gaps = 8"));
    colors(&system, "dark", "#ff00ff");
    session.press(ESC);
    session.wait_for("tree restored after leaving the diff", |screen| {
        screen.first().contains("Drift")
    });
    session.press(ENTER);
    session.wait_for_colors("new diff loads custom magenta hue", |screen| {
        mode_is(screen, false) && hue_is(screen, true)
    });
    session.press(ESC);
    session.wait_for("tree restored after leaving the diff", |screen| {
        screen.first().contains("Drift")
    });
    session.press("q");
    assert_eq!(session.wait_exit(), 0);
}
