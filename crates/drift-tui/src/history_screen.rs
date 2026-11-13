use lunatui_core::{clip_display_prefix, width, Attrs, ClusterTable, Grid, Rect, Style};
use lunatui_term::{Event, KeyCode, KeyKind, MouseButton, MouseKind, Scroll};
use lunatui_theme::{Theme, Token, Variant};
use lunatui_widgets::RenderCx;

use lunatui_runtime::keymap::Keymap;

use drift_core::HistoryView;

use crate::commands::{
    chords_text, global_keymap, history_keymap, press, GlobalCommand, HistoryCommand,
};
use crate::tree_screen::wrap;

const WHEEL_STEP: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryAction {
    Back,
    Refresh,
    Undo { target: String, force: bool },
    Help,
    Palette,
}

pub(crate) const HINTS: &[(&str, KeyCode)] = &[
    ("[Q] Back", KeyCode::Char('q')),
    ("[?] Help", KeyCode::Char('?')),
    ("[:] Palette", KeyCode::Char(':')),
    ("[Enter] Undo", KeyCode::Enter),
    ("[U] Undo and force", KeyCode::Char('U')),
    ("[r] Refresh", KeyCode::Char('r')),
];

const BARE_HINTS: &[(&str, KeyCode)] = &[
    ("[Q] Back", KeyCode::Char('q')),
    ("[?] Help", KeyCode::Char('?')),
    ("[:] Palette", KeyCode::Char(':')),
    ("[r] Refresh", KeyCode::Char('r')),
];

struct Layout {
    title: Rect,
    info: Rect,
    header: Rect,
    body: Rect,
    hints: Vec<(Rect, KeyCode, String)>,
}

pub struct HistoryScreen {
    view: HistoryView,
    lines: Vec<String>,
    selected: usize,
    scroll: usize,
    follow: bool,
    keymap: Keymap,
    globals: Keymap,
    notice: Option<String>,
    error: Option<String>,
    area: Option<Rect>,
}

impl HistoryScreen {
    pub fn new(view: HistoryView) -> Self {
        let lines = view.render_text().lines().map(str::to_string).collect();
        Self {
            view,
            lines,
            selected: 0,
            scroll: 0,
            follow: true,
            keymap: history_keymap(),
            globals: global_keymap(),
            notice: None,
            error: None,
            area: None,
        }
    }

    pub fn failed(message: &str) -> Self {
        let mut screen = Self::new(HistoryView {
            rows: Vec::new(),
            unreadable: Vec::new(),
        });
        screen.error = Some(message.to_string());
        screen
    }

    pub fn replace(&mut self, view: HistoryView) {
        let notice = self.notice.take();
        let kept = self.selected_id().map(str::to_string);
        let selected = self.selected;
        *self = Self::new(view);
        self.notice = notice;
        let last = self.view.rows.len().saturating_sub(1);
        self.selected = kept
            .and_then(|id| self.view.rows.iter().position(|row| row.id == id))
            .unwrap_or_else(|| selected.min(last));
    }

    pub fn set_notice(&mut self, text: &str) {
        self.notice = Some(text.to_string());
    }

    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    pub fn selected_id(&self) -> Option<&str> {
        self.view.rows.get(self.selected).map(|row| row.id.as_str())
    }

    pub fn handle(&mut self, event: &Event) -> Option<HistoryAction> {
        if matches!(event, Event::Key(key) if key.kind == KeyKind::Press) {
            self.notice = None;
        }
        match event {
            Event::Key(key) if key.kind == KeyKind::Press => self.key(key.code),
            Event::Mouse(mouse) => self.mouse(mouse.kind, mouse.column, mouse.row),
            _ => None,
        }
    }

    fn undo(&self, force: bool) -> Option<HistoryAction> {
        self.selected_id().map(|id| HistoryAction::Undo {
            target: id.to_string(),
            force,
        })
    }

    fn key(&mut self, code: KeyCode) -> Option<HistoryAction> {
        self.follow = true;
        let event = press(code);
        if let Some(id) = self.globals.dispatch(&event) {
            match GlobalCommand::from_id(id)? {
                GlobalCommand::Help => return Some(HistoryAction::Help),
                GlobalCommand::Palette => return Some(HistoryAction::Palette),
                GlobalCommand::UndoLast => return self.run(HistoryCommand::Undo),
            }
        }
        let id = self.keymap.dispatch(&event)?;
        self.run(HistoryCommand::from_id(id)?)
    }

