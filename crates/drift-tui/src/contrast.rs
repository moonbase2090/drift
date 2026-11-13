use lunatui_core::Color;
use lunatui_theme::{Theme, Token, Variant};

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
