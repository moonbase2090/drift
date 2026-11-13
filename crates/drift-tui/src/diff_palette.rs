use std::ffi::OsString;
use std::path::PathBuf;

use lunatui_core::Color;
use lunatui_theme::{Theme, Token, Variant};

const FALLBACK_BACKGROUND: [u8; 3] = [24, 24, 32];
const ROW_PERCENT: u16 = 20;
const WORD_PERCENT: u16 = 36;
const FILLER_PERCENT: u16 = 7;
const MARK_CONTRAST: u32 = 450;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiffPalette {
    green: [u8; 3],
    red: [u8; 3],
    yellow: [u8; 3],
    muted: [u8; 3],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKind {
    Plain,
    Added,
    Removed,
    Changed,
    Filler,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tints {
    palette: DiffPalette,
    background: [u8; 3],
}

impl DiffPalette {
    pub const fn fallback() -> Self {
        Self {
            green: [126, 200, 120],
            red: [232, 106, 118],
            yellow: [224, 175, 104],
            muted: [120, 126, 150],
        }
    }

    pub fn from_colors(source: &str) -> Self {
        let mut palette = Self::fallback();
        for line in source.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let Some(color) = parse_hex(value) else {
                continue;
            };
            match key.trim() {
                "green" => palette.green = color,
                "red" => palette.red = color,
                "yellow" => palette.yellow = color,
                "muted" => palette.muted = color,
                _ => {}
            }
        }
        palette
    }

    pub fn active() -> Self {
        colors_path()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .map(|source| Self::from_colors(&source))
            .unwrap_or_else(Self::fallback)
    }

    pub fn tints(self, background: Color) -> Tints {
        let background = match background {
            Color::Rgb(red, green, blue) => [red, green, blue],
            _ => FALLBACK_BACKGROUND,
        };
        Tints {
            palette: self,
            background,
        }
    }
}

pub(crate) fn variant_from_colors(source: &str) -> Variant {
    let light = source.lines().any(|line| {
        let line = line.split('#').next().unwrap_or(line);
        let Some((key, value)) = line.split_once('=') else {
            return false;
        };
        key.trim() == "mode" && value.trim().trim_matches(['"', '\'']) == "light"
    });
    if light {
        Variant::Light
    } else {
        Variant::Dark
    }
}

pub(crate) fn active_variant() -> Variant {
    colors_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|source| variant_from_colors(&source))
        .unwrap_or(Variant::Dark)
}

const LIGHT_TEXT: [(Token, Color); 5] = [
    (Token::Accent, Color::Rgb(0x1a, 0x4f, 0x8b)),
    (Token::Ok, Color::Rgb(0x1b, 0x6b, 0x2f)),
    (Token::Warn, Color::Rgb(0x8a, 0x4b, 0x08)),
    (Token::Fail, Color::Rgb(0x9b, 0x1c, 0x2c)),
    (Token::FocusRing, Color::Rgb(0x3b, 0x2f, 0x8a)),
];

pub(crate) fn theme_for(variant: Variant) -> Theme {
    match variant {
        Variant::Light => Theme::with_overrides(variant, &LIGHT_TEXT),
        Variant::Dark => Theme::builtin(variant),
    }
}

pub(crate) fn active_theme() -> Theme {
    theme_for(active_variant())
}

impl Tints {
    pub fn row(&self, kind: RowKind) -> Option<Color> {
        match kind {
            RowKind::Plain => None,
            RowKind::Filler => Some(self.mixed(self.palette.muted, FILLER_PERCENT)),
            _ => Some(self.mixed(self.hue(kind), ROW_PERCENT)),
        }
    }

    pub fn word(&self, kind: RowKind) -> Option<Color> {
        match kind {
            RowKind::Plain | RowKind::Filler => None,
            _ => Some(self.mixed(self.hue(kind), WORD_PERCENT)),
        }
    }

    pub fn mark(&self, kind: RowKind) -> Option<(char, Color)> {
        let glyph = match kind {
            RowKind::Added => '+',
            RowKind::Removed => '-',
            RowKind::Changed => '~',
            RowKind::Plain | RowKind::Filler => return None,
        };
        let ground = self.blend(self.hue(kind), ROW_PERCENT);
        let [red, green, blue] = legible(self.hue(kind), ground, MARK_CONTRAST);
        Some((glyph, Color::Rgb(red, green, blue)))
    }

