use lunatui_core::{clip_display_prefix, Attrs, ClusterTable, Grid, Rect, Style};
use lunatui_runtime::keymap::KeyChord;
use lunatui_term::{Event, KeyCode, KeyEvent, KeyKind, MouseButton, MouseKind, Scroll};
use lunatui_theme::Token;
use lunatui_widgets::{CommandPalette, PaletteEntry, RenderCx, SearchBox, Widget};

use crate::commands::{chords_text, help_lines, keymap_of, Context, GlobalCommand};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Stay,
    Close,
    Run(Event),
}

pub struct Help {
    lines: Vec<String>,
    scroll: usize,
    rows: u16,
}

impl Help {
    pub fn new(context: Context) -> Self {
        Self {
            lines: help_lines(context),
            scroll: 0,
            rows: 0,
        }
    }

    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    fn limit(&self) -> usize {
        self.lines
            .len()
            .saturating_sub(usize::from(self.rows.max(1)))
    }

    pub fn handle(&mut self, event: &Event) -> Outcome {
        match event {
            Event::Key(key) if key.kind == KeyKind::Press => self.key(key.code),
            Event::Mouse(mouse) => {
                self.wheel(mouse.kind);
                Outcome::Stay
            }
            _ => Outcome::Stay,
        }
    }

    fn key(&mut self, code: KeyCode) -> Outcome {
        let half = usize::from(self.rows.max(2) / 2);
        match code {
            KeyCode::Esc | KeyCode::Char('q' | '?') => return Outcome::Close,
            KeyCode::Char('j') | KeyCode::Down => self.scroll_by(1),
            KeyCode::Char('k') | KeyCode::Up => self.scroll_by(-1),
            KeyCode::PageDown => self.scroll_by(half as isize),
            KeyCode::PageUp => self.scroll_by(-(half as isize)),
            KeyCode::Char('g') | KeyCode::Home => self.scroll = 0,
            KeyCode::Char('G') | KeyCode::End => self.scroll = self.limit(),
            _ => {}
        }
        Outcome::Stay
    }

    fn wheel(&mut self, kind: MouseKind) {
        match kind {
            MouseKind::Scroll(Scroll::Down) => self.scroll_by(3),
            MouseKind::Scroll(Scroll::Up) => self.scroll_by(-3),
            _ => {}
        }
    }

    fn scroll_by(&mut self, delta: isize) {
        self.scroll = self.scroll.saturating_add_signed(delta).min(self.limit());
    }

    pub fn render(&mut self, grid: &mut Grid, area: Rect, clusters: &mut ClusterTable) {
        let theme = crate::diff_palette::active_theme();
        let mut cx = RenderCx::new(grid, clusters, &theme);
        let base = Style::new()
            .fg(cx.token(Token::TextPrimary))
            .bg(cx.token(Token::Surface0));
        cx.fill(area, base, ' ');
        if area.width < 20 || area.height < 4 {
            put(
                &mut cx,
                Rect::new(area.x, area.y, area.width, 1),
                "Too small",
                base,
            );
            return;
        }
        let accent = base
            .fg(cx.token(Token::Accent))
            .with_attrs(Attrs::new().set_bold(true));
        let dim = base.with_attrs(Attrs::new().set_dim(true));
        let bold = base.with_attrs(Attrs::new().set_bold(true));
        put(
            &mut cx,
            Rect::new(area.x, area.y, area.width, 1),
            " Drift  Help",
            accent,
        );
        self.rows = area.height - 2;
        self.scroll = self.scroll.min(self.limit());
        for (offset, line) in self
            .lines
            .iter()
            .skip(self.scroll)
            .take(usize::from(self.rows))
            .enumerate()
        {
            let style = if line.starts_with(' ') || line.is_empty() {
                base
            } else {
                bold
            };
            put(
                &mut cx,
                Rect::new(area.x, area.y + 1 + offset as u16, area.width, 1),
                &format!(" {line}"),
                style,
            );
        }
        put(
            &mut cx,
            Rect::new(area.x, area.y + area.height - 1, area.width, 1),
            " [Esc] Close  [j/k] Scroll  generated from the keymaps",
            dim,
        );
    }
}

pub struct Entry {
    pub title: String,
    pub shortcut: String,
    pub chord: KeyChord,
}

pub struct Palette {
    entries: Vec<Entry>,
    query: String,
    selected: Option<usize>,
    area: Option<Rect>,
}

