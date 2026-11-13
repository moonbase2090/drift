use std::time::{Duration, Instant};

use lunatui_core::{clip_display_prefix, width, Attrs, ClusterTable, Grid, Rect, Style};
use lunatui_term::{Event, KeyCode, KeyKind, MouseButton, MouseKind, Scroll};
use lunatui_theme::{Theme, Token, Variant};
use lunatui_widgets::RenderCx;

use lunatui_runtime::keymap::Keymap;

use drift_core::{RowKind, ScanReport, Status, TreeRow, TreeView};

use crate::commands::{chords_text, global_keymap, press, tree_keymap, GlobalCommand, TreeCommand};

const DOUBLE_CLICK: Duration = Duration::from_millis(400);
const WHEEL_STEP: usize = 3;
const WIDE: u16 = 100;
const SPLIT: u16 = 110;
const SUMMARY_ROWS: u16 = 8;
const LABEL_WIDTH: usize = 16;

pub fn glyph(status: Status) -> &'static str {
    match status {
        Status::Unchanged => "✓",
        Status::UpstreamChanged => "↓",
        Status::LocalOverride => "≠",
        Status::Conflict => "✗",
        Status::UserOnly => "+",
        Status::Missing => "?",
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Quit,
    Rescan,
    Open(String),
    Keep(Vec<String>),
    Take(Vec<String>),
    Merge(Vec<String>),
    Ignore(Vec<String>),
    History,
    Help,
    Palette,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Normal,
    Search,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Panel {
    Failed,
    Untracked,
    NoDrift,
    NoMatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    Caret,
    Checkbox,
    Body,
}

struct Layout {
    title: Rect,
    counts: Vec<(Rect, Status)>,
    info: Rect,
    tree: Rect,
    summary: Option<Rect>,
    divider: Option<Rect>,
    hint: Rect,
    hints: Vec<(Rect, KeyCode, String)>,
}

pub struct TreeScreen {
    view: TreeView,
    package_version: String,
    pending: Option<(String, String)>,
    error: Option<String>,
    notice: Option<String>,
    mode: Mode,
    keymap: Keymap,
    globals: Keymap,
    scroll: usize,
    follow: bool,
    area: Option<Rect>,
    last_click: Option<(usize, Instant)>,
    hover: Option<Status>,
}

const BARE_HINTS: &[(&str, KeyCode)] = &[
    ("[Q] Quit", KeyCode::Char('q')),
    ("[?] Help", KeyCode::Char('?')),
    ("[:] Palette", KeyCode::Char(':')),
    ("[R] Rescan", KeyCode::Char('r')),
    ("[H] History", KeyCode::Char('H')),
];

const NO_MATCH_HINTS: &[(&str, KeyCode)] = &[
    ("[Q] Quit", KeyCode::Char('q')),
    ("[?] Help", KeyCode::Char('?')),
    ("[:] Palette", KeyCode::Char(':')),
    ("[/] Search", KeyCode::Char('/')),
    ("[R] Rescan", KeyCode::Char('r')),
    ("[H] History", KeyCode::Char('H')),
];

pub(crate) const HINTS: &[(&str, KeyCode)] = &[
    ("[Q] Quit", KeyCode::Char('q')),
    ("[?] Help", KeyCode::Char('?')),
    ("[:] Palette", KeyCode::Char(':')),
    ("[/] Search", KeyCode::Char('/')),
    ("[Enter] Open", KeyCode::Enter),
    ("[o] Keep", KeyCode::Char('o')),
    ("[t] Take", KeyCode::Char('t')),
    ("[m] Merge", KeyCode::Char('m')),
    ("[H] History", KeyCode::Char('H')),
];

impl TreeScreen {
    pub fn new(report: &ScanReport) -> Self {
        Self {
            view: TreeView::new(report.entries.clone()),
            package_version: report.package_version.clone(),
            pending: report.update_pending().then(|| {
                (
                    report.reviewed_version.clone().unwrap_or_default(),
                    report.package_version.clone(),
                )
            }),
            error: None,
            notice: None,
            mode: Mode::Normal,
            keymap: tree_keymap(),
            globals: global_keymap(),
            scroll: 0,
            follow: true,
            area: None,
            last_click: None,
            hover: None,
        }
    }

    pub fn failed(message: &str) -> Self {
        let mut screen = Self::new(&ScanReport {
            package_version: String::new(),
            reviewed_version: None,
            entries: Vec::new(),
            backups: Vec::new(),
            ignored: 0,
        });
        screen.error = Some(message.to_string());
        screen
    }

    pub fn set_notice(&mut self, text: &str) {
        self.notice = Some(text.to_string());
    }

    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    pub fn view(&self) -> &TreeView {
        &self.view
    }

    pub fn searching(&self) -> bool {
        self.mode == Mode::Search
    }

    pub fn handle(&mut self, event: &Event) -> Option<Action> {
        self.handle_at(event, Instant::now())
    }

    pub fn handle_at(&mut self, event: &Event, now: Instant) -> Option<Action> {
        if matches!(event, Event::Key(key) if key.kind == KeyKind::Press) {
            self.notice = None;
        }
        match event {
            Event::Key(key) if key.kind == KeyKind::Press => self.key(key.code),
            Event::Mouse(mouse) => self.mouse(mouse.kind, mouse.column, mouse.row, now),
            _ => None,
        }
    }

    fn key(&mut self, code: KeyCode) -> Option<Action> {
        if self.mode == Mode::Search {
            return self.search_key(code);
        }
        let event = press(code);
        if let Some(id) = self.globals.dispatch(&event) {
            return match GlobalCommand::from_id(id)? {
                GlobalCommand::Help => Some(Action::Help),
                GlobalCommand::Palette => Some(Action::Palette),
            };
        }
        let id = self.keymap.dispatch(&event)?;
        self.run(TreeCommand::from_id(id)?)
    }

    pub fn run(&mut self, command: TreeCommand) -> Option<Action> {
        if self.panel() == Some(Panel::NoDrift) && acts_on_rows(command) {
            return None;
        }
        match command {
            TreeCommand::Quit => return Some(Action::Quit),
            TreeCommand::Rescan => return Some(Action::Rescan),
            TreeCommand::Down => self.move_by(1),
            TreeCommand::Up => self.move_by(-1),
            TreeCommand::Collapse => {
                self.view.collapse_or_parent();
                self.follow = true;
            }
            TreeCommand::Expand => {
                self.view.expand_or_child();
                self.follow = true;
            }
            TreeCommand::Top => {
                self.view.top();
                self.follow = true;
            }
            TreeCommand::Bottom => {
                self.view.bottom();
                self.follow = true;
            }
            TreeCommand::PageDown => self.move_by(self.page()),
            TreeCommand::PageUp => self.move_by(-self.page()),
            TreeCommand::Open => return self.open(),
            TreeCommand::Search => self.mode = Mode::Search,
            TreeCommand::ClearFilter => self.view.set_status_filter(None),
            TreeCommand::Mark => self.view.toggle_mark(self.view.selected()),
            TreeCommand::MarkAll => self.view.mark_all_in_view(),
            TreeCommand::Keep => return self.act(Action::Keep),
            TreeCommand::Take => return self.act(Action::Take),
            TreeCommand::Merge => return self.act(Action::Merge),
            TreeCommand::Ignore => return self.act(Action::Ignore),
            TreeCommand::History => return Some(Action::History),
            TreeCommand::Back => self.back(),
            filter => {
                if let Some(status) = filter.status() {
                    self.filter(status);
                }
            }
        }
        None
    }

    fn search_key(&mut self, code: KeyCode) -> Option<Action> {
        match code {
            KeyCode::Char(c) => {
                let mut query = self.view.query().to_string();
                query.push(c);
                self.view.set_query(&query);
                self.follow = true;
            }
            KeyCode::Backspace => {
                let mut query = self.view.query().to_string();
                query.pop();
                self.view.set_query(&query);
                self.follow = true;
            }
            KeyCode::Enter => self.mode = Mode::Normal,
            KeyCode::Esc => {
                self.view.set_query("");
                self.mode = Mode::Normal;
            }
            KeyCode::Down => self.move_by(1),
            KeyCode::Up => self.move_by(-1),
            _ => {}
        }
        None
    }

    fn back(&mut self) {
        if !self.view.query().is_empty() {
            self.view.set_query("");
        } else if self.view.status_filter().is_some() {
            self.view.set_status_filter(None);
        } else {
            self.view.clear_marks();
        }
    }

    fn filter(&mut self, status: Status) {
        let next = if self.view.status_filter() == Some(status) {
            None
        } else {
            Some(status)
        };
        self.view.set_status_filter(next);
        self.follow = true;
    }

    fn page(&self) -> isize {
        self.area
            .and_then(|area| self.layout(area))
            .map_or(10, |found| (found.tree.height / 2).max(1) as isize)
    }

    fn move_by(&mut self, delta: isize) {
        self.view.move_by(delta);
        self.follow = true;
    }

    fn open(&mut self) -> Option<Action> {
        let row = self.view.selected_row()?.clone();
        match row.kind {
            RowKind::File => self
                .view
                .selected_entry()
                .map(|entry| Action::Open(entry.path.clone())),
            RowKind::Dir { .. } => {
                self.view.toggle(self.view.selected());
                self.follow = true;
                None
            }
        }
    }

    fn targets(&self) -> Vec<String> {
        if self.view.marked_count() > 0 {
            return self
                .view
                .marked()
                .iter()
                .map(|entry| entry.path.clone())
                .collect();
        }
        match self.view.selected_row() {
            Some(row) if row.entry.is_some() => self
                .view
                .selected_entry()
                .map(|entry| vec![entry.path.clone()])
                .unwrap_or_default(),
            Some(row) => self.view.paths_under(row.node),
            None => Vec::new(),
        }
    }

    fn act(&self, make: fn(Vec<String>) -> Action) -> Option<Action> {
        let targets = self.targets();
        (!targets.is_empty()).then(|| make(targets))
    }

    fn mouse(&mut self, kind: MouseKind, column: u16, row: u16, now: Instant) -> Option<Action> {
        let found = self.area.and_then(|area| self.layout(area))?;
        match kind {
            MouseKind::Down(MouseButton::Left) => self.click(&found, column, row, now),
            MouseKind::Move => {
                self.hover = found
                    .counts
                    .iter()
                    .find(|(rect, _)| contains(*rect, column, row))
                    .map(|(_, status)| *status);
                None
            }
            MouseKind::Scroll(Scroll::Down) => {
                self.scroll = self.scroll.saturating_add(WHEEL_STEP);
                self.follow = false;
                None
            }
            MouseKind::Scroll(Scroll::Up) => {
                self.scroll = self.scroll.saturating_sub(WHEEL_STEP);
                self.follow = false;
                None
            }
            _ => None,
        }
    }

    fn click(&mut self, found: &Layout, column: u16, row: u16, now: Instant) -> Option<Action> {
        if let Some((_, status)) = found
            .counts
            .iter()
            .find(|(rect, _)| contains(*rect, column, row))
        {
            self.filter(*status);
            return None;
        }
        if let Some((_, code, _)) = found
            .hints
            .iter()
            .find(|(rect, _, _)| contains(*rect, column, row))
        {
            return self.key(*code);
        }
        if !contains(found.tree, column, row) || self.panel().is_some() {
            return None;
        }
        let index = self.scroll + usize::from(row - found.tree.y);
        let target = self.view.rows().get(index).cloned()?;
        let hit = row_hit(&target, column - found.tree.x);
        self.view.select(index);
        self.follow = true;
        match hit {
            Hit::Caret => {
                self.view.toggle(index);
                None
            }
            Hit::Checkbox => {
                self.view.toggle_mark(index);
                None
            }
            Hit::Body => {
                let double = self.last_click.is_some_and(|(at, when)| {
                    at == target.node && now.duration_since(when) <= DOUBLE_CLICK
                });
                self.last_click = Some((target.node, now));
                if double {
                    self.last_click = None;
                    self.open()
                } else {
                    None
                }
            }
        }
    }

    fn layout(&self, area: Rect) -> Option<Layout> {
        if area.width < 20 || area.height < 6 {
            return None;
        }
        let counts = count_items(&self.view, area.width);
        let count_rows = counts
            .iter()
            .map(|(rect, _)| rect.y)
            .max()
            .map_or(1, |y| y + 1);
        let title = Rect::new(area.x, area.y, area.width, 1);
        let counts: Vec<(Rect, Status)> = counts
            .into_iter()
            .map(|(rect, status)| {
                (
                    Rect::new(area.x + rect.x, area.y + 1 + rect.y, rect.width, 1),
                    status,
                )
            })
            .collect();
        let info = Rect::new(area.x, area.y + 1 + count_rows, area.width, 1);
        let hint = Rect::new(area.x, area.y + area.height - 1, area.width, 1);
        let top = info.y + 1;
        let body_height = hint.y.saturating_sub(top);
        let (tree, summary, divider) = if area.width >= SPLIT {
            let tree_width = area.width * 3 / 5;
            (
                Rect::new(area.x, top, tree_width, body_height),
                Some(Rect::new(
                    area.x + tree_width + 2,
                    top,
                    area.width - tree_width - 2,
                    body_height,
                )),
                Some(Rect::new(area.x + tree_width, top, 1, body_height)),
            )
        } else if body_height >= SUMMARY_ROWS + 6 {
            (
                Rect::new(area.x, top, area.width, body_height - SUMMARY_ROWS),
                Some(Rect::new(
                    area.x,
                    top + body_height - SUMMARY_ROWS + 1,
                    area.width,
                    SUMMARY_ROWS - 1,
                )),
                Some(Rect::new(
                    area.x,
                    top + body_height - SUMMARY_ROWS,
                    area.width,
                    1,
                )),
            )
        } else {
            (Rect::new(area.x, top, area.width, body_height), None, None)
        };
        let mut hints = Vec::new();
        let mut x = area.x;
        for (label, code) in self.hints() {
            let cells = u16::try_from(width(label)).unwrap_or(u16::MAX);
            if x + cells > area.x + area.width {
                break;
            }
            hints.push((Rect::new(x, hint.y, cells, 1), *code, (*label).to_string()));
            x += cells + 2;
        }
        Some(Layout {
            title,
            counts,
            info,
            tree,
            summary,
            divider,
            hint,
            hints,
        })
    }

    fn panel(&self) -> Option<Panel> {
        if self.error.is_some() {
            return Some(Panel::Failed);
        }
        if self.view.entries().is_empty() {
            return Some(Panel::Untracked);
        }
        if self.view.rows().is_empty() {
            return Some(Panel::NoMatch);
        }
        let narrowed = !self.view.query().is_empty() || self.view.status_filter().is_some();
        (self.view.drifted() == 0 && !narrowed).then_some(Panel::NoDrift)
    }

    fn hints(&self) -> &'static [(&'static str, KeyCode)] {
        match self.panel() {
            None => HINTS,
            Some(Panel::NoMatch) => NO_MATCH_HINTS,
            Some(_) => BARE_HINTS,
        }
    }

    fn panel_lines(&self, panel: Panel) -> (String, Vec<String>) {
        let total = self.view.entries().len();
        match panel {
            Panel::Failed => (
                "Scan failed".to_string(),
                vec![
                    self.error.clone().unwrap_or_default(),
                    String::new(),
                    "Nothing was changed. Check that omarchy-settings is installed and that the package database and your config files can be read, then rescan."
                        .to_string(),
                ],
            ),
            Panel::Untracked => (
                "No tracked files".to_string(),
                vec![
                    "The installed package lists no files for Drift to track.".to_string(),
                    String::new(),
                    "If you expected some, check that omarchy-settings is installed and up to date, then rescan."
                        .to_string(),
                ],
            ),
            Panel::NoDrift => {
                let counts = self.view.counts();
                let mut lines = vec!["Nothing needs review.".to_string()];
                let parts: Vec<String> = Status::ALL
                    .iter()
                    .filter(|status| counts[status_index(**status)] > 0)
                    .map(|status| format!("{} {}", counts[status_index(*status)], status.label()))
                    .collect();
                lines.push(format!("Of {total} tracked files: {}.", parts.join(", ")));
                if let Some((from, to)) = &self.pending {
                    lines.push(format!(
                        "Package {to} is waiting for review (last reviewed {from})."
                    ));
                }
                lines.push(String::new());
                lines.push(
                    "Press 1 to 6 to list the files with one status, or open History to look back."
                        .to_string(),
                );
                ("No drift".to_string(), lines)
            }
            Panel::NoMatch => {
                let mut lines = Vec::new();
                if !self.view.query().is_empty() {
                    lines.push(format!("Search: {}", self.view.query()));
                }
                if let Some(status) = self.view.status_filter() {
                    lines.push(format!("Filter: {}", status.label()));
                }
                lines.push(String::new());
                lines.push(format!(
                    "Press {} to clear the search or filter.",
                    chords_text(TreeCommand::Back.chords())
                ));
                ("No files match".to_string(), lines)
            }
        }
    }

    fn draw_panel(&self, cx: &mut RenderCx<'_>, rect: Rect, panel: Panel, base: Style, dim: Style) {
        let (title, body) = self.panel_lines(panel);
        let width = usize::from(rect.width.saturating_sub(2));
        let title_style = match panel {
            Panel::Failed => base
                .fg(cx.token(Token::Fail))
                .with_attrs(Attrs::new().set_bold(true)),
            Panel::NoDrift => base
                .fg(cx.token(Token::Ok))
                .with_attrs(Attrs::new().set_bold(true)),
            _ => base.with_attrs(Attrs::new().set_bold(true)),
        };
        let mut lines = vec![(title, title_style)];
        let mut body = body;
        if let Some(notice) = &self.notice {
            body.push(String::new());
            body.push(notice.clone());
        }
        for text in body {
            if text.is_empty() {
                lines.push((text, dim));
            } else {
                for part in wrap(&text, width) {
                    lines.push((part, base));
                }
            }
        }
        for (offset, (text, style)) in lines.iter().take(usize::from(rect.height)).enumerate() {
            put(
                cx,
                Rect::new(
                    rect.x + 1,
                    rect.y + offset as u16,
                    rect.width.saturating_sub(1),
                    1,
                ),
                text,
                *style,
            );
        }
    }

    fn settle_scroll(&mut self, rows: usize) {
        let total = self.view.rows().len();
        if rows == 0 {
            self.scroll = 0;
            return;
        }
        if self.follow {
            let selected = self.view.selected();
            if selected < self.scroll {
                self.scroll = selected;
            } else if selected >= self.scroll + rows {
                self.scroll = selected + 1 - rows;
            }
        }
        self.scroll = self.scroll.min(total.saturating_sub(rows));
    }

    pub fn render(&mut self, grid: &mut Grid, area: Rect, clusters: &mut ClusterTable) {
        self.area = Some(area);
        let theme = Theme::builtin(Variant::Dark);
        let mut cx = RenderCx::new(grid, clusters, &theme);
        let base = Style::new()
            .fg(cx.token(Token::TextPrimary))
            .bg(cx.token(Token::Surface0));
        cx.fill(area, base, ' ');
        let Some(found) = self.layout(area) else {
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

        let mut title = if area.width < WIDE {
            format!(" Drift  {}", self.package_version)
        } else {
            format!(" Drift  omarchy-settings {}", self.package_version)
        };
        let drifted = self.view.drifted();
        title.push_str(&format!("  {drifted} drifted"));
        if let Some((from, to)) = &self.pending {
            title.push_str(&format!("  | update pending review: {from} -> {to}"));
        }
        if self.error.is_some() {
            title = " Drift  scan failed".to_string();
        }
        put(&mut cx, found.title, &title, accent);

        let counts = self.view.counts();
        for (rect, status) in &found.counts {
            let active = self.view.status_filter() == Some(*status);
            let style = if active {
                selected_style
            } else {
                status_style(&cx, base, *status)
            };
            put(
                &mut cx,
                *rect,
                &count_text(*status, counts[status_index(*status)], active),
                style,
            );
        }

        put(&mut cx, found.info, &self.info_text(), dim);
        self.settle_scroll(usize::from(found.tree.height));
        self.draw_rows(&mut cx, &found, base, dim, selected_style);
        if let Some(rect) = found.divider.filter(|_| self.panel().is_none()) {
            let style = dim;
            if rect.width == 1 {
                cx.fill(rect, style, '|');
            } else {
                cx.fill(rect, style, '-');
            }
        }
        let overflow = width(&self.info_text()) > usize::from(found.info.width);
        if let Some(rect) = found.summary.filter(|_| self.panel().is_none()) {
            self.draw_summary(&mut cx, rect, (base, dim, bold), overflow);
        }
        for (rect, _, label) in &found.hints {
            put(&mut cx, *rect, label, dim);
        }
        let _ = found.hint;
    }

    fn info_text(&self) -> String {
        let mut parts = Vec::new();
        if let Some(status) = self.hover {
            parts.push(format!("{}: {}", status.label(), status.explain()));
        }
        if let Some(notice) = &self.notice {
            parts.push(notice.clone());
        }
        if self.mode == Mode::Search {
            parts.push(format!("search: {}_", self.view.query()));
        } else if !self.view.query().is_empty() {
            parts.push(format!("search: {}", self.view.query()));
        }
        if let Some(status) = self.view.status_filter() {
            parts.push(format!("filter: {}", status.label()));
        }
        if self.view.marked_count() > 0 {
            parts.push(format!("{} marked", self.view.marked_count()));
        }
        let total = self.view.entries().len();
        parts.push(format!("{} of {} files", self.view.visible_files(), total));
        format!(" {}", parts.join("  |  "))
    }

    fn draw_rows(
        &self,
        cx: &mut RenderCx<'_>,
        found: &Layout,
        base: Style,
        dim: Style,
        selected_style: Style,
    ) {
        if let Some(panel) = self.panel() {
            self.draw_panel(cx, found.tree, panel, base, dim);
            return;
        }
        let rows = self.view.rows();
        for (offset, row) in rows
            .iter()
            .skip(self.scroll)
            .take(usize::from(found.tree.height))
            .enumerate()
        {
            let rect = Rect::new(
                found.tree.x,
                found.tree.y + offset as u16,
                found.tree.width,
                1,
            );
            let index = self.scroll + offset;
            let selected = index == self.view.selected();
            let style = if selected {
                cx.fill(rect, selected_style, ' ');
                selected_style
            } else {
                match row.entry {
                    Some(entry) => status_style(cx, base, self.view.entries()[entry].status),
                    None => base.with_attrs(Attrs::new().set_bold(true)),
                }
            };
            put(cx, rect, &self.row_text(row, selected), style);
        }
    }

    fn row_text(&self, row: &TreeRow, selected: bool) -> String {
        let marker = if selected { '>' } else { ' ' };
        let indent = " ".repeat(usize::from(row.depth) * 2);
        match &row.kind {
            RowKind::Dir {
                expanded,
                files,
                drifted,
            } => {
                let caret = if *expanded { '▾' } else { '▸' };
                let (marked, total) = self.view.dir_marked(row.node);
                let check = if total > 0 && marked == total {
                    "[x]"
                } else if marked > 0 {
                    "[-]"
                } else {
                    "[ ]"
                };
                format!(
                    "{marker} {indent}{caret} {check} {}/  ({files} {}, {drifted} drifted)",
                    row.name,
                    if *files == 1 { "file" } else { "files" }
                )
            }
            RowKind::File => {
                let entry = &self.view.entries()[row.entry.unwrap_or(0)];
                let check = if row.entry.is_some_and(|index| self.view.is_marked(index)) {
                    "[x]"
                } else {
                    "[ ]"
                };
                let mut text = format!(
                    "{marker} {indent}  {check} {} {:<LABEL_WIDTH$}  {}",
                    glyph(entry.status),
                    entry.status.label(),
                    row.name
                );
                if let Some(badge) = entry.badge {
                    text.push_str(&format!("  [{}]", badge.label()));
                }
                text
            }
        }
    }

    fn draw_summary(
        &self,
        cx: &mut RenderCx<'_>,
        rect: Rect,
        styles: (Style, Style, Style),
        overflow: bool,
    ) {
        let (base, dim, bold) = styles;
        let mut lines: Vec<(String, Style)> = Vec::new();
        if let Some(notice) = self.notice.as_ref().filter(|_| overflow) {
            for part in wrap(notice, usize::from(rect.width)) {
                lines.push((part, bold));
            }
            lines.push((String::new(), dim));
        }
        match self.view.selected_row() {
            Some(row) => match row.entry {
                Some(index) => {
                    let entry = &self.view.entries()[index];
                    lines.push((
                        format!("{} {}", glyph(entry.status), entry.status.label()),
                        status_style(cx, base, entry.status)
                            .with_attrs(Attrs::new().set_bold(true)),
                    ));
                    lines.push((entry.path.clone(), bold));
                    lines.push((
                        format!("template: {}", entry.template.as_deref().unwrap_or("none")),
                        base,
                    ));
                    lines.push((format!("base: {}", entry.base.label()), base));
                    if let Some(badge) = entry.badge {
                        lines.push((format!("badge: {}", badge.label()), base));
                        for part in wrap(badge.explain(), usize::from(rect.width)) {
                            lines.push((part, dim));
                        }
                    }
                    for part in wrap(entry.status.explain(), usize::from(rect.width)) {
                        lines.push((part, dim));
                    }
                    if !entry.note.is_empty() {
                        lines.push((format!("note: {}", entry.note), dim));
                    }
                    if entry.volatile {
                        lines.push(("app rewrites this file".to_string(), dim));
                    }
                }
                None => {
                    lines.push((format!("{}/", row.name), bold));
                    let counts = self.view.dir_status_counts(row.node);
                    for status in Status::ALL {
                        let n = counts[status_index(status)];
                        if n > 0 {
                            lines.push((
                                format!("{} {} {n}", glyph(status), status.label()),
                                status_style(cx, base, status),
                            ));
                        }
                    }
                }
            },
            None => lines.push(("Nothing selected.".to_string(), dim)),
        }
        for (offset, (text, style)) in lines.iter().take(usize::from(rect.height)).enumerate() {
            put(
                cx,
                Rect::new(rect.x, rect.y + offset as u16, rect.width, 1),
                text,
                *style,
            );
        }
    }
}

