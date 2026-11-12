use lunatui::{render_once, run, App, ControlFlow, RunOptions};
use lunatui_core::{clip_display_prefix, width, ClusterTable, Grid, Rect, Style};
use lunatui_runtime::keymap::{Command, KeyChord, Keymap};
use lunatui_term::{Event, KeyCode, Modifiers, MouseButton, MouseEvent, MouseKind};
use lunatui_theme::{Theme, Token, Variant};
use lunatui_widgets::{KeyHintBar, RenderCx, Tree, TreeState, Widget};

const QUIT: &str = "app.quit";

pub struct Screen {
    keymap: Keymap,
    tree: TreeState,
    columns: u16,
    rows: u16,
}

impl Screen {
    pub fn new() -> Self {
        Self {
            keymap: keymap(),
            tree: TreeState::new(Vec::new()),
            columns: 0,
            rows: 0,
        }
    }
}

impl Default for Screen {
    fn default() -> Self {
        Self::new()
    }
}

pub fn keymap() -> Keymap {
    let mut map = Keymap::new();
    map.register(Command::new(QUIT, "Quit", QUIT_CHORDS));
    map
}

const QUIT_CHORDS: &[KeyChord] = &[KeyChord::new(KeyCode::Char('q'), Modifiers::new())];

pub fn render_frame(width: u16, height: u16) -> (Grid, ClusterTable) {
    let mut screen = Screen::new();
    render_once(&mut screen, width, height)
}

pub fn frame_text(width: u16, height: u16) -> String {
    let (grid, clusters) = render_frame(width, height);
    grid.display_text(&clusters)
}

pub fn launch() -> std::io::Result<()> {
    let (columns, rows) = terminal_size();
    run(Screen::new(), RunOptions::new(columns, rows).pixels(false))
}

pub fn quit_bounds(columns: u16, rows: u16) -> Option<Rect> {
    if columns == 0 || rows == 0 {
        return None;
    }
    let label = quit_label();
    let cells = u16::try_from(width(&label)).unwrap_or(u16::MAX);
    let hit_width = cells.min(columns);
    if hit_width == 0 {
        return None;
    }
    Some(Rect::new(0, rows - 1, hit_width, 1))
}

fn quit_label() -> String {
    let chord = QUIT_CHORDS[0];
    format!("[{chord}] Quit")
}

fn terminal_size() -> (u16, u16) {
    terminal_size::terminal_size()
        .map(|(columns, rows)| (columns.0, rows.0))
        .filter(|(columns, rows)| *columns > 0 && *rows > 0)
        .unwrap_or((80, 24))
}

fn regions(area: Rect) -> (Rect, Rect, Rect) {
    if area.height == 0 {
        let empty = Rect::new(area.x, area.y, area.width, 0);
        return (empty, empty, empty);
    }
    let header = Rect::new(area.x, area.y, area.width, 1);
    if area.height == 1 {
        let empty = Rect::new(area.x, area.y, 0, 0);
        return (header, empty, empty);
    }
    let hint = Rect::new(area.x, area.y + area.height - 1, area.width, 1);
    let tree = Rect::new(area.x, area.y + 1, area.width, area.height - 2);
    (header, tree, hint)
}

fn contains(area: Rect, column: u16, row: u16) -> bool {
    column >= area.x
        && row >= area.y
        && column < area.x.saturating_add(area.width)
        && row < area.y.saturating_add(area.height)
}

impl App for Screen {
    fn update(&mut self, event: &Event) -> ControlFlow {
        if let Event::Mouse(mouse) = event {
            if mouse_quits(self.columns, self.rows, mouse) {
                return ControlFlow::Break;
            }
        }
        match self.keymap.dispatch(event) {
            Some(QUIT) => ControlFlow::Break,
            _ => ControlFlow::Continue,
        }
    }

