use lunatui_core::Color;
use lunatui_theme::{Theme, Token, Variant};

use crate::diff_palette::{DiffPalette, RowKind};

fn rgb(color: Color) -> [u8; 3] {
    match color {
        Color::Rgb(red, green, blue) => [red, green, blue],
        other => panic!("expected an rgb color, got {other:?}"),
    }
}

fn linear(value: u8) -> f64 {
    let scaled = f64::from(value) / 255.0;
    if scaled <= 0.03928 {
        scaled / 12.92
    } else {
        ((scaled + 0.055) / 1.055).powf(2.4)
    }
}

fn luminance(color: [u8; 3]) -> f64 {
    0.2126 * linear(color[0]) + 0.7152 * linear(color[1]) + 0.0722 * linear(color[2])
}

fn ratio(first: Color, second: Color) -> f64 {
    let (one, two) = (luminance(rgb(first)), luminance(rgb(second)));
    let (light, dark) = if one >= two { (one, two) } else { (two, one) };
    (light + 0.05) / (dark + 0.05)
}

#[test]
fn the_wcag_ratio_matches_known_values() {
    let black = Color::Rgb(0, 0, 0);
    let white = Color::Rgb(255, 255, 255);
    assert!((ratio(black, white) - 21.0).abs() < 0.01);
    assert!((ratio(white, white) - 1.0).abs() < 0.001);
    assert!((ratio(Color::Rgb(119, 119, 119), white) - 4.48).abs() < 0.02);
}

const LIGHT_SHORTFALLS: [Token; 5] = [
    Token::Accent,
    Token::Ok,
    Token::Warn,
    Token::Fail,
    Token::FocusRing,
];

fn shortfalls(variant: Variant) -> Vec<Token> {
    let theme = Theme::builtin(variant);
    let ground = theme.resolve(Token::Surface0);
    [
        (Token::TextPrimary, 4.5),
        (Token::Accent, 4.5),
        (Token::Ok, 4.5),
        (Token::Warn, 4.5),
        (Token::Fail, 4.5),
        (Token::FocusRing, 3.0),
    ]
    .into_iter()
    .filter(|(token, wanted)| ratio(theme.resolve(*token), ground) < *wanted)
    .map(|(token, _)| token)
    .collect()
}

#[test]
fn the_dark_theme_drift_runs_in_keeps_text_status_and_focus_legible() {
    assert_eq!(shortfalls(Variant::Dark), Vec::<Token>::new());
}

#[test]
fn the_light_fallback_keeps_text_legible_and_its_known_status_shortfalls_are_pinned() {
    let found = shortfalls(Variant::Light);
    assert!(!found.contains(&Token::TextPrimary));
    assert_eq!(found, LIGHT_SHORTFALLS.to_vec());
}

const OMARCHY: [(&str, &str); 4] = [
    (
        "tokyo-night",
        "red = \"#f7768e\"\ngreen = \"#9ece6a\"\nyellow = \"#e0af68\"\n",
    ),
    (
        "rose-pine",
        "red = \"#b4637a\"\ngreen = \"#286983\"\nyellow = \"#ea9d34\"\n",
    ),
    (
        "flexoki-light",
        "red = \"#D14D41\"\ngreen = \"#879A39\"\nyellow = \"#D0A215\"\n",
    ),
    (
        "white",
        "red = \"#2a2a2a\"\ngreen = \"#3a3a3a\"\nyellow = \"#4a4a4a\"\n",
    ),
];

fn diff_shortfalls(variant: Variant, palette: DiffPalette) -> Vec<String> {
    let theme = Theme::builtin(variant);
    let ground = theme.resolve(Token::Surface0);
    let text = theme.resolve(Token::TextPrimary);
    let tints = palette.tints(ground);
    let mut found = Vec::new();
    for kind in [RowKind::Added, RowKind::Removed, RowKind::Changed] {
        let row = tints.row(kind).expect("row tint");
        let word = tints.word(kind).expect("word tint");
        let (_, mark) = tints.mark(kind).expect("mark");
        for (what, ratio) in [
            ("text on row", ratio(text, row)),
            ("text on changed characters", ratio(text, word)),
            ("marker on row", ratio(mark, row)),
        ] {
            if ratio < 4.5 {
                found.push(format!("{kind:?} {what} {ratio:.2}"));
            }
        }
    }
    found
}

#[test]
fn omarchy_hues_and_the_fallback_keep_diff_text_and_markers_legible_on_both_themes() {
    let mut failures = Vec::new();
    for (name, variant) in [("dark", Variant::Dark), ("light", Variant::Light)] {
        let mut palettes = vec![("fallback", DiffPalette::fallback())];
        palettes.extend(
            OMARCHY
                .iter()
                .map(|(theme, source)| (*theme, DiffPalette::from_colors(source))),
        );
        assert_eq!(palettes.len(), 5);
        for (palette, found) in palettes {
            for shortfall in diff_shortfalls(variant, found) {
                failures.push(format!("{name} + {palette}: {shortfall}"));
            }
        }
    }
    assert_eq!(failures, Vec::<String>::new());
}