pub fn row_hit(row: &TreeRow, column: u16) -> Hit {
    let base = 2 + row.depth * 2;
    let (caret_at, check_at) = match row.kind {
        RowKind::Dir { .. } => (Some(base), base + 2),
        RowKind::File => (None, base + 4),
    };
    if caret_at == Some(column) {
        Hit::Caret
    } else if (check_at..check_at + 3).contains(&column) {
        Hit::Checkbox
    } else {
        Hit::Body
    }
}

fn count_text(status: Status, count: usize, active: bool) -> String {
    let number = status_index(status) + 1;
    let text = format!("{number} {} {} {count}", glyph(status), status.label());
    if active {
        format!("[{text}]")
    } else {
        format!(" {text} ")
    }
}

fn count_items(view: &TreeView, columns: u16) -> Vec<(Rect, Status)> {
    let counts = view.counts();
    let mut items = Vec::new();
    let (mut x, mut y) = (0u16, 0u16);
    for status in Status::ALL {
        let text = count_text(status, counts[status_index(status)], false);
        let cells = u16::try_from(width(&text)).unwrap_or(u16::MAX);
        if columns < WIDE && status_index(status) == 3 {
            x = 0;
            y = 1;
        } else if x + cells > columns && x > 0 {
            x = 0;
            y += 1;
        }
        items.push((Rect::new(x, y, cells.min(columns), 1), status));
        x += cells;
    }
    items
}

