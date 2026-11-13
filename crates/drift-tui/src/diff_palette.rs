use std::ffi::OsString;
use std::path::PathBuf;

use lunatui_core::Color;

const FALLBACK_BACKGROUND: [u8; 3] = [24, 24, 32];
const ROW_PERCENT: u16 = 20;
const WORD_PERCENT: u16 = 46;
const FILLER_PERCENT: u16 = 7;

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
        let [red, green, blue] = self.hue(kind);
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

    fn mixed(&self, hue: [u8; 3], percent: u16) -> Color {
        let channel = |from: u8, to: u8| {
            let blended = u16::from(from) * (100 - percent) + u16::from(to) * percent;
            u8::try_from(blended / 100).unwrap_or(u8::MAX)
        };
        Color::Rgb(
            channel(self.background[0], hue[0]),
            channel(self.background[1], hue[1]),
            channel(self.background[2], hue[2]),
        )
    }
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
}