pub fn palette_entries(context: Context) -> Vec<Entry> {
    let mut entries = Vec::new();
    let mut add = |keymap: lunatui_runtime::keymap::Keymap, skip: Option<&str>| {
        for command in keymap.commands() {
            if Some(command.id()) == skip {
                continue;
            }
            let Some(chord) = command.chords().first().copied() else {
                continue;
            };
            entries.push(Entry {
                title: command.title().to_string(),
                shortcut: chords_text(command.chords()),
                chord,
            });
        }
    };
    add(keymap_of(context), None);
    add(
        crate::commands::global_keymap(),
        Some(GlobalCommand::Palette.id()),
    );
    entries
}

impl Palette {
    pub fn new(context: Context) -> Self {
        let mut palette = Self {
            entries: palette_entries(context),
            query: String::new(),
            selected: None,
            area: None,
        };
        palette.selected = palette.matched().first().copied();
        palette
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn titles(&self) -> Vec<&str> {
        self.matched()
            .into_iter()
            .map(|index| self.entries[index].title.as_str())
            .collect()
    }

    pub fn selected_title(&self) -> Option<&str> {
        self.selected
            .map(|index| self.entries[index].title.as_str())
    }

    fn with_widget<T>(&self, show: bool, f: impl FnOnce(&CommandPalette<'_>) -> T) -> T {
        let items: Vec<PaletteEntry<'_>> = self
            .entries
            .iter()
            .map(|entry| PaletteEntry::new(&entry.title).shortcut(&entry.shortcut))
            .collect();
        let palette = CommandPalette::new(self.query.as_str(), self.query.len(), &items)
            .selected(if show { self.position() } else { None })
            .title("Command palette")
            .placeholder("Type a command");
        f(&palette)
    }

    fn matched(&self) -> Vec<usize> {
        self.with_widget(false, |palette| {
            palette
                .matches()
                .into_iter()
                .map(|found| found.index)
                .collect()
        })
    }

    fn position(&self) -> Option<usize> {
        let selected = self.selected?;
        self.matched().iter().position(|index| *index == selected)
    }

    fn step(&mut self, forward: bool) {
        let found = self.matched();
        let next = SearchBox::step(self.position(), found.len(), forward);
        self.selected = next.and_then(|at| found.get(at).copied());
    }

    fn requery(&mut self) {
        let found = self.matched();
        if !self.selected.is_some_and(|index| found.contains(&index)) {
            self.selected = found.first().copied();
        }
    }

    fn run(&self, index: usize) -> Outcome {
        let chord = self.entries[index].chord;
        Outcome::Run(Event::Key(
            KeyEvent::press(chord.code()).modifiers(chord.modifiers()),
        ))
    }

    pub fn handle(&mut self, event: &Event) -> Outcome {
        match event {
            Event::Key(key) if key.kind == KeyKind::Press => match key.code {
                KeyCode::Esc => return Outcome::Close,
                KeyCode::Enter => {
                    return match self.selected {
                        Some(index) => self.run(index),
                        None => Outcome::Stay,
                    };
                }
                KeyCode::Down => self.step(true),
                KeyCode::Up => self.step(false),
                KeyCode::Backspace => {
                    self.query.pop();
                    self.requery();
                }
                KeyCode::Char(c) if !key.modifiers.is_control() && !key.modifiers.is_alt() => {
                    self.query.push(c);
                    self.requery();
                }
                _ => {}
            },
            Event::Mouse(mouse) => {
                if let (MouseKind::Down(MouseButton::Left), Some(area)) = (mouse.kind, self.area) {
                    let hit = self.with_widget(true, |palette| {
                        palette.entry_at(area, mouse.column, mouse.row)
                    });
                    if let Some(at) = hit {
                        let found = self.matched();
                        if let Some(index) = found.get(at).copied() {
                            return self.run(index);
                        }
                    }
                }
            }
            _ => {}
        }
        Outcome::Stay
    }

    pub fn render(&mut self, grid: &mut Grid, area: Rect, clusters: &mut ClusterTable) {
        self.area = Some(area);
        let theme = crate::diff_palette::active_theme();
        let mut cx = RenderCx::new(grid, clusters, &theme);
        self.with_widget(true, |palette| palette.render(area, &mut cx));
    }
}

fn put(cx: &mut RenderCx<'_>, rect: Rect, text: &str, style: Style) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    cx.put_str(rect.x, rect.y, clip_display_prefix(text, rect.width), style);
}

#[cfg(test)]
mod tests {
    use lunatui::{render_once, App, ControlFlow};
    use lunatui_term::{KeyEvent, MouseEvent};

    use super::*;
    use crate::commands::{help_lines, TREE_COMMANDS};

    struct Palettes<'a>(&'a mut Palette);

    impl App for Palettes<'_> {
        fn update(&mut self, _event: &Event) -> ControlFlow {
            ControlFlow::Continue
        }

        fn render(&mut self, grid: &mut Grid, area: Rect, clusters: &mut ClusterTable) {
            self.0.render(grid, area, clusters);
        }
    }

