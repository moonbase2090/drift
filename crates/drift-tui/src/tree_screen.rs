use std::time::{Duration, Instant};

use lunatui_core::{clip_display_prefix, width, Attrs, ClusterTable, Grid, Rect, Style};
use lunatui_term::{Event, KeyCode, KeyKind, MouseButton, MouseKind, Scroll};
use lunatui_theme::{Theme, Token, Variant};
use lunatui_widgets::RenderCx;

use drift_core::{RowKind, ScanReport, Status, TreeRow, TreeView};

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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Normal,
    Search,
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
    mode: Mode,
    scroll: usize,
    follow: bool,
    area: Option<Rect>,
    last_click: Option<(usize, Instant)>,
}

const HINTS: &[(&str, KeyCode)] = &[
    ("[Q] Quit", KeyCode::Char('q')),
    ("[/] Search", KeyCode::Char('/')),
    ("[1-6] Filter", KeyCode::Char('1')),
    ("[0] Clear", KeyCode::Char('0')),
    ("[Space] Mark", KeyCode::Char(' ')),
    ("[Enter] Open", KeyCode::Enter),
    ("[o] Keep", KeyCode::Char('o')),
    ("[t] Take", KeyCode::Char('t')),
    ("[m] Merge", KeyCode::Char('m')),
    ("[r] Rescan", KeyCode::Char('r')),
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
            mode: Mode::Normal,
            scroll: 0,
            follow: true,
            area: None,
            last_click: None,
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
        match code {
            KeyCode::Char('q') => return Some(Action::Quit),
            KeyCode::Char('r') => return Some(Action::Rescan),
            KeyCode::Char('j') | KeyCode::Down => self.move_by(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_by(-1),
            KeyCode::Char('h') | KeyCode::Left => {
                self.view.collapse_or_parent();
                self.follow = true;
            }
            KeyCode::Char('l') | KeyCode::Right => {
                self.view.expand_or_child();
                self.follow = true;
            }
            KeyCode::Char('g') | KeyCode::Home => {
                self.view.top();
                self.follow = true;
            }
            KeyCode::Char('G') | KeyCode::End => {
                self.view.bottom();
                self.follow = true;
            }
            KeyCode::PageDown => self.move_by(self.page()),
            KeyCode::PageUp => self.move_by(-self.page()),
            KeyCode::Enter => return self.open(),
            KeyCode::Char('/') => self.mode = Mode::Search,
            KeyCode::Char(digit @ '1'..='6') => {
                let status = Status::ALL[usize::from(digit as u8 - b'1')];
                self.filter(status);
            }
            KeyCode::Char('0') => self.view.set_status_filter(None),
            KeyCode::Char(' ') => self.view.toggle_mark(self.view.selected()),
            KeyCode::Char('a') => self.view.mark_all_in_view(),
            KeyCode::Char('o') => return self.act(Action::Keep),
            KeyCode::Char('t') => return self.act(Action::Take),
            KeyCode::Char('m') => return self.act(Action::Merge),
            KeyCode::Char('i') => return self.act(Action::Ignore),
            KeyCode::Esc => self.back(),
            _ => {}
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
        if !contains(found.tree, column, row) {
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
        for (label, code) in HINTS {
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
        if let Some(message) = &self.error {
            title = format!(" Drift  scan failed: {message}");
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
        if let Some(rect) = found.divider {
            let style = dim;
            if rect.width == 1 {
                cx.fill(rect, style, '|');
            } else {
                cx.fill(rect, style, '-');
            }
        }
        if let Some(rect) = found.summary {
            self.draw_summary(&mut cx, rect, base, dim, bold);
        }
        for (rect, _, label) in &found.hints {
            put(&mut cx, *rect, label, dim);
        }
        let _ = found.hint;
    }

    fn info_text(&self) -> String {
        let mut parts = Vec::new();
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
        let rows = self.view.rows();
        if rows.is_empty() {
            let text = if self.error.is_some() {
                "Nothing was scanned."
            } else if self.view.entries().is_empty() {
                "No tracked files."
            } else {
                "No files match the search or filter."
            };
            put(
                cx,
                Rect::new(
                    found.tree.x + 1,
                    found.tree.y,
                    found.tree.width.saturating_sub(1),
                    1,
                ),
                text,
                dim,
            );
            return;
        }
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
        base: Style,
        dim: Style,
        bold: Style,
    ) {
        let mut lines: Vec<(String, Style)> = Vec::new();
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
            "tree-empty-80x24" => include_str!("../snapshots/tree-empty-80x24.txt"),
            "tree-failed-80x24" => include_str!("../snapshots/tree-failed-80x24.txt"),
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
    fn filter_search_collapse_empty_and_failed_states_match_their_snapshots() {
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

        let mut screen = TreeScreen::new(&ScanReport {
            package_version: "4.5.0-3".to_string(),
            reviewed_version: None,
            entries: Vec::new(),
            backups: Vec::new(),
            ignored: 0,
        });
        assert_eq!(text(&mut screen, 80, 24), snapshot("tree-empty-80x24"));

        let mut screen = TreeScreen::failed("no package database");
        assert_eq!(text(&mut screen, 80, 24), snapshot("tree-failed-80x24"));
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
    fn frames_with_five_thousand_rows_stay_under_the_budget() {
        for (columns, rows) in [(80, 24), (200, 60)] {
            let mut screen = TreeScreen::new(&big_report(5000));
            for _ in 0..3 {
                let _ = render_once(&mut Driver(&mut screen), columns, rows);
            }
            let mut samples = Vec::new();
            for step in 0..200 {
                match step % 4 {
                    0 => {
                        key(&mut screen, KeyCode::PageDown);
                    }
                    1 => {
                        key(&mut screen, KeyCode::Down);
                    }
                    2 => {
                        screen.handle(&Event::Mouse(MouseEvent::new(
                            MouseKind::Scroll(Scroll::Down),
                            5,
                            5,
                        )));
                    }
                    _ => {
                        key(&mut screen, KeyCode::Char(' '));
                    }
                }
                let started = Instant::now();
                let _ = render_once(&mut Driver(&mut screen), columns, rows);
                samples.push(started.elapsed());
            }
            samples.sort();
            let p99 = samples[samples.len() * 99 / 100];
            let median = samples[samples.len() / 2];
            assert!(
                p99 < Duration::from_millis(8),
                "{columns}x{rows} p99 {p99:?} median {median:?}"
            );
        }
    }

    #[test]
    fn keystrokes_on_five_thousand_rows_stay_under_the_budget() {
        let mut screen = TreeScreen::new(&big_report(5000));
        let _ = render_once(&mut Driver(&mut screen), 120, 36);
        let started = Instant::now();
        press(&mut screen, "/file4");
        key(&mut screen, KeyCode::Enter);
        press(&mut screen, "2");
        press(&mut screen, "0");
        key(&mut screen, KeyCode::Esc);
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "{:?}",
            started.elapsed()
        );
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
        assert!(grid.display_text(&clusters).contains("scan failed: boom"));
    }
}