    fn hue(&self, kind: RowKind) -> [u8; 3] {
        match kind {
            RowKind::Added => self.palette.green,
            RowKind::Removed => self.palette.red,
            RowKind::Changed => self.palette.yellow,
            RowKind::Plain | RowKind::Filler => self.palette.muted,
        }
    }

    fn blend(&self, hue: [u8; 3], percent: u16) -> [u8; 3] {
        mix(self.background, hue, percent)
    }

    fn mixed(&self, hue: [u8; 3], percent: u16) -> Color {
        let [red, green, blue] = self.blend(hue, percent);
        Color::Rgb(red, green, blue)
    }
}

fn mix(from: [u8; 3], to: [u8; 3], percent: u16) -> [u8; 3] {
    let channel = |from: u8, to: u8| {
        let blended = u16::from(from) * (100 - percent) + u16::from(to) * percent;
        u8::try_from(blended / 100).unwrap_or(u8::MAX)
    };
    [
        channel(from[0], to[0]),
        channel(from[1], to[1]),
        channel(from[2], to[2]),
    ]
}

fn luminance(color: [u8; 3]) -> f64 {
    let linear = |value: u8| {
        let scaled = f64::from(value) / 255.0;
        if scaled <= 0.03928 {
            scaled / 12.92
        } else {
            ((scaled + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(color[0]) + 0.7152 * linear(color[1]) + 0.0722 * linear(color[2])
}

fn contrast_hundredths(first: [u8; 3], second: [u8; 3]) -> u32 {
    let (one, two) = (luminance(first), luminance(second));
    let (light, dark) = if one >= two { (one, two) } else { (two, one) };
    let ratio = (light + 0.05) / (dark + 0.05) * 100.0;
    if ratio >= f64::from(u32::MAX) {
        u32::MAX
    } else {
        ratio as u32
    }
}

fn legible(hue: [u8; 3], ground: [u8; 3], wanted: u32) -> [u8; 3] {
    let toward = if luminance(ground) < 0.5 { 255 } else { 0 };
    (0..=20_u16)
        .map(|step| mix(hue, [toward; 3], step * 5))
        .find(|candidate| contrast_hundredths(*candidate, ground) >= wanted)
        .unwrap_or([toward; 3])
}

fn parse_hex(value: &str) -> Option<[u8; 3]> {
    let value = value.trim().trim_matches(['"', '\'']);
    let digits = value.strip_prefix('#')?;
    if digits.len() != 6 || !digits.is_ascii() {
        return None;
    }
    let pair = |at: usize| u8::from_str_radix(&digits[at..at + 2], 16).ok();
    Some([pair(0)?, pair(2)?, pair(4)?])
}

fn colors_path() -> Option<PathBuf> {
    colors_path_in(std::env::var_os("XDG_STATE_HOME"), std::env::var_os("HOME"))
}

fn colors_path_in(state: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    let base = match state {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => PathBuf::from(home?).join(".local/state"),
    };
    Some(base.join("omarchy/current/theme/colors.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKYO: &str =
        "mode = \"dark\"\nred = \"#f7768e\"\ngreen = \"#9ece6a\"\nyellow = \"#e0af68\"\n";

    fn rgb(color: Option<Color>) -> [u8; 3] {
        match color {
            Some(Color::Rgb(red, green, blue)) => [red, green, blue],
            other => panic!("expected an rgb color, got {other:?}"),
        }
    }

    #[test]
    fn theme_colors_replace_the_fallback_hues() {
        let themed = DiffPalette::from_colors(TOKYO);
        assert_eq!(themed.green, [0x9e, 0xce, 0x6a]);
        assert_eq!(themed.red, [0xf7, 0x76, 0x8e]);
        assert_eq!(themed.yellow, [0xe0, 0xaf, 0x68]);
        assert_eq!(themed.muted, DiffPalette::fallback().muted);
    }

    #[test]
    fn unreadable_or_partial_themes_keep_readable_fallbacks() {
        assert_eq!(DiffPalette::from_colors(""), DiffPalette::fallback());
        assert_eq!(
            DiffPalette::from_colors("green = \"nope\"\nred = #12\n"),
            DiffPalette::fallback()
        );
        let partial = DiffPalette::from_colors("red = \"#ff0000\"");
        assert_eq!(partial.red, [255, 0, 0]);
        assert_eq!(partial.green, DiffPalette::fallback().green);
    }

    #[test]
    fn row_tints_sit_between_the_background_and_the_hue() {
        let tints = DiffPalette::from_colors(TOKYO).tints(Color::Rgb(26, 27, 38));
        let added = rgb(tints.row(RowKind::Added));
        assert!(added[1] > added[0] && added[1] > added[2]);
        assert!(added[1] > 27 && added[1] < 0xce);
        let removed = rgb(tints.row(RowKind::Removed));
        assert!(removed[0] > removed[1] && removed[0] > removed[2]);
    }

    #[test]
    fn the_changed_characters_are_stronger_than_the_row() {
        let tints = DiffPalette::fallback().tints(Color::Rgb(24, 24, 32));
        for kind in [RowKind::Added, RowKind::Removed, RowKind::Changed] {
            let row = rgb(tints.row(kind));
            let word = rgb(tints.word(kind));
            let background = [24_u8, 24, 32];
            let gap = |color: [u8; 3]| {
                color
                    .iter()
                    .zip(background)
                    .map(|(a, b)| u32::from(a.abs_diff(b)))
                    .sum::<u32>()
            };
            assert!(gap(word) > gap(row), "{kind:?}");
        }
    }

    #[test]
    fn a_marker_that_would_vanish_on_its_row_is_lifted_and_a_good_one_is_kept() {
        let dark = Color::Rgb(30, 30, 46);
        let rose = DiffPalette::from_colors("green = \"#286983\"").tints(dark);
        let (_, lifted) = rose.mark(RowKind::Added).expect("mark");
        let lifted = rgb(Some(lifted));
        assert_ne!(lifted, [0x28, 0x69, 0x83]);
        let row = rgb(rose.row(RowKind::Added));
        assert!(contrast_hundredths(lifted, row) >= MARK_CONTRAST);
        let fallback = DiffPalette::fallback().tints(dark);
        let (_, kept) = fallback.mark(RowKind::Added).expect("mark");
        assert_eq!(rgb(Some(kept)), DiffPalette::fallback().green);
    }

    #[test]
    fn plain_rows_are_left_alone_and_markers_follow_the_kind() {
        let tints = DiffPalette::fallback().tints(Color::Default);
        assert_eq!(tints.row(RowKind::Plain), None);
        assert_eq!(tints.word(RowKind::Plain), None);
        assert_eq!(tints.mark(RowKind::Plain), None);
        assert_eq!(tints.mark(RowKind::Filler), None);
        assert!(tints.row(RowKind::Filler).is_some());
        let glyphs: Vec<char> = [RowKind::Added, RowKind::Removed, RowKind::Changed]
            .into_iter()
            .filter_map(|kind| tints.mark(kind).map(|(glyph, _)| glyph))
            .collect();
        assert_eq!(glyphs, ['+', '-', '~']);
    }

    #[test]
    fn the_active_theme_file_lives_in_the_omarchy_state_directory() {
        let wanted = "omarchy/current/theme/colors.toml";
        assert_eq!(
            colors_path_in(Some("/state".into()), Some("/home/me".into())),
            Some(PathBuf::from("/state").join(wanted))
        );
        assert_eq!(
            colors_path_in(None, Some("/home/me".into())),
            Some(PathBuf::from("/home/me/.local/state").join(wanted))
        );
        assert_eq!(
            colors_path_in(Some("".into()), Some("/home/me".into())),
            Some(PathBuf::from("/home/me/.local/state").join(wanted))
        );
        assert_eq!(colors_path_in(None, None), None);
    }

    #[test]
    fn a_light_omarchy_theme_selects_the_light_variant_and_everything_else_stays_dark() {
        assert_eq!(
            variant_from_colors("mode = \"light\"\nbackground = \"#ffffff\"\n"),
            Variant::Light
        );
        assert_eq!(variant_from_colors("  mode = 'light'"), Variant::Light);
        assert_eq!(variant_from_colors(TOKYO), Variant::Dark);
        assert_eq!(variant_from_colors("mode = \"dark\"\n"), Variant::Dark);
        assert_eq!(variant_from_colors("mode = \"LIGHT\"\n"), Variant::Dark);
        assert_eq!(variant_from_colors("# mode = \"light\"\n"), Variant::Dark);
        assert_eq!(variant_from_colors(""), Variant::Dark);
        let light = Theme::builtin(Variant::Light);
        let dark = Theme::builtin(Variant::Dark);
        assert_eq!(light.variant(), Variant::Light);
        assert_ne!(
            light.resolve(lunatui_theme::Token::Surface0),
            dark.resolve(lunatui_theme::Token::Surface0)
        );
        assert_eq!(active_theme().variant(), active_variant());
    }
}
