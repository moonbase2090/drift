use lunatui_core::{clip_display_prefix, width, Attrs, ClusterTable, Grid, Rect, Style};
use lunatui_term::{Event, KeyCode, KeyKind, MouseButton, MouseKind, Scroll};
use lunatui_theme::{Theme, Token, Variant};
use lunatui_widgets::RenderCx;

use drift_core::HistoryView;

const WHEEL_STEP: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryAction {
    Back,
    Refresh,
    Undo { target: String, force: bool },
}

const HINTS: &[(&str, KeyCode)] = &[
    ("[Q] Back", KeyCode::Char('q')),
    ("[Enter] Undo", KeyCode::Enter),
    ("[U] Undo and force", KeyCode::Char('U')),
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
        let selected = self.selected;
        *self = Self::new(view);
        self.notice = notice;
        self.selected = selected.min(self.view.rows.len().saturating_sub(1));
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
        match code {
            KeyCode::Char('q') | KeyCode::Esc => return Some(HistoryAction::Back),
            KeyCode::Char('r') => return Some(HistoryAction::Refresh),
            KeyCode::Enter | KeyCode::Char('u') => return self.undo(false),
            KeyCode::Char('U') => return self.undo(true),
            KeyCode::Char('j') | KeyCode::Down => self.move_by(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_by(-1),
            KeyCode::Char('g') | KeyCode::Home => self.selected = 0,
            KeyCode::Char('G') | KeyCode::End => {
                self.selected = self.view.rows.len().saturating_sub(1);
            }
            KeyCode::PageDown => self.move_by(self.page()),
            KeyCode::PageUp => self.move_by(-self.page()),
            _ => {}
        }
        None
    }

    fn page(&self) -> isize {
        self.area
            .and_then(layout)
            .map_or(10, |found| (found.body.height / 2).max(1) as isize)
    }

    fn move_by(&mut self, delta: isize) {
        let last = self.view.rows.len().saturating_sub(1);
        self.selected = self.selected.saturating_add_signed(delta).min(last);
    }

    fn mouse(&mut self, kind: MouseKind, column: u16, row: u16) -> Option<HistoryAction> {
        let found = self.area.and_then(layout)?;
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
        let Some(found) = layout(area) else {
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
            Some(message) => format!(" Drift  history failed: {message}"),
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
            put(
                &mut cx,
                Rect::new(
                    found.body.x + 1,
                    found.body.y,
                    found.body.width.saturating_sub(1),
                    1,
                ),
                if self.error.is_some() {
                    "Nothing was loaded."
                } else {
                    "No transactions yet."
                },
                dim,
            );
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

fn layout(area: Rect) -> Option<Layout> {
    if area.width < 20 || area.height < 6 {
        return None;
    }
    let hint_y = area.y + area.height - 1;
    let mut hints = Vec::new();
    let mut x = area.x;
    for (label, code) in HINTS {
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
        let mut empty = HistoryScreen::new(HistoryView {
            rows: Vec::new(),
            unreadable: Vec::new(),
        });
        assert_eq!(
            text(&mut empty, 60, 8),
            include_str!("../snapshots/history-empty-60x8.txt")
        );
        let mut failed = HistoryScreen::failed("state directory is unreadable");
        assert_eq!(
            text(&mut failed, 60, 8),
            include_str!("../snapshots/history-failed-60x8.txt")
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
}