    pub fn run(&mut self, command: HistoryCommand) -> Option<HistoryAction> {
        match command {
            HistoryCommand::Back => return Some(HistoryAction::Back),
            HistoryCommand::Refresh => return Some(HistoryAction::Refresh),
            HistoryCommand::Undo => return self.undo(false),
            HistoryCommand::UndoForce => return self.undo(true),
            HistoryCommand::Down => self.move_by(1),
            HistoryCommand::Up => self.move_by(-1),
            HistoryCommand::Top => self.selected = 0,
            HistoryCommand::Bottom => {
                self.selected = self.view.rows.len().saturating_sub(1);
            }
            HistoryCommand::PageDown => self.move_by(self.page()),
            HistoryCommand::PageUp => self.move_by(-self.page()),
        }
        None
    }

    fn page(&self) -> isize {
        self.area
            .and_then(|area| layout(area, self.view.is_empty()))
            .map_or(10, |found| (found.body.height / 2).max(1) as isize)
    }

    fn move_by(&mut self, delta: isize) {
        let last = self.view.rows.len().saturating_sub(1);
        self.selected = self.selected.saturating_add_signed(delta).min(last);
    }

    fn mouse(&mut self, kind: MouseKind, column: u16, row: u16) -> Option<HistoryAction> {
        let found = self
            .area
            .and_then(|area| layout(area, self.view.is_empty()))?;
        match kind {
            MouseKind::Down(MouseButton::Left) => {
                if let Some((_, code, _)) = found
                    .hints
                    .iter()
                    .find(|(rect, _, _)| contains(*rect, column, row))
                {
                    return self.key(*code);
                }
                if contains(found.body, column, row) {
                    self.follow = true;
                    let index = self.scroll + usize::from(row - found.body.y);
                    if index < self.view.rows.len() {
                        self.selected = index;
                    }
                }
                None
            }
            MouseKind::Scroll(Scroll::Down) => {
                self.follow = false;
                self.scroll = self.scroll.saturating_add(WHEEL_STEP);
                None
            }
            MouseKind::Scroll(Scroll::Up) => {
                self.follow = false;
                self.scroll = self.scroll.saturating_sub(WHEEL_STEP);
                None
            }
            _ => None,
        }
    }

    fn settle(&mut self, rows: usize) {
        let limit = self.lines.len().saturating_sub(1).saturating_sub(rows);
        self.scroll = self.scroll.min(limit);
        if !self.follow {
            return;
        }
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if rows > 0 && self.selected >= self.scroll + rows {
            self.scroll = self.selected + 1 - rows;
        }
    }

    pub fn render(&mut self, grid: &mut Grid, area: Rect, clusters: &mut ClusterTable) {
        self.area = Some(area);
        let theme = Theme::builtin(Variant::Dark);
        let mut cx = RenderCx::new(grid, clusters, &theme);
        let base = Style::new()
            .fg(cx.token(Token::TextPrimary))
            .bg(cx.token(Token::Surface0));
        cx.fill(area, base, ' ');
        let Some(found) = layout(area, self.view.is_empty()) else {
            put(
                &mut cx,
                Rect::new(area.x, area.y, area.width, 1),
                "Too small",
                base,
            );
            return;
        };
        let dim = base.with_attrs(Attrs::new().set_dim(true));
        let bold = base.with_attrs(Attrs::new().set_bold(true));
        let accent = base
            .fg(cx.token(Token::Accent))
            .with_attrs(Attrs::new().set_bold(true));
        let selected_style = base
            .fg(cx.token(Token::Surface0))
            .bg(cx.token(Token::TextPrimary));

        let count = self.view.rows.len();
        let title = match &self.error {
            Some(_) => " Drift  History  could not be read".to_string(),
            None => format!(
                " Drift  History  {count} transaction{}",
                if count == 1 { "" } else { "s" }
            ),
        };
        put(&mut cx, found.title, &title, accent);
        let info = match &self.notice {
            Some(notice) => format!(" {notice}"),
            None => " Newest first. Undo restores files and reviewed bases.".to_string(),
        };
        put(&mut cx, found.info, &info, dim);

        if self.view.is_empty() {
            let mut lines: Vec<(String, Style)> = Vec::new();
            let width = usize::from(found.body.width.saturating_sub(2));
            match &self.error {
                Some(message) => {
                    lines.push(("Could not read the history.".to_string(), bold));
                    for part in wrap(message, width) {
                        lines.push((part, base));
                    }
                    lines.push((String::new(), dim));
                    let key = chords_text(HistoryCommand::Refresh.chords());
                    for part in wrap(
                        &format!("Nothing was changed. Press {key} to try again."),
                        width,
                    ) {
                        lines.push((part, dim));
                    }
                }
                None => {
                    lines.push(("No transactions yet.".to_string(), bold));
                    lines.push((String::new(), dim));
                    for part in wrap(
                        "Keep, Take and Merge in the tree record a transaction here, and Undo restores it.",
                        width,
                    ) {
                        lines.push((part, dim));
                    }
                }
            }
            for (offset, (text, style)) in lines
                .iter()
                .take(usize::from(found.body.height))
                .enumerate()
            {
                put(
                    &mut cx,
                    Rect::new(
                        found.body.x + 1,
                        found.body.y + offset as u16,
                        found.body.width.saturating_sub(1),
                        1,
                    ),
                    text,
                    *style,
                );
            }
        } else {
            if let Some(line) = self.lines.first() {
                put(&mut cx, found.header, &format!(" {line}"), bold);
            }
            let rows = usize::from(found.body.height);
            self.settle(rows);
            for (offset, line) in self
                .lines
                .iter()
                .skip(1)
                .skip(self.scroll)
                .take(rows)
                .enumerate()
            {
                let rect = Rect::new(
                    found.body.x,
                    found.body.y + offset as u16,
                    found.body.width,
                    1,
                );
                let index = self.scroll + offset;
                let style = if index == self.selected {
                    cx.fill(rect, selected_style, ' ');
                    selected_style
                } else if index >= count {
                    dim
                } else {
                    base
                };
                let marker = if index == self.selected { '>' } else { ' ' };
                put(&mut cx, rect, &format!("{marker}{line}"), style);
            }
        }
        for (rect, _, label) in &found.hints {
            put(&mut cx, *rect, label, dim);
        }
    }
}