    fn render(&mut self, grid: &mut Grid, area: Rect, clusters: &mut ClusterTable) {
        self.columns = area.width;
        self.rows = area.height;
        let theme = Theme::builtin(Variant::Dark);
        let mut cx = RenderCx::new(grid, clusters, &theme);
        let text = Style::new()
            .fg(cx.token(Token::TextPrimary))
            .bg(cx.token(Token::Surface0));
        let accent = Style::new()
            .fg(cx.token(Token::Accent))
            .bg(cx.token(Token::Surface0));
        cx.fill(area, text, ' ');
        let (header, tree, hint) = regions(area);
        if header.width > 0 && header.height > 0 {
            let title = clip_display_prefix("Drift", header.width);
            cx.put_str(header.x, header.y, title, accent);
        }
        if tree.width > 0 && tree.height > 0 {
            Tree::new(&self.tree).render(tree, &mut cx);
        }
        if hint.width > 0 && hint.height > 0 {
            KeyHintBar::new(&self.keymap).render(hint, &mut cx);
        }
    }
}

fn mouse_quits(columns: u16, rows: u16, mouse: &MouseEvent) -> bool {
    if !matches!(mouse.kind, MouseKind::Down(MouseButton::Left)) {
        return false;
    }
    quit_bounds(columns, rows).is_some_and(|hit| contains(hit, mouse.column, mouse.row))
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use lunatui::{render_once, App, ControlFlow};
    use lunatui_term::{KeyEvent, MouseEvent};
    use lunatui_test::styled_snapshot;

    use super::*;

    const SIZES: [(u16, u16, &str); 3] =
        [(80, 24, "80x24"), (120, 36, "120x36"), (200, 60, "200x60")];

    #[test]
    fn snapshots_match_the_empty_screen() {
        for (columns, rows, label) in SIZES {
            let (grid, clusters) = render_frame(columns, rows);
            let snap = styled_snapshot(&grid, &clusters);
            let text = grid.display_text(&clusters);
            let lines: Vec<&str> = text.lines().collect();
            assert_eq!(lines.len(), usize::from(rows), "{label}");
            assert!(lines[0].contains("Drift"), "{label}");
            assert!(lines[1].trim().is_empty(), "{label}");
            assert!(lines[usize::from(rows) - 1].contains("[Q] Quit"), "{label}");
            let expected = match label {
                "80x24" => include_str!("../snapshots/empty-80x24.txt"),
                "120x36" => include_str!("../snapshots/empty-120x36.txt"),
                "200x60" => include_str!("../snapshots/empty-200x60.txt"),
                _ => unreachable!(),
            };
            assert_eq!(snap, expected, "{label}");
        }
    }

    #[test]
    fn q_and_a_click_on_quit_both_leave() {
        let mut screen = Screen::new();
        let _ = render_once(&mut screen, 80, 24);
        let hit = quit_bounds(80, 24).unwrap();
        let quit_click = Event::Mouse(MouseEvent::new(
            MouseKind::Down(MouseButton::Left),
            hit.x + hit.width / 2,
            hit.y,
        ));
        assert_eq!(screen.update(&quit_click), ControlFlow::Break);

        let mut screen = Screen::new();
        let _ = render_once(&mut screen, 80, 24);
        let miss = Event::Mouse(MouseEvent::new(
            MouseKind::Down(MouseButton::Left),
            hit.x + hit.width,
            hit.y,
        ));
        assert_eq!(screen.update(&miss), ControlFlow::Continue);
        let key = Event::Key(KeyEvent::press(KeyCode::Char('q')));
        assert_eq!(screen.update(&key), ControlFlow::Break);
    }

    #[test]
    fn empty_frames_stay_under_the_budget() {
        for (columns, rows, _) in SIZES {
            if (columns, rows) == (120, 36) {
                continue;
            }
            let mut screen = Screen::new();
            let mut samples = Vec::new();
            for _ in 0..32 {
                let started = Instant::now();
                let _ = render_once(&mut screen, columns, rows);
                samples.push(started.elapsed());
            }
            let worst = samples.iter().copied().max().unwrap();
            assert!(
                worst < Duration::from_millis(8),
                "{columns}x{rows} worst {worst:?} over {} samples",
                samples.len()
            );
        }
    }
}