    struct Helps<'a>(&'a mut Help);

    impl App for Helps<'_> {
        fn update(&mut self, _event: &Event) -> ControlFlow {
            ControlFlow::Continue
        }

        fn render(&mut self, grid: &mut Grid, area: Rect, clusters: &mut ClusterTable) {
            self.0.render(grid, area, clusters);
        }
    }

    fn key(code: KeyCode) -> Event {
        Event::Key(KeyEvent::press(code))
    }

    fn type_text(palette: &mut Palette, text: &str) {
        for c in text.chars() {
            let _ = palette.handle(&key(KeyCode::Char(c)));
        }
    }

    fn frame(palette: &mut Palette, columns: u16, rows: u16) -> String {
        let (grid, clusters) = render_once(&mut Palettes(palette), columns, rows);
        grid.display_text(&clusters)
    }

    #[test]
    fn the_palette_lists_the_actions_of_its_screen_and_the_help_but_not_itself() {
        let palette = Palette::new(Context::Tree);
        let titles = palette.titles();
        for command in TREE_COMMANDS {
            assert!(titles.contains(&command.title()), "{}", command.id());
        }
        assert!(titles.contains(&"Show the help"));
        assert!(!titles.contains(&"Open the command palette"));
        assert_eq!(palette.selected_title(), Some("Quit"));
        let history = Palette::new(Context::History);
        assert!(history.titles().contains(&"Undo the selected transaction"));
        assert!(!history.titles().contains(&"Quit"));
    }

    #[test]
    fn typing_filters_backspace_widens_and_the_selection_follows_the_matches() {
        let mut palette = Palette::new(Context::Tree);
        type_text(&mut palette, "keep");
        assert_eq!(palette.query(), "keep");
        assert_eq!(palette.titles(), ["Keep mine"]);
        assert_eq!(palette.selected_title(), Some("Keep mine"));
        let _ = palette.handle(&key(KeyCode::Backspace));
        assert_eq!(palette.query(), "kee");
        assert!(palette.selected_title().is_some());
        type_text(&mut palette, "zzz");
        assert!(palette.titles().is_empty());
        assert_eq!(palette.selected_title(), None);
        assert_eq!(palette.handle(&key(KeyCode::Enter)), Outcome::Stay);
    }

    #[test]
    fn arrows_move_through_the_matches_and_wrap() {
        let mut palette = Palette::new(Context::Tree);
        type_text(&mut palette, "filter");
        let titles: Vec<String> = palette.titles().iter().map(|t| t.to_string()).collect();
        assert_eq!(titles.len(), 8);
        assert!(
            titles
                .iter()
                .filter(|title| title.starts_with("Filter: "))
                .count()
                == 6
        );
        assert_eq!(palette.selected_title(), Some(titles[0].as_str()));
        let _ = palette.handle(&key(KeyCode::Down));
        assert_eq!(palette.selected_title(), Some(titles[1].as_str()));
        let _ = palette.handle(&key(KeyCode::Up));
        let _ = palette.handle(&key(KeyCode::Up));
        assert_eq!(palette.selected_title(), Some(titles[7].as_str()));
    }

    #[test]
    fn enter_runs_the_selected_action_as_its_first_key_and_escape_closes() {
        let mut palette = Palette::new(Context::Tree);
        type_text(&mut palette, "take");
        assert_eq!(
            palette.handle(&key(KeyCode::Enter)),
            Outcome::Run(key(KeyCode::Char('t')))
        );
        assert_eq!(palette.handle(&key(KeyCode::Esc)), Outcome::Close);
        let mut diff = Palette::new(Context::Diff);
        type_text(&mut diff, "previous pane");
        let Outcome::Run(Event::Key(pressed)) = diff.handle(&key(KeyCode::Enter)) else {
            panic!("run");
        };
        assert_eq!(pressed.code, KeyCode::Tab);
        assert!(pressed.modifiers.is_shift());
    }

    #[test]
    fn the_palette_draws_its_matches_with_their_keys_and_a_click_runs_one() {
        let mut palette = Palette::new(Context::Tree);
        type_text(&mut palette, "mer");
        let text = frame(&mut palette, 100, 30);
        assert!(text.contains("Command palette"), "{text}");
        assert!(text.contains("Merge"), "{text}");
        assert!(text.contains(" m"), "{text}");
        let row = text
            .lines()
            .position(|line| line.contains("Merge"))
            .unwrap() as u16;
        let column = text
            .lines()
            .nth(row as usize)
            .unwrap()
            .find("Merge")
            .unwrap() as u16;
        let click = Event::Mouse(MouseEvent::new(
            MouseKind::Down(MouseButton::Left),
            column,
            row,
        ));
        assert_eq!(
            palette.handle(&click),
            Outcome::Run(key(KeyCode::Char('m')))
        );
        let miss = Event::Mouse(MouseEvent::new(MouseKind::Down(MouseButton::Left), 0, 0));
        assert_eq!(palette.handle(&miss), Outcome::Stay);
    }

    #[test]
    fn the_help_shows_the_generated_lines_scrolls_and_closes() {
        let mut help = Help::new(Context::Tree);
        assert_eq!(help.lines(), help_lines(Context::Tree).as_slice());
        let (grid, clusters) = render_once(&mut Helps(&mut help), 80, 12);
        let first = grid.display_text(&clusters);
        assert!(
            first.contains("Drift  Help") && first.contains("Everywhere"),
            "{first}"
        );
        assert!(!first.contains("Open the diff"), "{first}");
        for _ in 0..8 {
            let _ = help.handle(&key(KeyCode::Char('j')));
        }
        let (grid, clusters) = render_once(&mut Helps(&mut help), 80, 12);
        let later = grid.display_text(&clusters);
        assert!(!later.contains("Everywhere"), "{later}");
        let _ = help.handle(&key(KeyCode::End));
        let _ = help.handle(&key(KeyCode::Home));
        let (grid, clusters) = render_once(&mut Helps(&mut help), 80, 12);
        assert!(grid.display_text(&clusters).contains("Everywhere"));
        let wheel = Event::Mouse(MouseEvent::new(MouseKind::Scroll(Scroll::Down), 5, 5));
        let _ = help.handle(&wheel);
        for closer in [KeyCode::Esc, KeyCode::Char('q'), KeyCode::Char('?')] {
            assert_eq!(help.handle(&key(closer)), Outcome::Close);
        }
        assert_eq!(help.handle(&key(KeyCode::Char('x'))), Outcome::Stay);
    }

    #[test]
    fn page_keys_and_arrows_scroll_by_the_expected_amounts() {
        let mut help = Help::new(Context::Tree);
        let _ = render_once(&mut Helps(&mut help), 80, 12);
        let _ = help.handle(&key(KeyCode::PageDown));
        assert_eq!(help.scroll, 5);
        let _ = help.handle(&key(KeyCode::Down));
        assert_eq!(help.scroll, 6);
        let _ = help.handle(&key(KeyCode::Up));
        let _ = help.handle(&key(KeyCode::Char('k')));
        assert_eq!(help.scroll, 4);
        let _ = help.handle(&key(KeyCode::PageUp));
        assert_eq!(help.scroll, 0);
        let _ = help.handle(&key(KeyCode::Char('G')));
        assert_eq!(help.scroll, help.limit());
        let _ = help.handle(&key(KeyCode::PageDown));
        assert_eq!(help.scroll, help.limit());
        let _ = help.handle(&key(KeyCode::Char('g')));
        assert_eq!(help.scroll, 0);
        let up = Event::Mouse(MouseEvent::new(MouseKind::Scroll(Scroll::Up), 5, 5));
        let _ = help.handle(&up);
        assert_eq!(help.scroll, 0);
        let release = Event::Key(KeyEvent::press(KeyCode::Char('j')).kind(KeyKind::Release));
        let _ = help.handle(&release);
        assert_eq!(help.scroll, 0);
    }

    #[test]
    fn a_tiny_terminal_says_so() {
        let mut help = Help::new(Context::Tree);
        let (grid, clusters) = render_once(&mut Helps(&mut help), 15, 3);
        assert!(grid.display_text(&clusters).contains("Too small"));
    }
}