fn layout(area: Rect, bare: bool) -> Option<Layout> {
    if area.width < 20 || area.height < 6 {
        return None;
    }
    let hint_y = area.y + area.height - 1;
    let mut hints = Vec::new();
    let mut x = area.x;
    for (label, code) in if bare { BARE_HINTS } else { HINTS } {
        let cells = u16::try_from(width(label)).unwrap_or(u16::MAX);
        if x + cells > area.x + area.width {
            break;
        }
        hints.push((Rect::new(x, hint_y, cells, 1), *code, (*label).to_string()));
        x += cells + 2;
    }
    Some(Layout {
        title: Rect::new(area.x, area.y, area.width, 1),
        info: Rect::new(area.x, area.y + 1, area.width, 1),
        header: Rect::new(area.x, area.y + 2, area.width, 1),
        body: Rect::new(area.x, area.y + 3, area.width, hint_y - (area.y + 3)),
        hints,
    })
}

fn put(cx: &mut RenderCx<'_>, rect: Rect, text: &str, style: Style) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    cx.put_str(rect.x, rect.y, clip_display_prefix(text, rect.width), style);
}

fn contains(area: Rect, column: u16, row: u16) -> bool {
    column >= area.x
        && row >= area.y
        && column < area.x.saturating_add(area.width)
        && row < area.y.saturating_add(area.height)
}

#[cfg(test)]
mod tests {
    use lunatui::{render_once, App, ControlFlow};
    use lunatui_term::{KeyEvent, MouseEvent};

    use drift_core::HistoryRow;

    use super::*;