fn status_index(status: Status) -> usize {
    Status::ALL
        .iter()
        .position(|candidate| *candidate == status)
        .unwrap_or(0)
}

fn status_style(cx: &RenderCx<'_>, base: Style, status: Status) -> Style {
    match status {
        Status::Unchanged => base.fg(cx.token(Token::Ok)),
        Status::UpstreamChanged => base.fg(cx.token(Token::Warn)),
        Status::LocalOverride => base.fg(cx.token(Token::Accent)),
        Status::Conflict => base.fg(cx.token(Token::Fail)),
        Status::UserOnly => base.with_attrs(Attrs::new().set_dim(true)),
        Status::Missing => base
            .fg(cx.token(Token::Warn))
            .with_attrs(Attrs::new().set_bold(true)),
    }
}

fn acts_on_rows(command: TreeCommand) -> bool {
    matches!(
        command,
        TreeCommand::Open
            | TreeCommand::Mark
            | TreeCommand::MarkAll
            | TreeCommand::Keep
            | TreeCommand::Take
            | TreeCommand::Merge
            | TreeCommand::Ignore
    )
}

pub(crate) fn wrap(text: &str, columns: usize) -> Vec<String> {
    let columns = columns.max(1);
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split(' ') {
        let mut rest = word;
        while width(rest) > columns {
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
            }
            let head = clip_display_prefix(rest, u16::try_from(columns).unwrap_or(u16::MAX));
            let cut = head
                .len()
                .max(rest.chars().next().map_or(1, char::len_utf8));
            lines.push(rest[..cut].to_string());
            rest = &rest[cut..];
        }
        let joined = if line.is_empty() {
            width(rest)
        } else {
            width(&line) + 1 + width(rest)
        };
        if joined > columns && !line.is_empty() {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(rest);
    }
    if !line.is_empty() || lines.is_empty() {
        lines.push(line);
    }
    lines
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
    use std::time::Instant;

    use drift_fixtures::cpu_cost;
    use lunatui::{render_once, App, ControlFlow};
    use lunatui_term::{KeyEvent, MouseEvent};

    use drift_core::{Badge, BaseProvenance, ScanEntry};

    use super::*;

    fn entry(path: &str, status: Status) -> ScanEntry {
        ScanEntry {
            path: path.to_string(),
            template: Some(format!("config/{}", path.trim_start_matches(".config/"))),
            status,
            badge: None,
            base: BaseProvenance::None,
            volatile: false,
            note: String::new(),
        }
    }

    fn sample_report() -> ScanReport {
        let mut entries = vec![
            entry(".config/hypr/monitors.lua", Status::Conflict),
            entry(".config/hypr/looknfeel.lua", Status::UpstreamChanged),
            entry(".config/hypr/input.lua", Status::Unchanged),
            entry(".config/waybar/style.css", Status::LocalOverride),
            entry(".config/waybar/config.jsonc", Status::Unchanged),
            entry(".config/btop/btop.conf", Status::Missing),
            entry(".config/alacritty.toml", Status::UserOnly),
            entry(
                ".config/chromium/Default/Preferences",
                Status::LocalOverride,
            ),
        ];
        entries[1].badge = Some(Badge::MergesCleanly);
        entries[1].base = BaseProvenance::Reviewed;
        entries[3].badge = Some(Badge::BaseUnknown);
        entries[7].volatile = true;
        entries[7].note = "app rewrites this file".to_string();
        ScanReport {
            package_version: "4.5.0-3".to_string(),
            reviewed_version: Some("4.4.0-1".to_string()),
            entries,
            backups: Vec::new(),
            ignored: 0,
        }
    }

    fn big_report(count: usize) -> ScanReport {
        let entries = (0..count)
            .map(|n| {
                entry(
                    &format!(".config/dir{:03}/file{n:04}.conf", n % 120),
                    Status::ALL[n % 6],
                )
            })
            .collect();
        ScanReport {
            package_version: "4.5.0-3".to_string(),
            reviewed_version: None,
            entries,
            backups: Vec::new(),
            ignored: 0,
        }
    }

    struct Driver<'a>(&'a mut TreeScreen);

    impl App for Driver<'_> {
        fn update(&mut self, event: &Event) -> ControlFlow {
            let _ = self.0.handle(event);
            ControlFlow::Continue
        }

        fn render(&mut self, grid: &mut Grid, area: Rect, clusters: &mut ClusterTable) {
            self.0.render(grid, area, clusters);
        }
    }

    fn text(screen: &mut TreeScreen, columns: u16, rows: u16) -> String {
        let (grid, clusters) = render_once(&mut Driver(screen), columns, rows);
        grid.display_text(&clusters)
    }

    fn opened() -> TreeScreen {
        let mut screen = TreeScreen::new(&sample_report());
        let _ = text(&mut screen, 120, 36);
        screen
    }

    fn key(screen: &mut TreeScreen, code: KeyCode) -> Option<Action> {
        screen.handle(&Event::Key(KeyEvent::press(code)))
    }

    fn press(screen: &mut TreeScreen, keys: &str) {
        for c in keys.chars() {
            let _ = key(screen, KeyCode::Char(c));
        }
    }

    fn selected_path(screen: &TreeScreen) -> String {
        screen
            .view()
            .selected_entry()
            .map(|entry| entry.path.clone())
            .unwrap_or_default()
    }

    fn click_event(column: u16, row: u16) -> Event {
        Event::Mouse(MouseEvent::new(
            MouseKind::Down(MouseButton::Left),
            column,
            row,
        ))
    }

    fn row_of(shown: &str, needle: &str) -> u16 {
        shown
            .lines()
            .position(|line| line.contains(needle))
            .unwrap_or_else(|| panic!("{needle} not on screen:\n{shown}")) as u16
    }

    fn snapshot(name: &str) -> &'static str {
        match name {
            "tree-80x24" => include_str!("../snapshots/tree-80x24.txt"),
            "tree-120x36" => include_str!("../snapshots/tree-120x36.txt"),
            "tree-200x60" => include_str!("../snapshots/tree-200x60.txt"),
            "tree-filtered-80x24" => include_str!("../snapshots/tree-filtered-80x24.txt"),
            "tree-search-120x36" => include_str!("../snapshots/tree-search-120x36.txt"),
            "tree-collapsed-80x24" => include_str!("../snapshots/tree-collapsed-80x24.txt"),
            _ => unreachable!(),
        }
    }

    #[test]
    fn snapshots_match_at_three_sizes() {
        for (columns, rows) in [(80, 24), (120, 36), (200, 60)] {
            let mut screen = TreeScreen::new(&sample_report());
            let shown = text(&mut screen, columns, rows);
            assert_eq!(
                shown,
                snapshot(&format!("tree-{columns}x{rows}")),
                "{columns}x{rows}"
            );
        }
    }

    #[test]
    fn filter_search_and_collapse_states_match_their_snapshots() {
        let mut screen = TreeScreen::new(&sample_report());
        let _ = text(&mut screen, 80, 24);
        press(&mut screen, "2");
        assert_eq!(text(&mut screen, 80, 24), snapshot("tree-filtered-80x24"));

        let mut screen = TreeScreen::new(&sample_report());
        let _ = text(&mut screen, 120, 36);
        press(&mut screen, "/wa");
        assert_eq!(text(&mut screen, 120, 36), snapshot("tree-search-120x36"));

        let mut screen = TreeScreen::new(&sample_report());
        let _ = text(&mut screen, 80, 24);
        press(&mut screen, "jjlhh");
        press(&mut screen, "h");
        assert_eq!(text(&mut screen, 80, 24), snapshot("tree-collapsed-80x24"));
    }

    #[test]
    fn every_status_has_a_distinct_one_cell_glyph_and_a_label_on_every_row() {
        let mut seen = Vec::new();
        for status in Status::ALL {
            assert_eq!(width(glyph(status)), 1, "{status:?}");
            assert!(!seen.contains(&glyph(status)), "{status:?}");
            seen.push(glyph(status));
        }
        let mut screen = TreeScreen::new(&sample_report());
        let shown = text(&mut screen, 120, 36);
        for entry in sample_report().entries {
            let name = entry.path.rsplit('/').next().unwrap().to_string();
            let line = shown
                .lines()
                .find(|line| line.contains(&name) && line.contains("[ ]"))
                .unwrap();
            assert!(line.contains(glyph(entry.status)), "{line}");
            assert!(line.contains(entry.status.label()), "{line}");
        }
    }

    #[test]
    fn the_header_counts_every_status_with_its_key_glyph_and_label() {
        let mut screen = TreeScreen::new(&sample_report());
        let shown = text(&mut screen, 200, 60);
        let header: Vec<&str> = shown.lines().take(3).collect();
        let joined = header.join("\n");
        for (number, status, count) in [
            (1, Status::Unchanged, 2),
            (2, Status::UpstreamChanged, 1),
            (3, Status::LocalOverride, 2),
            (4, Status::Conflict, 1),
            (5, Status::UserOnly, 1),
            (6, Status::Missing, 1),
        ] {
            let expected = format!("{number} {} {} {count}", glyph(status), status.label());
            assert!(joined.contains(&expected), "{expected}\n{joined}");
        }
        assert!(joined.contains("3 drifted"), "{joined}");
        assert!(
            joined.contains("update pending review: 4.4.0-1 -> 4.5.0-3"),
            "{joined}"
        );
        let mut narrow = TreeScreen::new(&sample_report());
        let shown = text(&mut narrow, 80, 24);
        assert!(
            shown.lines().nth(2).unwrap().contains("4 ✗ conflict 1"),
            "{shown}"
        );
    }

    #[test]
    fn movement_keys_move_collapse_and_jump() {
        let mut screen = opened();
        assert_eq!(screen.view().selected(), 0);
        press(&mut screen, "j");
        assert_eq!(screen.view().selected(), 1);
        key(&mut screen, KeyCode::Down);
        assert_eq!(screen.view().selected(), 2);
        press(&mut screen, "k");
        key(&mut screen, KeyCode::Up);
        assert_eq!(screen.view().selected(), 0);
        press(&mut screen, "G");
        assert_eq!(screen.view().selected(), screen.view().rows().len() - 1);
        press(&mut screen, "g");
        assert_eq!(screen.view().selected(), 0);
        key(&mut screen, KeyCode::End);
        assert_eq!(selected_path(&screen), ".config/alacritty.toml");
        key(&mut screen, KeyCode::Home);
        assert_eq!(screen.view().selected(), 0);
        key(&mut screen, KeyCode::PageDown);
        assert!(screen.view().selected() > 3);
        key(&mut screen, KeyCode::PageUp);
        assert_eq!(screen.view().selected(), 0);
        let rows = screen.view().rows().len();
        press(&mut screen, "h");
        assert_eq!(screen.view().rows().len(), 1);
        press(&mut screen, "l");
        assert_eq!(screen.view().rows().len(), rows);
        key(&mut screen, KeyCode::Left);
        assert_eq!(screen.view().rows().len(), 1);
        key(&mut screen, KeyCode::Right);
        assert_eq!(screen.view().rows().len(), rows);
        key(&mut screen, KeyCode::Right);
        assert_eq!(screen.view().selected(), 1);
        press(&mut screen, "jh");
        assert_eq!(screen.view().selected(), 1);
    }

    #[test]
    fn enter_opens_a_file_and_toggles_a_directory() {
        let mut screen = opened();
        let rows = screen.view().rows().len();
        assert_eq!(key(&mut screen, KeyCode::Enter), None);
        assert_eq!(screen.view().rows().len(), 1);
        key(&mut screen, KeyCode::Enter);
        assert_eq!(screen.view().rows().len(), rows);
        press(&mut screen, "j");
        assert_eq!(key(&mut screen, KeyCode::Enter), None);
        assert_eq!(screen.view().rows().len(), rows - 1);
        key(&mut screen, KeyCode::Enter);
        assert_eq!(screen.view().rows().len(), rows);
        press(&mut screen, "j");
        assert_eq!(
            key(&mut screen, KeyCode::Enter),
            Some(Action::Open(".config/btop/btop.conf".to_string()))
        );
    }

    #[test]
    fn digits_filter_by_status_zero_clears_and_the_same_digit_toggles() {
        let mut screen = opened();
        for (digit, status) in ['1', '2', '3', '4', '5', '6'].into_iter().zip(Status::ALL) {
            press(&mut screen, &digit.to_string());
            assert_eq!(screen.view().status_filter(), Some(status), "{digit}");
            assert!(screen.view().visible_files() > 0);
        }
        press(&mut screen, "0");
        assert_eq!(screen.view().status_filter(), None);
        assert_eq!(screen.view().visible_files(), 8);
        press(&mut screen, "44");
        assert_eq!(screen.view().status_filter(), None);
        press(&mut screen, "7");
        assert_eq!(screen.view().status_filter(), None);
    }

    #[test]
    fn slash_searches_incrementally_and_enter_keeps_it_while_escape_drops_it() {
        let mut screen = opened();
        press(&mut screen, "/");
        assert!(screen.searching());
        press(&mut screen, "j0q");
        assert_eq!(screen.view().query(), "j0q");
        assert_eq!(screen.view().visible_files(), 0);
        key(&mut screen, KeyCode::Backspace);
        key(&mut screen, KeyCode::Backspace);
        key(&mut screen, KeyCode::Backspace);
        press(&mut screen, "STY");
        assert_eq!(screen.view().visible_files(), 1);
        assert!(text(&mut screen, 120, 36).contains("search: STY_"));
        assert_eq!(key(&mut screen, KeyCode::Enter), None);
        assert!(!screen.searching());
        assert_eq!(screen.view().query(), "STY");
        press(&mut screen, "q");
        assert_eq!(key(&mut screen, KeyCode::Char('q')), Some(Action::Quit));
        press(&mut screen, "/");
        press(&mut screen, "x");
        key(&mut screen, KeyCode::Esc);
        assert!(!screen.searching());
        assert_eq!(screen.view().query(), "");
        assert_eq!(screen.view().visible_files(), 8);
    }

    #[test]
    fn typing_in_search_does_not_trigger_commands_but_arrows_still_move() {
        let mut screen = opened();
        press(&mut screen, "/");
        assert_eq!(key(&mut screen, KeyCode::Char('q')), None);
        assert_eq!(key(&mut screen, KeyCode::Char('o')), None);
        assert_eq!(key(&mut screen, KeyCode::Char('1')), None);
        assert_eq!(screen.view().status_filter(), None);
        key(&mut screen, KeyCode::Backspace);
        key(&mut screen, KeyCode::Backspace);
        key(&mut screen, KeyCode::Backspace);
        key(&mut screen, KeyCode::Down);
        assert_eq!(screen.view().selected(), 1);
        key(&mut screen, KeyCode::Up);
        assert_eq!(screen.view().selected(), 0);
    }

    #[test]
    fn escape_backs_out_one_layer_at_a_time() {
        let mut screen = opened();
        press(&mut screen, "a");
        press(&mut screen, "2");
        press(&mut screen, "/sty");
        key(&mut screen, KeyCode::Enter);
        key(&mut screen, KeyCode::Esc);
        assert_eq!(screen.view().query(), "");
        assert_eq!(screen.view().status_filter(), Some(Status::UpstreamChanged));
        key(&mut screen, KeyCode::Esc);
        assert_eq!(screen.view().status_filter(), None);
        assert!(screen.view().marked_count() > 0);
        key(&mut screen, KeyCode::Esc);
        assert_eq!(screen.view().marked_count(), 0);
        assert_eq!(key(&mut screen, KeyCode::Esc), None);
    }

    #[test]
    fn space_marks_and_a_marks_everything_in_the_filter() {
        let mut screen = opened();
        press(&mut screen, "jj");
        press(&mut screen, " ");
        assert_eq!(screen.view().marked_count(), 1);
        press(&mut screen, " ");
        assert_eq!(screen.view().marked_count(), 0);
        press(&mut screen, "1");
        press(&mut screen, "a");
        assert_eq!(screen.view().marked_count(), 2);
        press(&mut screen, "0");
        assert_eq!(screen.view().marked_count(), 2);
        press(&mut screen, "a");
        assert_eq!(screen.view().marked_count(), 8);
        let shown = text(&mut screen, 120, 36);
        assert!(shown.contains("8 marked"), "{shown}");
        assert!(shown.contains("[x]"), "{shown}");
        press(&mut screen, "a");
        assert_eq!(screen.view().marked_count(), 0);
    }

    #[test]
    fn a_directory_checkbox_shows_partial_marks() {
        let mut screen = opened();
        press(&mut screen, "jjjjjjj");
        assert_eq!(selected_path(&screen), ".config/hypr/input.lua");
        press(&mut screen, " ");
        let shown = text(&mut screen, 120, 36);
        assert!(
            shown
                .lines()
                .any(|line| line.contains("hypr/") && line.contains("[-]")),
            "{shown}"
        );
        assert!(
            shown
                .lines()
                .any(|line| line.contains("btop/") && line.contains("[ ]")),
            "{shown}"
        );
    }

    #[test]
    fn per_file_actions_act_on_the_selection_or_the_marks_and_i_asks_to_ignore() {
        let mut screen = opened();
        press(&mut screen, "jjjjjjj");
        assert_eq!(selected_path(&screen), ".config/hypr/input.lua");
        assert_eq!(
            key(&mut screen, KeyCode::Char('o')),
            Some(Action::Keep(vec![".config/hypr/input.lua".to_string()]))
        );
        assert_eq!(
            key(&mut screen, KeyCode::Char('t')),
            Some(Action::Take(vec![".config/hypr/input.lua".to_string()]))
        );
        assert_eq!(
            key(&mut screen, KeyCode::Char('m')),
            Some(Action::Merge(vec![".config/hypr/input.lua".to_string()]))
        );
        assert_eq!(
            key(&mut screen, KeyCode::Char('i')),
            Some(Action::Ignore(vec![".config/hypr/input.lua".to_string()]))
        );
        press(&mut screen, " ");
        press(&mut screen, "j");
        press(&mut screen, " ");
        let Some(Action::Take(paths)) = key(&mut screen, KeyCode::Char('t')) else {
            panic!("take");
        };
        assert_eq!(paths.len(), 2);
        let mut screen = opened();
        press(&mut screen, "j");
        let Some(Action::Keep(paths)) = key(&mut screen, KeyCode::Char('o')) else {
            panic!("keep");
        };
        assert_eq!(paths, [".config/btop/btop.conf"]);
        press(&mut screen, "/zzz");
        key(&mut screen, KeyCode::Enter);
        assert_eq!(key(&mut screen, KeyCode::Char('o')), None);
        assert_eq!(key(&mut screen, KeyCode::Char('i')), None);
    }

    #[test]
    fn r_asks_to_rescan_and_q_quits_and_unknown_keys_do_nothing() {
        let mut screen = opened();
        assert_eq!(key(&mut screen, KeyCode::Char('r')), Some(Action::Rescan));
        assert_eq!(key(&mut screen, KeyCode::Char('x')), None);
        assert_eq!(key(&mut screen, KeyCode::Backspace), None);
        let mut release = KeyEvent::press(KeyCode::Char('q'));
        release.kind = KeyKind::Release;
        assert_eq!(screen.handle(&Event::Key(release)), None);
        assert_eq!(key(&mut screen, KeyCode::Char('q')), Some(Action::Quit));
    }

    #[test]
    fn clicking_a_row_selects_it_and_a_quick_second_click_opens_it() {
        let mut screen = opened();
        let shown = text(&mut screen, 120, 36);
        let row = row_of(&shown, "input.lua");
        let t0 = Instant::now();
        assert_eq!(screen.handle_at(&click_event(40, row), t0), None);
        assert_eq!(selected_path(&screen), ".config/hypr/input.lua");
        assert_eq!(
            screen.handle_at(&click_event(40, row), t0 + Duration::from_millis(200)),
            Some(Action::Open(".config/hypr/input.lua".to_string()))
        );
        assert_eq!(
            screen.handle_at(&click_event(40, row), t0 + Duration::from_millis(300)),
            None
        );
    }

    #[test]
    fn slow_or_split_clicks_are_not_a_double_click() {
        let mut screen = opened();
        let shown = text(&mut screen, 120, 36);
        let (first, second) = (row_of(&shown, "input.lua"), row_of(&shown, "looknfeel.lua"));
        let t0 = Instant::now();
        screen.handle_at(&click_event(40, first), t0);
        assert_eq!(
            screen.handle_at(&click_event(40, first), t0 + Duration::from_millis(900)),
            None
        );
        screen.handle_at(&click_event(40, first), t0 + Duration::from_secs(5));
        assert_eq!(
            screen.handle_at(&click_event(40, second), t0 + Duration::from_secs(5)),
            None
        );
        assert_eq!(selected_path(&screen), ".config/hypr/looknfeel.lua");
    }

    #[test]
    fn a_second_click_on_a_different_file_after_the_rows_changed_is_not_a_double_click() {
        let t0 = Instant::now();
        let quick = t0 + Duration::from_millis(100);

        let mut screen = opened();
        let shown = text(&mut screen, 120, 36);
        let first = row_of(&shown, "[ ] ? missing");
        screen.handle_at(&click_event(40, first), t0);
        assert_eq!(selected_path(&screen), ".config/btop/btop.conf");
        press(&mut screen, "4");
        let shown = text(&mut screen, 120, 36);
        assert_eq!(row_of(&shown, "[ ] ✗ conflict"), first);
        assert_eq!(screen.handle_at(&click_event(40, first), quick), None);
        assert_eq!(selected_path(&screen), ".config/hypr/monitors.lua");

        let mut screen = opened();
        let shown = text(&mut screen, 120, 36);
        let first = row_of(&shown, "[ ] ? missing");
        screen.handle_at(&click_event(40, first), t0);
        press(&mut screen, "/mon");
        key(&mut screen, KeyCode::Enter);
        let shown = text(&mut screen, 120, 36);
        let moved = row_of(&shown, "[ ] ✗ conflict");
        assert_eq!(moved, first);
        assert_eq!(screen.handle_at(&click_event(40, moved), quick), None);
        assert_eq!(
            screen.handle_at(&click_event(40, moved), quick + Duration::from_millis(100)),
            Some(Action::Open(".config/hypr/monitors.lua".to_string()))
        );
    }

    #[test]
    fn the_same_file_clicked_twice_quickly_still_opens_after_a_rebuild_in_between() {
        let t0 = Instant::now();
        let mut screen = opened();
        let shown = text(&mut screen, 120, 36);
        let row = row_of(&shown, "[ ] ✗ conflict");
        screen.handle_at(&click_event(40, row), t0);
        press(&mut screen, "4");
        let shown = text(&mut screen, 120, 36);
        let row = row_of(&shown, "[ ] ✗ conflict");
        assert_eq!(
            screen.handle_at(&click_event(40, row), t0 + Duration::from_millis(150)),
            Some(Action::Open(".config/hypr/monitors.lua".to_string()))
        );
    }

    #[test]
    fn the_caret_toggles_the_directory_and_the_checkbox_marks() {
        let mut screen = opened();
        let shown = text(&mut screen, 120, 36);
        let hypr = row_of(&shown, "hypr/");
        let rows = screen.view().rows().len();
        let caret = 2 + 2;
        screen.handle_at(&click_event(caret, hypr), Instant::now());
        assert_eq!(screen.view().rows().len(), rows - 3);
        screen.handle_at(&click_event(caret, hypr), Instant::now());
        assert_eq!(screen.view().rows().len(), rows);
        let checkbox = 2 + 2 + 2;
        screen.handle_at(&click_event(checkbox, hypr), Instant::now());
        assert_eq!(screen.view().marked_count(), 3);
        screen.handle_at(&click_event(checkbox + 2, hypr), Instant::now());
        assert_eq!(screen.view().marked_count(), 0);
        let shown = text(&mut screen, 120, 36);
        let file = row_of(&shown, "input.lua");
        screen.handle_at(&click_event(2 + 4 + 4, file), Instant::now());
        assert_eq!(screen.view().marked_count(), 1);
        assert_eq!(selected_path(&screen), ".config/hypr/input.lua");
    }

    #[test]
    fn the_wheel_scrolls_the_list_without_moving_the_selection() {
        let mut screen = TreeScreen::new(&big_report(200));
        let before = text(&mut screen, 80, 24);
        assert!(before.contains("dir000"), "{before}");
        let wheel = |scroll| Event::Mouse(MouseEvent::new(MouseKind::Scroll(scroll), 10, 10));
        for _ in 0..5 {
            screen.handle(&wheel(Scroll::Down));
        }
        let after = text(&mut screen, 80, 24);
        assert!(
            !after
                .lines()
                .nth(5)
                .unwrap()
                .eq(before.lines().nth(5).unwrap()),
            "{after}"
        );
        assert_eq!(screen.view().selected(), 0);
        for _ in 0..10 {
            screen.handle(&wheel(Scroll::Up));
        }
        assert_eq!(text(&mut screen, 80, 24), before);
        for _ in 0..500 {
            screen.handle(&wheel(Scroll::Down));
        }
        let end = text(&mut screen, 80, 24);
        assert!(end.contains("dir119"), "{end}");
    }

    #[test]
    fn clicking_a_header_count_filters_and_clicking_it_again_clears() {
        let mut screen = opened();
        let shown = text(&mut screen, 120, 36);
        let row = row_of(&shown, "4 ✗ conflict 1");
        let column = shown
            .lines()
            .nth(row as usize)
            .unwrap()
            .find("4 ✗")
            .map(|at| {
                shown.lines().nth(row as usize).unwrap()[..at]
                    .chars()
                    .count()
            })
            .unwrap() as u16;
        screen.handle_at(&click_event(column + 2, row), Instant::now());
        assert_eq!(screen.view().status_filter(), Some(Status::Conflict));
        let shown = text(&mut screen, 120, 36);
        assert!(shown.contains("[4 ✗ conflict 1]"), "{shown}");
        assert!(shown.contains("filter: conflict"), "{shown}");
        screen.handle_at(&click_event(column + 2, row), Instant::now());
        assert_eq!(screen.view().status_filter(), None);
    }

    fn variant(action: &Option<Action>) -> Option<String> {
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
        for (columns, rows) in [(80_u16, 24_u16), (120, 36)] {
            for row in 0..rows {
                for column in 0..columns {
                    let mut screen = TreeScreen::new(&sample_report());
                    let _ = text(&mut screen, columns, rows);
                    let found = screen.handle_at(&click_event(column, row), Instant::now());
                    by_mouse.extend(variant(&found));
                }
            }
        }
        let mut by_key = std::collections::BTreeSet::new();
        for command in crate::commands::TREE_COMMANDS {
            for chord in command.chords() {
                for steps in 0..8 {
                    let mut screen = opened();
                    press(&mut screen, &"j".repeat(steps));
                    let found = key(&mut screen, chord.code());
                    by_key.extend(variant(&found));
                }
            }
        }
        for command in crate::commands::GLOBAL_COMMANDS {
            for chord in command.chords() {
                let mut screen = opened();
                by_key.extend(variant(&key(&mut screen, chord.code())));
            }
        }
        assert!(by_mouse.len() >= 4, "{by_mouse:?}");
        let only_mouse: Vec<_> = by_mouse.difference(&by_key).collect();
        assert!(only_mouse.is_empty(), "mouse-only actions: {only_mouse:?}");
    }

    #[test]
    fn clicking_hint_items_runs_their_keys() {
        let mut screen = opened();
        let shown = text(&mut screen, 120, 36);
        let row = row_of(&shown, "[Q] Quit");
        let line = shown.lines().nth(row as usize).unwrap();
        let column_of =
            |needle: &str| line[..line.find(needle).unwrap()].chars().count() as u16 + 1;
        assert_eq!(
            screen.handle_at(&click_event(column_of("[/] Search"), row), Instant::now()),
            None
        );
        assert!(screen.searching());
        key(&mut screen, KeyCode::Esc);
        assert_eq!(
            screen.handle_at(&click_event(column_of("[Q] Quit"), row), Instant::now()),
            Some(Action::Quit)
        );
        let Some(Action::Keep(paths)) =
            screen.handle_at(&click_event(column_of("[o] Keep"), row), Instant::now())
        else {
            panic!("keep");
        };
        assert_eq!(paths.len(), 8);
        let mut sorted = paths.clone();
        sorted.sort();
        assert_eq!(paths, sorted);
        assert_eq!(
            screen.handle_at(&click_event(column_of("[H] History"), row), Instant::now()),
            Some(Action::History)
        );
    }

    #[test]
    fn a_capital_h_asks_for_the_history_and_a_lowercase_h_still_collapses() {
        let mut screen = opened();
        assert_eq!(key(&mut screen, KeyCode::Char('H')), Some(Action::History));
        assert_eq!(key(&mut screen, KeyCode::Char('h')), None);
        assert!(text(&mut screen, 120, 36).contains("[H] History"));
    }

    #[test]
    fn clicks_outside_the_interactive_parts_and_other_buttons_do_nothing() {
        let mut screen = opened();
        let _ = text(&mut screen, 120, 36);
        assert_eq!(screen.handle_at(&click_event(0, 0), Instant::now()), None);
        assert_eq!(
            screen.handle_at(&click_event(119, 35), Instant::now()),
            None
        );
        assert_eq!(screen.handle_at(&click_event(10, 34), Instant::now()), None);
        let right = Event::Mouse(MouseEvent::new(MouseKind::Down(MouseButton::Right), 40, 6));
        assert_eq!(screen.handle(&right), None);
        assert_eq!(screen.view().selected(), 0);
        let mut fresh = TreeScreen::new(&sample_report());
        assert_eq!(fresh.handle(&click_event(40, 6)), None);
    }

    #[test]
    fn row_hits_follow_the_depth() {
        let dir = |depth| TreeRow {
            node: 1,
            depth,
            kind: RowKind::Dir {
                expanded: true,
                files: 1,
                drifted: 0,
            },
            name: "d".to_string(),
            entry: None,
        };
        let file = |depth| TreeRow {
            node: 2,
            depth,
            kind: RowKind::File,
            name: "f".to_string(),
            entry: Some(0),
        };
        assert_eq!(row_hit(&dir(0), 2), Hit::Caret);
        assert_eq!(row_hit(&dir(0), 4), Hit::Checkbox);
        assert_eq!(row_hit(&dir(0), 6), Hit::Checkbox);
        assert_eq!(row_hit(&dir(0), 7), Hit::Body);
        assert_eq!(row_hit(&dir(2), 6), Hit::Caret);
        assert_eq!(row_hit(&dir(2), 3), Hit::Body);
        assert_eq!(row_hit(&file(1), 7), Hit::Body);
        assert_eq!(row_hit(&file(1), 8), Hit::Checkbox);
        assert_eq!(row_hit(&file(1), 10), Hit::Checkbox);
        assert_eq!(row_hit(&file(1), 6), Hit::Body);
        assert_eq!(row_hit(&file(1), 11), Hit::Body);
    }

    #[test]
    fn the_selected_row_stays_on_screen_when_moving_far() {
        let mut screen = TreeScreen::new(&big_report(200));
        let _ = text(&mut screen, 80, 24);
        for _ in 0..60 {
            key(&mut screen, KeyCode::Char('j'));
        }
        let shown = text(&mut screen, 80, 24);
        assert!(shown.lines().any(|line| line.starts_with('>')), "{shown}");
        key(&mut screen, KeyCode::Char('G'));
        let shown = text(&mut screen, 80, 24);
        let marked = shown.lines().find(|line| line.starts_with('>')).unwrap();
        assert!(marked.contains("file"), "{shown}");
        key(&mut screen, KeyCode::Char('g'));
        let shown = text(&mut screen, 80, 24);
        assert!(
            shown
                .lines()
                .any(|line| line.starts_with('>') && line.contains(".config/")),
            "{shown}"
        );
    }

    #[test]
    fn the_summary_describes_the_selection() {
        let mut screen = opened();
        let shown = text(&mut screen, 200, 60);
        assert!(
            shown.contains("2 files") || shown.contains("✗ conflict 1"),
            "{shown}"
        );
        press(&mut screen, "jjjjjjjj");
        let shown = text(&mut screen, 200, 60);
        assert!(
            shown.contains("template: config/hypr/looknfeel.lua"),
            "{shown}"
        );
        assert!(shown.contains("base: reviewed"), "{shown}");
        assert!(shown.contains("badge: merges cleanly"), "{shown}");
        press(&mut screen, "/chromium");
        key(&mut screen, KeyCode::Enter);
        press(&mut screen, "G");
        let shown = text(&mut screen, 200, 60);
        assert!(shown.contains("note: app rewrites this file"), "{shown}");
        assert!(shown.contains("app rewrites this file"), "{shown}");
    }

    #[test]
    fn keystrokes_on_five_thousand_rows_stay_under_the_budget() {
        let mut screen = TreeScreen::new(&big_report(5000));
        let _ = render_once(&mut Driver(&mut screen), 120, 36);
        let took = cpu_cost(|| {
            press(&mut screen, "/file4");
            key(&mut screen, KeyCode::Enter);
            press(&mut screen, "2");
            press(&mut screen, "0");
            key(&mut screen, KeyCode::Esc);
        });
        assert!(took < Duration::from_millis(500), "{took:?}");
    }

    #[test]
    fn a_tiny_terminal_says_so_and_still_quits() {
        let mut screen = TreeScreen::new(&sample_report());
        assert!(text(&mut screen, 10, 4).contains("Too small"));
        assert_eq!(key(&mut screen, KeyCode::Char('q')), Some(Action::Quit));
    }

    #[test]
    fn the_screen_wraps_the_tree_and_quits_on_q() {
        use crate::Screen;
        let mut screen = Screen::with_report(&sample_report());
        let _ = render_once(&mut screen, 120, 36);
        assert_eq!(
            screen.update(&Event::Key(KeyEvent::press(KeyCode::Char('j')))),
            ControlFlow::Continue
        );
        assert_eq!(
            screen.update(&Event::Key(KeyEvent::press(KeyCode::Char('o')))),
            ControlFlow::Continue
        );
        assert_eq!(
            screen.take_action(),
            Some(Action::Keep(vec![".config/btop/btop.conf".to_string()]))
        );
        assert_eq!(screen.take_action(), None);
        assert_eq!(
            screen.update(&Event::Key(KeyEvent::press(KeyCode::Char('q')))),
            ControlFlow::Break
        );
        let mut failed = Screen::failed("boom");
        let (grid, clusters) = render_once(&mut failed, 80, 24);
        let shown = grid.display_text(&clusters);
        assert!(
            shown.contains("scan failed") && shown.contains("boom"),
            "{shown}"
        );
    }

    #[test]
    fn a_notice_is_shown_on_the_info_line_and_cleared_by_the_next_key() {
        let mut screen = opened();
        screen.set_notice("Applied 2 file(s).");
        assert_eq!(screen.notice(), Some("Applied 2 file(s)."));
        assert!(text(&mut screen, 120, 36).contains("Applied 2 file(s).  |  8 of 8 files"));
        let wheel = Event::Mouse(MouseEvent::new(MouseKind::Scroll(Scroll::Down), 5, 5));
        screen.handle(&wheel);
        assert_eq!(screen.notice(), Some("Applied 2 file(s)."));
        press(&mut screen, "j");
        assert_eq!(screen.notice(), None);
        assert!(!text(&mut screen, 120, 36).contains("Applied 2 file(s)."));
    }

    macro_rules! state_snapshots {
        ($($name:literal),* $(,)?) => {
            &[$(($name, include_str!(concat!("../snapshots/", $name, ".txt")))),*]
        };
    }

    const STATE_SNAPSHOTS: &[(&str, &str)] = state_snapshots![
        "tree-state-nodrift-80x24",
        "tree-state-nodrift-120x36",
        "tree-state-nodrift-200x60",
        "tree-state-untracked-80x24",
        "tree-state-untracked-120x36",
        "tree-state-untracked-200x60",
        "tree-state-failed-80x24",
        "tree-state-failed-120x36",
        "tree-state-failed-200x60",
        "tree-state-nomatch-80x24",
        "tree-state-nomatch-120x36",
        "tree-state-nomatch-200x60"
    ];

    fn state_snapshot(name: &str) -> &'static str {
        STATE_SNAPSHOTS
            .iter()
            .find(|(known, _)| *known == name)
            .map(|(_, text)| *text)
            .unwrap_or_else(|| panic!("no snapshot {name}"))
    }

    fn calm_report() -> ScanReport {
        let mut report = sample_report();
        report
            .entries
            .retain(|entry| !drift_core::counts_as_drift(entry.status));
        report
    }

    fn state_screen(state: &str) -> TreeScreen {
        match state {
            "nodrift" => TreeScreen::new(&calm_report()),
            "untracked" => TreeScreen::new(&ScanReport {
                package_version: "4.5.0-3".to_string(),
                reviewed_version: None,
                entries: Vec::new(),
                backups: Vec::new(),
                ignored: 0,
            }),
            "failed" => TreeScreen::failed(
                "/var/lib/pacman/local/omarchy-settings-4.5.0-3/files: Permission denied (os error 13)",
            ),
            "nomatch" => {
                let mut screen = TreeScreen::new(&sample_report());
                let _ = text(&mut screen, 80, 24);
                press(&mut screen, "/zzz");
                screen
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn every_empty_and_error_state_matches_its_snapshot_at_three_sizes() {
        for state in ["nodrift", "untracked", "failed", "nomatch"] {
            for (columns, rows) in [(80, 24), (120, 36), (200, 60)] {
                let mut screen = state_screen(state);
                assert_eq!(
                    text(&mut screen, columns, rows),
                    state_snapshot(&format!("tree-state-{state}-{columns}x{rows}")),
                    "{state} {columns}x{rows}"
                );
            }
        }
    }

    fn last_row(screen: &mut TreeScreen, columns: u16, rows: u16) -> String {
        text(screen, columns, rows)
            .lines()
            .last()
            .unwrap_or_default()
            .trim_end()
            .to_string()
    }

    #[test]
    fn the_no_drift_screen_offers_only_history_and_rescan() {
        let mut screen = state_screen("nodrift");
        assert_eq!(
            last_row(&mut screen, 120, 36),
            "[Q] Quit  [?] Help  [:] Palette  [R] Rescan  [H] History"
        );
        for code in ['o', 't', 'm', 'i', ' ', 'a'] {
            assert_eq!(key(&mut screen, KeyCode::Char(code)), None, "{code:?}");
        }
        for command in crate::commands::TREE_COMMANDS
            .iter()
            .copied()
            .filter(|command| acts_on_rows(*command))
        {
            assert_eq!(screen.run(command), None, "{command:?}");
        }
        assert_eq!(screen.view().marked_count(), 0);
        assert_eq!(key(&mut screen, KeyCode::Enter), None);
        let shown = text(&mut screen, 120, 36);
        let index = screen
            .view()
            .rows()
            .iter()
            .position(|row| row.entry.is_some())
            .unwrap();
        let row = 3 + index as u16;
        assert_eq!(screen.handle(&click_event(40, row)), None, "{shown}");
        assert_eq!(screen.handle(&click_event(40, row)), None, "{shown}");
        assert_eq!(key(&mut screen, KeyCode::Char('H')), Some(Action::History));
        assert_eq!(key(&mut screen, KeyCode::Char('r')), Some(Action::Rescan));
        let shown = text(&mut screen, 120, 36);
        let rescan = row_of(&shown, "[R] Rescan");
        let column = shown
            .lines()
            .nth(rescan as usize)
            .unwrap()
            .find("[R]")
            .unwrap() as u16;
        assert_eq!(
            screen.handle(&click_event(column, rescan)),
            Some(Action::Rescan)
        );
    }

    #[test]
    fn a_status_key_leaves_the_no_drift_screen_and_clearing_brings_it_back() {
        let mut screen = state_screen("nodrift");
        assert!(text(&mut screen, 120, 36).contains("Nothing needs review."));
        press(&mut screen, "1");
        let listed = text(&mut screen, 120, 36);
        assert!(!listed.contains("Nothing needs review."), "{listed}");
        assert!(listed.contains("[o] Keep"), "{listed}");
        assert!(listed.contains("input.lua"), "{listed}");
        press(&mut screen, "1");
        assert!(text(&mut screen, 120, 36).contains("Nothing needs review."));
        press(&mut screen, "/in");
        assert!(text(&mut screen, 120, 36).contains("input.lua"));
    }

    #[test]
    fn a_filter_with_no_files_names_the_filter_and_how_to_clear_it() {
        let mut screen = TreeScreen::new(&calm_report());
        press(&mut screen, "4");
        let shown = text(&mut screen, 120, 36);
        assert!(shown.contains("No files match"), "{shown}");
        assert!(shown.contains("Filter: conflict"), "{shown}");
        assert!(shown.contains("Press Esc to clear"), "{shown}");
        assert!(last_row(&mut screen, 120, 36).contains("[/] Search"));
        key(&mut screen, KeyCode::Esc);
        assert!(text(&mut screen, 120, 36).contains("Nothing needs review."));
    }

    #[test]
    fn a_failed_scan_shows_the_whole_reason_and_offers_rescan_and_history() {
        let message =
            "/var/lib/pacman/local/omarchy-settings-4.5.0-3/files: Permission denied (os error 13)";
        for (columns, rows) in [(40, 14), (80, 24), (200, 60)] {
            let mut screen = TreeScreen::failed(message);
            let shown = text(&mut screen, columns, rows);
            let squeezed: String = shown.split_whitespace().collect();
            let wanted: String = message.split_whitespace().collect();
            assert!(squeezed.contains(&wanted), "{columns}x{rows}\n{shown}");
            assert!(shown.contains("Scan failed"), "{shown}");
            if columns >= 80 {
                assert!(shown.contains("[R] Rescan"), "{shown}");
            }
        }
        let mut screen = TreeScreen::failed(message);
        assert_eq!(key(&mut screen, KeyCode::Char('r')), Some(Action::Rescan));
        assert_eq!(key(&mut screen, KeyCode::Char('H')), Some(Action::History));
        assert_eq!(key(&mut screen, KeyCode::Char('o')), None);
    }

    #[test]
    fn wrapping_keeps_every_character_and_splits_words_longer_than_the_line() {
        let long = format!("{}/{}", "a".repeat(30), "b".repeat(30));
        for columns in [1, 7, 20, 80] {
            let lines = wrap(&format!("see {long} now 界界界 end"), columns);
            for line in &lines {
                assert!(width(line) <= columns.max(2), "{columns}: {line:?}");
            }
            let squeezed: String = lines.concat().split_whitespace().collect();
            assert_eq!(squeezed, format!("see{long}now界界界end"), "{columns}");
        }
        assert_eq!(wrap("", 10), vec![String::new()]);
    }

    #[test]
    fn the_summary_explains_the_status_and_the_badge_of_the_selected_file() {
        let squeeze = |shown: &str| shown.split_whitespace().collect::<Vec<_>>().join(" ");
        let wanted = |status: Status| {
            status
                .explain()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        };
        let mut screen = opened();
        press(&mut screen, "4");
        while screen.view().selected_entry().is_none() {
            press(&mut screen, "j");
        }
        let shown = text(&mut screen, 200, 60);
        let lines: Vec<&str> = shown.lines().collect();
        assert!(
            squeeze(&lines.join(" ")).contains(&wanted(Status::Conflict)),
            "{shown}"
        );
        let mut narrow = TreeScreen::new(&sample_report());
        press(&mut narrow, "4");
        while narrow.view().selected_entry().is_none() {
            press(&mut narrow, "j");
        }
        let shown = text(&mut narrow, 80, 24);
        assert!(
            squeeze(&shown).contains(
                &wanted(Status::Conflict)
                    .split(' ')
                    .take(4)
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
            "{shown}"
        );
    }

    #[test]
    fn hovering_a_status_count_explains_it_on_the_info_line() {
        let mut screen = opened();
        let shown = text(&mut screen, 200, 60);
        let row = row_of(&shown, "4 ✗ conflict");
        let column = shown
            .lines()
            .nth(row as usize)
            .unwrap()
            .find("4 ✗")
            .unwrap() as u16;
        let hover = |column, row| Event::Mouse(MouseEvent::new(MouseKind::Move, column, row));
        assert_eq!(screen.handle(&hover(column, row)), None);
        let shown = text(&mut screen, 200, 60);
        assert!(
            shown.contains(&format!("conflict: {}", Status::Conflict.explain())),
            "{shown}"
        );
        assert_eq!(screen.handle(&hover(0, 40)), None);
        assert!(
            !text(&mut screen, 200, 60).contains(Status::Conflict.explain().trim_end_matches('.'))
        );
    }

    #[test]
    fn a_notice_too_long_for_the_info_line_is_shown_in_full_beside_the_tree() {
        let notice = "io error at /home/user/.local/share/drift/state/backups/resolve-1800000000123/manifest.json: Permission denied (os error 13)";
        let wanted: String = notice.split_whitespace().collect();
        for (columns, rows) in [(80, 24), (120, 36), (200, 60)] {
            let mut screen = TreeScreen::new(&sample_report());
            screen.set_notice(notice);
            let shown = text(&mut screen, columns, rows);
            let rebuilt: String = shown
                .lines()
                .map(|line| line.rsplit('|').next().unwrap_or(line))
                .collect::<String>()
                .split_whitespace()
                .collect();
            assert!(
                shown.contains(notice) || rebuilt.contains(&wanted),
                "{columns}x{rows}\n{shown}"
            );
        }
        let mut calm = state_screen("nodrift");
        calm.set_notice(notice);
        let shown = text(&mut calm, 80, 24);
        let squeezed: String = shown.split_whitespace().collect();
        assert!(
            squeezed.contains("manifest.json:Permissiondenied(oserror13)"),
            "{shown}"
        );
    }
}