    struct Driver<'a>(&'a mut HistoryScreen);

    impl App for Driver<'_> {
        fn update(&mut self, _event: &Event) -> ControlFlow {
            ControlFlow::Continue
        }

        fn render(&mut self, grid: &mut Grid, area: Rect, clusters: &mut ClusterTable) {
            self.0.render(grid, area, clusters);
        }
    }

    fn text(screen: &mut HistoryScreen, columns: u16, rows: u16) -> String {
        let (grid, clusters) = render_once(&mut Driver(screen), columns, rows);
        grid.display_text(&clusters)
    }

    fn row(id: &str, time: &str, files: usize, actions: &str) -> HistoryRow {
        HistoryRow {
            id: id.to_string(),
            time: time.to_string(),
            package_version: "4.5.0-3".to_string(),
            files,
            actions: actions.to_string(),
        }
    }

    fn sample() -> HistoryView {
        HistoryView {
            rows: vec![
                row("undo-1800000100456", "2026-11-12T10:31:40Z", 1, "1 undo"),
                row(
                    "resolve-1800000000123",
                    "2026-11-12T10:30:00Z",
                    3,
                    "2 take upstream, 1 keep mine",
                ),
                row(
                    "resolve-1799999000000",
                    "2026-11-12T10:13:20Z",
                    1,
                    "1 merge",
                ),
            ],
            unreadable: vec!["broken-1".to_string()],
        }
    }

    fn key(screen: &mut HistoryScreen, code: KeyCode) -> Option<HistoryAction> {
        screen.handle(&Event::Key(KeyEvent::press(code)))
    }

    fn click(screen: &mut HistoryScreen, column: u16, row: u16) -> Option<HistoryAction> {
        screen.handle(&Event::Mouse(MouseEvent::new(
            MouseKind::Down(MouseButton::Left),
            column,
            row,
        )))
    }

    fn variant(action: &Option<HistoryAction>) -> Option<String> {
        action.as_ref().map(|found| {
            let text = format!("{found:?}");
            text.split(['(', ' '])
                .next()
                .unwrap_or_default()
                .to_string()
        })
    }

    #[test]
    fn nothing_a_click_can_ask_for_is_missing_from_the_keys() {
        let mut by_mouse = std::collections::BTreeSet::new();
        for (columns, rows) in [(80_u16, 14_u16), (120, 20)] {
            for row in 0..rows {
                for column in 0..columns {
                    let mut screen = HistoryScreen::new(sample());
                    let _ = text(&mut screen, columns, rows);
                    by_mouse.extend(variant(&click(&mut screen, column, row)));
                }
            }
        }
        let mut by_key = std::collections::BTreeSet::new();
        for command in crate::commands::HISTORY_COMMANDS {
            for chord in command.chords() {
                let mut screen = HistoryScreen::new(sample());
                let _ = text(&mut screen, 120, 20);
                by_key.extend(variant(&key(&mut screen, chord.code())));
            }
        }
        for command in crate::commands::GLOBAL_COMMANDS {
            for chord in command.chords() {
                let mut screen = HistoryScreen::new(sample());
                by_key.extend(variant(&key(&mut screen, chord.code())));
            }
        }
        assert!(by_mouse.len() >= 4, "{by_mouse:?}");
        let only_mouse: Vec<_> = by_mouse.difference(&by_key).collect();
        assert!(only_mouse.is_empty(), "mouse-only actions: {only_mouse:?}");
    }

    #[test]
    fn the_table_matches_its_snapshots() {
        let mut screen = HistoryScreen::new(sample());
        assert_eq!(
            text(&mut screen, 80, 12),
            include_str!("../snapshots/history-80x12.txt")
        );
        let mut screen = HistoryScreen::new(sample());
        screen.set_notice("Undid resolve-1800000000123.");
        assert_eq!(
            text(&mut screen, 110, 10),
            include_str!("../snapshots/history-notice-110x10.txt")
        );
    }

    #[test]
    fn a_tiny_terminal_says_so() {
        let mut screen = HistoryScreen::new(sample());
        assert!(text(&mut screen, 19, 5).contains("Too small"));
    }

    #[test]
    fn keys_move_the_selection_and_ask_for_undo_back_or_refresh() {
        let mut screen = HistoryScreen::new(sample());
        let _ = text(&mut screen, 80, 12);
        assert_eq!(screen.selected_id(), Some("undo-1800000100456"));
        assert_eq!(
            key(&mut screen, KeyCode::Enter),
            Some(HistoryAction::Undo {
                target: "undo-1800000100456".to_string(),
                force: false
            })
        );
        key(&mut screen, KeyCode::Char('j'));
        assert_eq!(screen.selected_id(), Some("resolve-1800000000123"));
        assert_eq!(
            key(&mut screen, KeyCode::Char('U')),
            Some(HistoryAction::Undo {
                target: "resolve-1800000000123".to_string(),
                force: true
            })
        );
        assert_eq!(
            key(&mut screen, KeyCode::Char('u')),
            Some(HistoryAction::Undo {
                target: "resolve-1800000000123".to_string(),
                force: false
            })
        );
        key(&mut screen, KeyCode::Down);
        key(&mut screen, KeyCode::Down);
        key(&mut screen, KeyCode::Down);
        assert_eq!(screen.selected_id(), Some("resolve-1799999000000"));
        key(&mut screen, KeyCode::Char('g'));
        assert_eq!(screen.selected_id(), Some("undo-1800000100456"));
        key(&mut screen, KeyCode::Char('G'));
        assert_eq!(screen.selected_id(), Some("resolve-1799999000000"));
        key(&mut screen, KeyCode::Up);
        key(&mut screen, KeyCode::Char('k'));
        assert_eq!(screen.selected_id(), Some("undo-1800000100456"));
        assert_eq!(
            key(&mut screen, KeyCode::Char('r')),
            Some(HistoryAction::Refresh)
        );
        assert_eq!(
            key(&mut screen, KeyCode::Char('q')),
            Some(HistoryAction::Back)
        );
        assert_eq!(key(&mut screen, KeyCode::Esc), Some(HistoryAction::Back));
        assert_eq!(key(&mut screen, KeyCode::Char('x')), None);
    }

    #[test]
    fn an_empty_history_has_nothing_to_undo() {
        let mut screen = HistoryScreen::new(HistoryView {
            rows: Vec::new(),
            unreadable: Vec::new(),
        });
        let _ = text(&mut screen, 60, 8);
        assert_eq!(screen.selected_id(), None);
        assert_eq!(key(&mut screen, KeyCode::Enter), None);
        assert_eq!(key(&mut screen, KeyCode::Char('U')), None);
    }

    #[test]
    fn clicking_selects_a_row_and_a_hint_runs_its_key() {
        let mut screen = HistoryScreen::new(sample());
        let shown = text(&mut screen, 80, 12);
        let line_of = |needle: &str| {
            shown
                .lines()
                .position(|line| line.contains(needle))
                .unwrap() as u16
        };
        assert_eq!(
            click(&mut screen, 5, line_of("resolve-1799999000000")),
            None
        );
        assert_eq!(screen.selected_id(), Some("resolve-1799999000000"));
        assert_eq!(click(&mut screen, 5, line_of("broken-1")), None);
        assert_eq!(screen.selected_id(), Some("resolve-1799999000000"));
        let hints = line_of("[Q] Back");
        let column = |needle: &str| {
            shown
                .lines()
                .nth(hints as usize)
                .unwrap()
                .find(needle)
                .unwrap() as u16
                + 1
        };
        assert_eq!(
            click(&mut screen, column("[Enter] Undo"), hints),
            Some(HistoryAction::Undo {
                target: "resolve-1799999000000".to_string(),
                force: false
            })
        );
        assert_eq!(
            click(&mut screen, column("[Q] Back"), hints),
            Some(HistoryAction::Back)
        );
        assert_eq!(click(&mut screen, 0, 0), None);
        let right = Event::Mouse(MouseEvent::new(MouseKind::Down(MouseButton::Right), 5, 4));
        assert_eq!(screen.handle(&right), None);
    }

    #[test]
    fn a_long_history_scrolls_to_keep_the_selection_visible() {
        let rows: Vec<HistoryRow> = (0..40)
            .map(|index| {
                row(
                    &format!("resolve-{index:03}"),
                    "2026-11-12T10:00:00Z",
                    1,
                    "1 merge",
                )
            })
            .collect();
        let mut screen = HistoryScreen::new(HistoryView {
            rows,
            unreadable: Vec::new(),
        });
        let first = text(&mut screen, 80, 12);
        assert!(first.contains("resolve-000") && !first.contains("resolve-020"));
        for _ in 0..20 {
            key(&mut screen, KeyCode::Char('j'));
        }
        let later = text(&mut screen, 80, 12);
        assert!(
            later.contains("resolve-020") && !later.contains("resolve-000"),
            "{later}"
        );
        key(&mut screen, KeyCode::Char('G'));
        assert!(text(&mut screen, 80, 12).contains("resolve-039"));
        key(&mut screen, KeyCode::PageUp);
        key(&mut screen, KeyCode::Home);
        assert!(text(&mut screen, 80, 12).contains("resolve-000"));
        let wheel = |kind| Event::Mouse(MouseEvent::new(MouseKind::Scroll(kind), 5, 5));
        screen.handle(&wheel(Scroll::Down));
        assert!(!text(&mut screen, 80, 12).contains("resolve-000"));
        screen.handle(&wheel(Scroll::Up));
        assert!(text(&mut screen, 80, 12).contains("resolve-000"));
    }

    #[test]
    fn a_notice_shows_until_the_next_key_and_replace_keeps_it_and_the_selection() {
        let mut screen = HistoryScreen::new(sample());
        key(&mut screen, KeyCode::Char('j'));
        screen.set_notice("Done.");
        let mut shorter = sample();
        shorter.rows.truncate(1);
        screen.replace(shorter);
        assert_eq!(screen.notice(), Some("Done."));
        assert_eq!(screen.selected_id(), Some("undo-1800000100456"));
        key(&mut screen, KeyCode::Char('j'));
        assert_eq!(screen.notice(), None);
    }

    #[test]
    fn replace_keeps_the_same_transaction_selected_when_a_new_one_appears_above_it() {
        let mut screen = HistoryScreen::new(sample());
        key(&mut screen, KeyCode::Char('j'));
        assert_eq!(screen.selected_id(), Some("resolve-1800000000123"));
        let mut longer = sample();
        longer.rows.insert(
            0,
            row("undo-1800000200000", "2026-11-12T10:33:20Z", 1, "1 undo"),
        );
        screen.replace(longer);
        assert_eq!(screen.selected_id(), Some("resolve-1800000000123"));

        let mut gone = sample();
        gone.rows.retain(|row| row.id != "resolve-1800000000123");
        screen.replace(gone);
        assert_eq!(screen.selected_id(), Some("resolve-1799999000000"));
    }

    macro_rules! state_snapshots {
        ($($name:literal),* $(,)?) => {
            &[$(($name, include_str!(concat!("../snapshots/", $name, ".txt")))),*]
        };
    }

    const STATE_SNAPSHOTS: &[(&str, &str)] = state_snapshots![
        "history-state-empty-60x10",
        "history-state-empty-80x14",
        "history-state-empty-120x20",
        "history-state-failed-60x10",
        "history-state-failed-80x14",
        "history-state-failed-120x20"
    ];

    fn state_snapshot(name: &str) -> &'static str {
        STATE_SNAPSHOTS
            .iter()
            .find(|(known, _)| *known == name)
            .map(|(_, text)| *text)
            .unwrap_or_else(|| panic!("no snapshot {name}"))
    }

    #[test]
    fn the_empty_and_failed_states_match_their_snapshots_at_three_sizes() {
        for (columns, rows) in [(60, 10), (80, 14), (120, 20)] {
            let mut empty = HistoryScreen::new(HistoryView {
                rows: Vec::new(),
                unreadable: Vec::new(),
            });
            assert_eq!(
                text(&mut empty, columns, rows),
                state_snapshot(&format!("history-state-empty-{columns}x{rows}")),
                "empty {columns}x{rows}"
            );
            let mut failed = HistoryScreen::failed(
                "could not read /home/user/.local/share/drift/state/backups: Permission denied (os error 13)",
            );
            assert_eq!(
                text(&mut failed, columns, rows),
                state_snapshot(&format!("history-state-failed-{columns}x{rows}")),
                "failed {columns}x{rows}"
            );
        }
    }

    #[test]
    fn an_empty_or_failed_history_offers_refresh_and_no_undo() {
        let bare = |screen: &mut HistoryScreen| {
            text(screen, 80, 14)
                .lines()
                .last()
                .unwrap_or_default()
                .trim_end()
                .to_string()
        };
        let mut empty = HistoryScreen::new(HistoryView {
            rows: Vec::new(),
            unreadable: Vec::new(),
        });
        assert_eq!(
            bare(&mut empty),
            "[Q] Back  [?] Help  [:] Palette  [r] Refresh"
        );
        assert_eq!(key(&mut empty, KeyCode::Enter), None);
        assert_eq!(key(&mut empty, KeyCode::Char('U')), None);
        assert_eq!(
            key(&mut empty, KeyCode::Char('r')),
            Some(HistoryAction::Refresh)
        );
        let reason = "could not read /home/user/.local/share/drift/state/backups: Permission denied (os error 13)";
        for (columns, rows) in [(30, 10), (60, 10), (200, 40)] {
            let mut failed = HistoryScreen::failed(reason);
            let shown = text(&mut failed, columns, rows);
            let squeezed: String = shown.split_whitespace().collect();
            let wanted: String = reason.split_whitespace().collect();
            assert!(squeezed.contains(&wanted), "{columns}x{rows}\n{shown}");
            assert!(shown.contains("Could not read the history."), "{shown}");
        }
        let mut listed = HistoryScreen::new(sample());
        assert!(text(&mut listed, 120, 14).contains("[Enter] Undo"));
    }

    #[test]
    fn an_unreadable_transaction_names_its_manifest_file() {
        let mut screen = HistoryScreen::new(sample());
        let shown = text(&mut screen, 100, 12);
        assert!(
            shown.contains("broken-1  (backups/broken-1/manifest.json could not be read)"),
            "{shown}"
        );
    }
}
