use std::cell::Cell;
use std::rc::Rc;

use lunatui::{run, App, ControlFlow, RunOptions};
use lunatui_core::{clip_display_prefix, width, Attrs, ClusterTable, Grid, Rect, Style};
use lunatui_term::{Event, KeyCode, KeyKind, MouseButton, MouseKind, Scroll};
use lunatui_theme::Token;
use lunatui_widgets::RenderCx;

use drift_core::{Action, PlanView};

const MAX_WIDTH: u16 = 100;
const MAX_HEIGHT: u16 = 36;
const MIN_WIDTH: u16 = 30;
const MIN_HEIGHT: u16 = 9;
const WHEEL_STEP: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Confirm,
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Cancel,
    Confirm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Head,
    Backup,
    Diff,
}

struct Line {
    file: usize,
    kind: Kind,
    text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Layout {
    dialog: Rect,
    header: Rect,
    list: Rect,
    prune: Option<Rect>,
    hint: Rect,
    cancel: Rect,
    confirm: Rect,
}

pub struct PlanDialog {
    view: PlanView,
    focus: Focus,
    selected: usize,
    expanded: Vec<bool>,
    scroll: usize,
    follow: bool,
    decision: Option<Decision>,
    area: Option<Rect>,
}

impl PlanDialog {
    pub fn new(view: PlanView) -> Self {
        let expanded = vec![false; view.files.len()];
        Self {
            view,
            focus: Focus::Cancel,
            selected: 0,
            expanded,
            scroll: 0,
            follow: true,
            decision: None,
            area: None,
        }
    }

    pub fn view(&self) -> &PlanView {
        &self.view
    }

    pub fn focus(&self) -> Focus {
        self.focus
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn is_expanded(&self, index: usize) -> bool {
        self.expanded.get(index).copied().unwrap_or(false)
    }

    pub fn decision(&self) -> Option<Decision> {
        self.decision
    }

    pub fn confirm_bounds(&self, area: Rect) -> Option<Rect> {
        layout(area, &self.view).map(|found| found.confirm)
    }

    pub fn cancel_bounds(&self, area: Rect) -> Option<Rect> {
        layout(area, &self.view).map(|found| found.cancel)
    }

    pub fn handle(&mut self, event: &Event) -> Option<Decision> {
        if self.decision.is_some() {
            return self.decision;
        }
        match event {
            Event::Key(key) if key.kind == KeyKind::Press => self.key(key.code),
            Event::Mouse(mouse) => self.mouse(mouse.kind, mouse.column, mouse.row),
            _ => {}
        }
        self.decision
    }

    fn key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('y' | 'Y') => self.decision = Some(Decision::Confirm),
            KeyCode::Char('n' | 'N' | 'q') | KeyCode::Esc => self.decision = Some(Decision::Cancel),
            KeyCode::Tab | KeyCode::Left | KeyCode::Right | KeyCode::Char('h' | 'l') => {
                self.focus = match self.focus {
                    Focus::Cancel => Focus::Confirm,
                    Focus::Confirm => Focus::Cancel,
                };
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                self.decision = Some(match self.focus {
                    Focus::Cancel => Decision::Cancel,
                    Focus::Confirm => Decision::Confirm,
                });
            }
            KeyCode::Up | KeyCode::Char('k') => self.select(self.selected.saturating_sub(1)),
            KeyCode::Down | KeyCode::Char('j') => self.select(self.selected + 1),
            KeyCode::Home => self.select(0),
            KeyCode::End => self.select(usize::MAX),
            KeyCode::Char('d' | 'D') => self.toggle(self.selected),
            _ => {}
        }
    }

    fn select(&mut self, index: usize) {
        let last = self.view.files.len().saturating_sub(1);
        self.selected = index.min(last);
        self.follow = true;
    }

    fn toggle(&mut self, index: usize) {
        if let Some(open) = self.expanded.get_mut(index) {
            *open = !*open;
            self.follow = true;
        }
    }

    fn mouse(&mut self, kind: MouseKind, column: u16, row: u16) {
        let Some(found) = self.area.and_then(|area| layout(area, &self.view)) else {
            return;
        };
        match kind {
            MouseKind::Down(MouseButton::Left) => {
                if contains(found.confirm, column, row) {
                    self.decision = Some(Decision::Confirm);
                } else if contains(found.cancel, column, row) {
                    self.decision = Some(Decision::Cancel);
                } else if contains(found.list, column, row) {
                    let offset = usize::from(row - found.list.y);
                    let lines = self.lines();
                    if let Some(line) = lines.get(self.scroll + offset) {
                        let file = line.file;
                        self.selected = file;
                        self.toggle(file);
                    }
                }
            }
            MouseKind::Scroll(Scroll::Down) => {
                self.scroll = self.scroll.saturating_add(WHEEL_STEP);
                self.follow = false;
            }
            MouseKind::Scroll(Scroll::Up) => {
                self.scroll = self.scroll.saturating_sub(WHEEL_STEP);
                self.follow = false;
            }
            _ => {}
        }
    }

    fn lines(&self) -> Vec<Line> {
        let mut out = Vec::new();
        for (index, file) in self.view.files.iter().enumerate() {
            let marker = if index == self.selected { '>' } else { ' ' };
            let open = if self.is_expanded(index) { '-' } else { '+' };
            out.push(Line {
                file: index,
                kind: Kind::Head,
                text: format!(
                    "{marker} [{open}] {:<13} {}  ({})",
                    file.action.label(),
                    file.path,
                    file.summary.label()
                ),
            });
            out.push(Line {
                file: index,
                kind: Kind::Backup,
                text: format!("        backup: {}", file.backup),
            });
            if self.is_expanded(index) {
                if file.action == Action::KeepMine && !file.diff.is_empty() {
                    out.push(Line {
                        file: index,
                        kind: Kind::Diff,
                        text: "        your file is kept; upstream differs like this:".to_string(),
                    });
                }
                for line in &file.diff {
                    out.push(Line {
                        file: index,
                        kind: Kind::Diff,
                        text: format!("        {line}"),
                    });
                }
                if file.diff.is_empty() {
                    out.push(Line {
                        file: index,
                        kind: Kind::Diff,
                        text: format!("        ({})", file.summary.label()),
                    });
                }
                if file.diff_truncated {
                    out.push(Line {
                        file: index,
                        kind: Kind::Diff,
                        text: "        ... diff cut".to_string(),
                    });
                }
            }
        }
        out
    }

    fn settle_scroll(&mut self, lines: &[Line], rows: usize) {
        if rows == 0 {
            self.scroll = 0;
            return;
        }
        if self.follow {
            if let Some(first) = lines
                .iter()
                .position(|line| line.file == self.selected && line.kind == Kind::Head)
            {
                if first < self.scroll {
                    self.scroll = first;
                } else if first >= self.scroll + rows {
                    self.scroll = first + 1 - rows;
                }
            }
        }
        self.scroll = self.scroll.min(lines.len().saturating_sub(rows));
    }

    pub fn render(&mut self, grid: &mut Grid, area: Rect, clusters: &mut ClusterTable) {
        self.area = Some(area);
        let theme = crate::diff_palette::active_theme();
        let mut cx = RenderCx::new(grid, clusters, &theme);
        let base = Style::new()
            .fg(cx.token(Token::TextPrimary))
            .bg(cx.token(Token::Surface0));
        cx.fill(area, base, ' ');
        let Some(found) = layout(area, &self.view) else {
            cx.put_str(
                area.x,
                area.y,
                clip_display_prefix("Too small: y / n", area.width),
                base,
            );
            return;
        };
        let panel = Style::new()
            .fg(cx.token(Token::TextPrimary))
            .bg(cx.token(Token::Surface1));
        let muted = panel.with_attrs(Attrs::new().set_dim(true));
        let border = panel.fg(cx.token(Token::FocusRing));
        let title_style = panel.fg(cx.token(Token::Accent));
        let warn = panel.fg(cx.token(Token::Warn));
        cx.fill(found.dialog, panel, ' ');
        draw_border(
            &mut cx,
            found.dialog,
            &format!(" Apply plan {} ", self.view.id),
            border,
            title_style,
        );

        let writes = self.view.writes();
        let header = format!(
            "{} - package {} - {} of {} files will be written",
            shown_time(&self.view.time),
            self.view.package_version,
            writes,
            self.view.files.len()
        );
        put(&mut cx, found.header, &header, muted);

        let lines = self.lines();
        self.settle_scroll(&lines, usize::from(found.list.height));
        let selected_style = panel.with_attrs(Attrs::new().set_bold(true));
        for (row, line) in lines
            .iter()
            .skip(self.scroll)
            .take(usize::from(found.list.height))
            .enumerate()
        {
            let rect = Rect::new(found.list.x, found.list.y + row as u16, found.list.width, 1);
            let style = match line.kind {
                Kind::Head if line.file == self.selected => selected_style,
                Kind::Head => panel,
                Kind::Backup => muted,
                Kind::Diff => diff_style(&cx, panel, &line.text),
            };
            if line.kind == Kind::Head && line.file == self.selected {
                cx.fill(rect, style, ' ');
            }
            if line.kind == Kind::Backup {
                put(&mut cx, rect, &keep_tail(&line.text, rect.width), style);
            } else {
                put(&mut cx, rect, &line.text, style);
            }
        }
        if let Some(rect) = found.prune {
            let text = format!(
                "{} old transaction(s) will be removed: {}",
                self.view.prune.len(),
                self.view.prune.join(", ")
            );
            put(&mut cx, rect, &text, warn);
        }
        put(
            &mut cx,
            found.hint,
            "d expand diff  up/down select  Tab switch  y confirm  n or Esc cancel",
            muted,
        );
        let focused = panel
            .bg(cx.token(Token::Accent))
            .fg(cx.token(Token::Surface0))
            .with_attrs(Attrs::new().set_bold(true));
        let (cancel_style, confirm_style) = match self.focus {
            Focus::Cancel => (focused, panel),
            Focus::Confirm => (panel, focused),
        };
        put(
            &mut cx,
            found.cancel,
            &button("Cancel", self.focus == Focus::Cancel),
            cancel_style,
        );
        put(
            &mut cx,
            found.confirm,
            &button("Confirm", self.focus == Focus::Confirm),
            confirm_style,
        );
    }
}

fn diff_style(cx: &RenderCx<'_>, panel: Style, text: &str) -> Style {
    match text.trim_start().chars().next() {
        Some('+') => panel.fg(cx.token(Token::Ok)),
        Some('-') => panel.fg(cx.token(Token::Fail)),
        Some('@') => panel.fg(cx.token(Token::Accent)),
        _ => panel.with_attrs(Attrs::new().set_dim(true)),
    }
}

fn keep_tail(text: &str, columns: u16) -> String {
    let limit = usize::from(columns);
    if width(text) <= limit {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut start = 0;
    while start < chars.len() {
        let tail: String = chars[start..].iter().collect();
        let candidate = format!("{}...{tail}", prefix_of(text));
        if width(&candidate) <= limit {
            return candidate;
        }
        start += 1;
    }
    String::new()
}

fn prefix_of(text: &str) -> &str {
    let indent = text.len() - text.trim_start().len();
    let label_end = text.find(':').map_or(indent, |at| at + 2);
    &text[..label_end.min(text.len())]
}

fn button(label: &str, focused: bool) -> String {
    if focused {
        format!("[>{label}<]")
    } else {
        format!("[ {label} ]")
    }
}

fn put(cx: &mut RenderCx<'_>, rect: Rect, text: &str, style: Style) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    cx.put_str(rect.x, rect.y, clip_display_prefix(text, rect.width), style);
}

fn draw_border(cx: &mut RenderCx<'_>, rect: Rect, title: &str, border: Style, title_style: Style) {
    let right = rect.x + rect.width - 1;
    let bottom = rect.y + rect.height - 1;
    let inner_width = rect.width.saturating_sub(2);
    let inner_height = rect.height.saturating_sub(2);
    cx.fill(Rect::new(rect.x + 1, rect.y, inner_width, 1), border, '-');
    cx.fill(Rect::new(rect.x + 1, bottom, inner_width, 1), border, '-');
    cx.fill(Rect::new(rect.x, rect.y + 1, 1, inner_height), border, '|');
    cx.fill(Rect::new(right, rect.y + 1, 1, inner_height), border, '|');
    for (x, y) in [
        (rect.x, rect.y),
        (right, rect.y),
        (rect.x, bottom),
        (right, bottom),
    ] {
        cx.put_str(x, y, "+", border);
    }
    let room = rect.width.saturating_sub(4);
    cx.put_str(
        rect.x + 2,
        rect.y,
        clip_display_prefix(title, room),
        title_style,
    );
}

fn contains(area: Rect, column: u16, row: u16) -> bool {
    column >= area.x
        && row >= area.y
        && column < area.x.saturating_add(area.width)
        && row < area.y.saturating_add(area.height)
}

fn button_width(label: &str) -> u16 {
    u16::try_from(width(&button(label, false))).unwrap_or(u16::MAX)
}

fn layout(area: Rect, view: &PlanView) -> Option<Layout> {
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        return None;
    }
    let dialog_width = area.width.min(MAX_WIDTH);
    let dialog_height = area.height.min(MAX_HEIGHT);
    let dialog = Rect::new(
        area.x + (area.width - dialog_width) / 2,
        area.y + (area.height - dialog_height) / 2,
        dialog_width,
        dialog_height,
    );
    let inner = Rect::new(
        dialog.x + 2,
        dialog.y + 1,
        dialog.width - 4,
        dialog.height - 2,
    );
    let has_prune = !view.prune.is_empty();
    let fixed = 1 + 1 + u16::from(has_prune) + 1 + 1;
    let list_height = inner.height.saturating_sub(fixed);
    let header = Rect::new(inner.x, inner.y, inner.width, 1);
    let list = Rect::new(inner.x, inner.y + 2, inner.width, list_height);
    let mut row = list.y + list_height;
    let prune = has_prune.then(|| {
        let rect = Rect::new(inner.x, row, inner.width, 1);
        row += 1;
        rect
    });
    let hint = Rect::new(inner.x, row, inner.width, 1);
    let buttons_y = row + 1;
    let confirm_width = button_width("Confirm");
    let cancel_width = button_width("Cancel");
    let confirm = Rect::new(
        inner.x + inner.width - confirm_width,
        buttons_y,
        confirm_width,
        1,
    );
    let cancel = Rect::new(confirm.x - 2 - cancel_width, buttons_y, cancel_width, 1);
    Some(Layout {
        dialog,
        header,
        list,
        prune,
        hint,
        cancel,
        confirm,
    })
}

struct DialogApp {
    dialog: PlanDialog,
    outcome: Rc<Cell<Option<Decision>>>,
}

impl App for DialogApp {
    fn update(&mut self, event: &Event) -> ControlFlow {
        match self.dialog.handle(event) {
            Some(decision) => {
                self.outcome.set(Some(decision));
                ControlFlow::Break
            }
            None => ControlFlow::Continue,
        }
    }

    fn render(&mut self, grid: &mut Grid, area: Rect, clusters: &mut ClusterTable) {
        self.dialog.render(grid, area, clusters);
    }
}

pub fn review_plan(view: PlanView) -> std::io::Result<Decision> {
    let (columns, rows) = crate::screen::terminal_dimensions();
    let outcome = Rc::new(Cell::new(None));
    let app = DialogApp {
        dialog: PlanDialog::new(view),
        outcome: Rc::clone(&outcome),
    };
    run(app, RunOptions::new(columns, rows).pixels(false))?;
    Ok(outcome.get().unwrap_or(Decision::Cancel))
}

pub fn apply_decision(
    decision: Decision,
    roots: &drift_apply::Roots,
    request: &drift_apply::Request<'_>,
) -> Result<Option<drift_apply::ApplyReport>, drift_apply::ApplyError> {
    match decision {
        Decision::Cancel => Ok(None),
        Decision::Confirm => drift_apply::apply(roots, request).map(Some),
    }
}

fn shown_time(stored: &str) -> String {
    utc_unix(stored)
        .and_then(local_stamp)
        .unwrap_or_else(|| stored.to_string())
}

fn utc_unix(text: &str) -> Option<i64> {
    let bytes = text.as_bytes();
    if bytes.len() != 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes[19] != b'Z'
    {
        return None;
    }
    let year = number(&bytes[0..4])?;
    let month = number(&bytes[5..7])?;
    let day = number(&bytes[8..10])?;
    let hour = number(&bytes[11..13])?;
    let minute = number(&bytes[14..16])?;
    let second = number(&bytes[17..19])?;
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }
    let days = days_from_civil(year, month, day);
    Some(days * 86_400 + hour * 3_600 + minute * 60 + second)
}

fn number(bytes: &[u8]) -> Option<i64> {
    let mut value = 0i64;
    for byte in bytes {
        if !byte.is_ascii_digit() {
            return None;
        }
        value = value * 10 + i64::from(byte - b'0');
    }
    Some(value)
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = (year - era * 400) as u64;
    let month_index = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * month_index + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year as u64;
    era * 146_097 + day_of_era as i64 - 719_468
}

fn local_stamp(secs: i64) -> Option<String> {
    let mut broken: libc::tm = unsafe { std::mem::zeroed() };
    let clock = secs as libc::time_t;
    if unsafe { libc::localtime_r(&clock, &mut broken) }.is_null() {
        return None;
    }
    let mut buf = [0u8; 48];
    let wrote = unsafe {
        libc::strftime(
            buf.as_mut_ptr().cast(),
            buf.len(),
            c"%Y-%m-%d %H:%M:%S %z".as_ptr(),
            &broken,
        )
    };
    if wrote == 0 {
        return None;
    }
    std::str::from_utf8(&buf[..wrote]).ok().map(str::to_string)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use drift_apply::{Payload, Request, Roots};
    use drift_core::{Action, FileContents, Plan, PlannedFile};
    use drift_fixtures::{cpu_cost, hash_tree, RootBuilder};
    use lunatui::render_once;
    use lunatui_term::{KeyEvent, MouseEvent};
    use sha2::{Digest, Sha256};

    use super::*;

    const STATE: &str = "~/.local/state/drift";
    const SIZES: [(u16, u16, &str); 3] =
        [(80, 24, "80x24"), (120, 36, "120x36"), (200, 60, "200x60")];

    fn sha(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    fn sample_files() -> Vec<(
        &'static str,
        Action,
        Option<&'static str>,
        Option<&'static str>,
    )> {
        vec![
            (
                "config/hypr/looknfeel.lua",
                Action::Merge,
                Some("gaps = 5\nrounding = 2\nblur = true\nshadow = false\n"),
                Some("gaps = 8\nrounding = 2\nblur = true\nshadow = true\nanimations = on\n"),
            ),
            (
                "config/waybar/style.css",
                Action::TakeUpstream,
                None,
                Some("* {\n  font-size: 13px;\n}\n"),
            ),
            (
                "config/hypr/monitors.lua",
                Action::KeepMine,
                Some("scale = 2\n"),
                Some("scale = 2\n"),
            ),
        ]
    }

    fn sample_view(prune: &[&str]) -> PlanView {
        let files: Vec<PlannedFile> = sample_files()
            .into_iter()
            .map(|(path, action, before, after)| {
                PlannedFile::new(
                    path,
                    action,
                    before.map(|text| sha(text.as_bytes())).as_deref(),
                    after.map(|text| sha(text.as_bytes())).as_deref(),
                    Some(0o644),
                    None,
                )
                .unwrap()
            })
            .collect();
        let plan = Plan::new("txn-42", "2026-06-01T09:30:00Z", "4.5.0-3", files, prune).unwrap();
        let contents: Vec<FileContents> = sample_files()
            .into_iter()
            .map(|(path, _, before, after)| FileContents {
                path: path.to_string(),
                before: before.map(|text| text.as_bytes().to_vec()),
                after: after.map(|text| text.as_bytes().to_vec()),
            })
            .collect();
        PlanView::build(&plan, STATE, &contents)
    }

    fn many_files_view(count: usize) -> PlanView {
        let files: Vec<PlannedFile> = (0..count)
            .map(|n| {
                PlannedFile::new(
                    &format!("config/dir/file{n:02}.conf"),
                    Action::TakeUpstream,
                    Some(&sha(b"old\n")),
                    Some(&sha(b"new\n")),
                    None,
                    None,
                )
                .unwrap()
            })
            .collect();
        let plan = Plan::new("txn-9", "2026-06-01T09:30:00Z", "4.5.0-3", files, &[]).unwrap();
        let contents: Vec<FileContents> = (0..count)
            .map(|n| FileContents {
                path: format!("config/dir/file{n:02}.conf"),
                before: Some(b"old\n".to_vec()),
                after: Some(b"new\n".to_vec()),
            })
            .collect();
        PlanView::build(&plan, STATE, &contents)
    }

    fn text(dialog: &mut PlanDialog, columns: u16, rows: u16) -> String {
        with_zone("UTC0", || paint(dialog, columns, rows))
    }

    fn paint(dialog: &mut PlanDialog, columns: u16, rows: u16) -> String {
        let (grid, clusters) = render_once(&mut Wrapper(dialog), columns, rows);
        grid.display_text(&clusters)
    }

    fn with_zone<T>(zone: &str, body: impl FnOnce() -> T) -> T {
        unsafe extern "C" {
            fn tzset();
        }
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = LOCK.lock().unwrap_or_else(|poison| poison.into_inner());
        let previous = std::env::var_os("TZ");
        std::env::set_var("TZ", zone);
        unsafe { tzset() };
        let value = body();
        match previous {
            Some(zone) => std::env::set_var("TZ", zone),
            None => std::env::remove_var("TZ"),
        }
        unsafe { tzset() };
        value
    }

    struct Wrapper<'a>(&'a mut PlanDialog);

    impl App for Wrapper<'_> {
        fn update(&mut self, event: &Event) -> ControlFlow {
            let _ = self.0.handle(event);
            ControlFlow::Continue
        }

        fn render(&mut self, grid: &mut Grid, area: Rect, clusters: &mut ClusterTable) {
            self.0.render(grid, area, clusters);
        }
    }

    fn key(code: KeyCode) -> Event {
        Event::Key(KeyEvent::press(code))
    }

    fn character(c: char) -> Event {
        key(KeyCode::Char(c))
    }

    fn click(column: u16, row: u16) -> Event {
        Event::Mouse(MouseEvent::new(
            MouseKind::Down(MouseButton::Left),
            column,
            row,
        ))
    }

    fn centre(rect: Rect) -> (u16, u16) {
        (rect.x + rect.width / 2, rect.y)
    }

    fn opened(prune: &[&str]) -> PlanDialog {
        let mut dialog = PlanDialog::new(sample_view(prune));
        let _ = text(&mut dialog, 120, 36);
        dialog
    }

    fn snapshot(name: &str) -> &'static str {
        match name {
            "collapsed-80x24" => include_str!("../snapshots/plan-collapsed-80x24.txt"),
            "collapsed-120x36" => include_str!("../snapshots/plan-collapsed-120x36.txt"),
            "collapsed-200x60" => include_str!("../snapshots/plan-collapsed-200x60.txt"),
            "expanded-80x24" => include_str!("../snapshots/plan-expanded-80x24.txt"),
            "expanded-120x36" => include_str!("../snapshots/plan-expanded-120x36.txt"),
            "small-20x6" => include_str!("../snapshots/plan-small-20x6.txt"),
            _ => unreachable!(),
        }
    }

    #[test]
    fn snapshots_match_at_three_sizes() {
        for (columns, rows, label) in SIZES {
            let mut dialog = PlanDialog::new(sample_view(&["old-1", "old-2"]));
            let shown = text(&mut dialog, columns, rows);
            assert_eq!(shown, snapshot(&format!("collapsed-{label}")), "{label}");
        }
    }

    #[test]
    fn expanded_diffs_and_the_confirm_focus_match_their_snapshots() {
        for (columns, rows, label) in [(80, 24, "80x24"), (120, 36, "120x36")] {
            let mut dialog = PlanDialog::new(sample_view(&["old-1", "old-2"]));
            let _ = text(&mut dialog, columns, rows);
            dialog.handle(&character('d'));
            dialog.handle(&character('j'));
            dialog.handle(&character('d'));
            dialog.handle(&key(KeyCode::Tab));
            let shown = text(&mut dialog, columns, rows);
            assert_eq!(shown, snapshot(&format!("expanded-{label}")), "{label}");
        }
    }

    #[test]
    fn a_terminal_too_small_for_the_dialog_says_so_and_still_decides() {
        let mut dialog = PlanDialog::new(sample_view(&[]));
        let shown = text(&mut dialog, 20, 6);
        assert_eq!(shown, snapshot("small-20x6"));
        assert_eq!(dialog.handle(&character('n')), Some(Decision::Cancel));
    }

    #[test]
    fn the_dialog_lists_every_file_action_backup_and_summary() {
        let mut dialog = PlanDialog::new(sample_view(&["old-1"]));
        let shown = text(&mut dialog, 120, 36);
        for needle in [
            "Apply plan txn-42",
            "2 of 3 files will be written",
            "merge",
            "config/hypr/looknfeel.lua",
            "(+3 -2)",
            "backup: ~/.local/state/drift/backups/txn-42/config/hypr/looknfeel.lua",
            "take upstream",
            "config/waybar/style.css",
            "(new file, 3 lines)",
            "backup: none (new file)",
            "keep mine",
            "(not written)",
            "backup: none (nothing is written)",
            "1 old transaction(s) will be removed: old-1",
        ] {
            assert!(shown.contains(needle), "{needle}\n{shown}");
        }
    }

    #[test]
    fn focus_starts_on_cancel_and_the_focused_button_is_marked_in_text() {
        let mut dialog = opened(&[]);
        assert_eq!(dialog.focus(), Focus::Cancel);
        let shown = text(&mut dialog, 120, 36);
        assert!(shown.contains("[>Cancel<]"), "{shown}");
        assert!(shown.contains("[ Confirm ]"), "{shown}");
        dialog.handle(&key(KeyCode::Tab));
        assert_eq!(dialog.focus(), Focus::Confirm);
        let shown = text(&mut dialog, 120, 36);
        assert!(shown.contains("[ Cancel ]"), "{shown}");
        assert!(shown.contains("[>Confirm<]"), "{shown}");
    }

    #[test]
    fn enter_on_the_default_focus_cancels() {
        for code in [KeyCode::Enter, KeyCode::Char(' ')] {
            let mut dialog = opened(&[]);
            assert_eq!(dialog.handle(&key(code)), Some(Decision::Cancel));
        }
    }

    #[test]
    fn y_or_moving_focus_then_enter_confirms() {
        let mut dialog = opened(&[]);
        assert_eq!(dialog.handle(&character('y')), Some(Decision::Confirm));
        let mut dialog = opened(&[]);
        assert_eq!(dialog.handle(&character('Y')), Some(Decision::Confirm));
        for move_focus in [
            KeyCode::Tab,
            KeyCode::Right,
            KeyCode::Left,
            KeyCode::Char('l'),
            KeyCode::Char('h'),
        ] {
            let mut dialog = opened(&[]);
            assert_eq!(dialog.handle(&key(move_focus)), None);
            assert_eq!(
                dialog.handle(&key(KeyCode::Enter)),
                Some(Decision::Confirm),
                "{move_focus:?}"
            );
        }
        let mut dialog = opened(&[]);
        dialog.handle(&key(KeyCode::Tab));
        dialog.handle(&key(KeyCode::Tab));
        assert_eq!(dialog.handle(&key(KeyCode::Enter)), Some(Decision::Cancel));
    }

    #[test]
    fn n_q_and_escape_cancel_from_either_focus() {
        for code in [
            KeyCode::Char('n'),
            KeyCode::Char('N'),
            KeyCode::Char('q'),
            KeyCode::Esc,
        ] {
            let mut dialog = opened(&[]);
            dialog.handle(&key(KeyCode::Tab));
            assert_eq!(
                dialog.handle(&key(code)),
                Some(Decision::Cancel),
                "{code:?}"
            );
        }
    }

    #[test]
    fn other_keys_and_key_releases_decide_nothing() {
        let mut dialog = opened(&[]);
        for code in [
            KeyCode::Char('x'),
            KeyCode::Char('z'),
            KeyCode::Backspace,
            KeyCode::Insert,
        ] {
            assert_eq!(dialog.handle(&key(code)), None);
        }
        let mut release = KeyEvent::press(KeyCode::Char('y'));
        release.kind = KeyKind::Release;
        assert_eq!(dialog.handle(&Event::Key(release)), None);
        assert_eq!(dialog.decision(), None);
    }

    #[test]
    fn the_first_decision_is_final() {
        let mut dialog = opened(&[]);
        assert_eq!(dialog.handle(&character('n')), Some(Decision::Cancel));
        assert_eq!(dialog.handle(&character('y')), Some(Decision::Cancel));
        let mut dialog = opened(&[]);
        assert_eq!(dialog.handle(&character('y')), Some(Decision::Confirm));
        assert_eq!(dialog.handle(&character('n')), Some(Decision::Confirm));
        assert_eq!(dialog.decision(), Some(Decision::Confirm));
    }

    #[test]
    fn d_toggles_the_selected_file_and_selection_moves_and_clamps() {
        let mut dialog = opened(&[]);
        assert_eq!(dialog.selected(), 0);
        dialog.handle(&character('k'));
        dialog.handle(&key(KeyCode::Up));
        assert_eq!(dialog.selected(), 0);
        dialog.handle(&character('d'));
        assert!(dialog.is_expanded(0));
        assert!(!dialog.is_expanded(1));
        let shown = text(&mut dialog, 120, 36);
        assert!(
            shown.contains("-rounding") || shown.contains("-gaps = 5"),
            "{shown}"
        );
        assert!(shown.contains("+gaps = 8"), "{shown}");
        dialog.handle(&character('d'));
        assert!(!dialog.is_expanded(0));
        dialog.handle(&key(KeyCode::Down));
        dialog.handle(&character('j'));
        dialog.handle(&key(KeyCode::Down));
        dialog.handle(&key(KeyCode::Down));
        assert_eq!(dialog.selected(), 2);
        dialog.handle(&key(KeyCode::Home));
        assert_eq!(dialog.selected(), 0);
        dialog.handle(&key(KeyCode::End));
        assert_eq!(dialog.selected(), 2);
        assert_eq!(dialog.decision(), None);
    }

    #[test]
    fn expanding_a_file_without_a_text_diff_says_why() {
        let mut dialog = opened(&[]);
        dialog.handle(&key(KeyCode::End));
        dialog.handle(&character('d'));
        let shown = text(&mut dialog, 120, 36);
        assert!(shown.contains("(not written)"), "{shown}");
    }

    #[test]
    fn expanding_a_keep_mine_file_shows_how_upstream_differs() {
        let file = PlannedFile::new(
            "config/hypr/monitors.lua",
            Action::KeepMine,
            Some(&sha(b"scale = 2\n")),
            Some(&sha(b"scale = 2\n")),
            None,
            None,
        )
        .unwrap();
        let plan = Plan::new("txn-1", "2026-06-01T09:30:00Z", "4.5.0-3", vec![file], &[]).unwrap();
        let contents = [FileContents {
            path: "config/hypr/monitors.lua".to_string(),
            before: Some(b"scale = 2\n".to_vec()),
            after: Some(b"scale = 3\n".to_vec()),
        }];
        let mut dialog = PlanDialog::new(PlanView::build(&plan, STATE, &contents));
        let _ = text(&mut dialog, 120, 36);
        let before = text(&mut dialog, 120, 36);
        assert!(!before.contains("scale = 3"), "{before}");
        dialog.handle(&character('d'));
        let shown = text(&mut dialog, 120, 36);
        assert!(shown.contains("-scale = 2"), "{shown}");
        assert!(shown.contains("+scale = 3"), "{shown}");
        assert!(shown.contains("your file is kept"), "{shown}");
        assert!(shown.contains("(not written)"), "{shown}");
        assert!(
            shown.contains("backup: none (nothing is written)"),
            "{shown}"
        );
    }

    #[test]
    fn clicking_confirm_or_cancel_decides_and_nothing_else_does() {
        let area = Rect::new(0, 0, 120, 36);
        let mut dialog = opened(&[]);
        let confirm = dialog.confirm_bounds(area).unwrap();
        let (column, row) = centre(confirm);
        assert_eq!(dialog.handle(&click(column, row)), Some(Decision::Confirm));

        let mut dialog = opened(&[]);
        let cancel = dialog.cancel_bounds(area).unwrap();
        let (column, row) = centre(cancel);
        assert_eq!(dialog.handle(&click(column, row)), Some(Decision::Cancel));

        let mut dialog = opened(&[]);
        let confirm = dialog.confirm_bounds(area).unwrap();
        assert_eq!(
            dialog.handle(&click(confirm.x + confirm.width, confirm.y)),
            None
        );
        assert_eq!(dialog.handle(&click(confirm.x, confirm.y + 1)), None);
        assert_eq!(dialog.handle(&click(0, 0)), None);
        let right = Event::Mouse(MouseEvent::new(
            MouseKind::Down(MouseButton::Right),
            confirm.x,
            confirm.y,
        ));
        assert_eq!(dialog.handle(&right), None);
        let up = Event::Mouse(MouseEvent::new(
            MouseKind::Up(MouseButton::Left),
            confirm.x,
            confirm.y,
        ));
        assert_eq!(dialog.handle(&up), None);
        assert_eq!(dialog.decision(), None);
    }

    #[test]
    fn a_click_before_the_first_render_is_ignored() {
        let mut dialog = PlanDialog::new(sample_view(&[]));
        assert_eq!(dialog.handle(&click(100, 30)), None);
        assert_eq!(dialog.decision(), None);
    }

    #[test]
    fn clicking_a_file_row_selects_and_expands_it() {
        let mut dialog = opened(&[]);
        let list_row = 4;
        let column = 20;
        assert_eq!(dialog.handle(&click(column, list_row)), None);
        assert!(dialog.is_expanded(0));
        assert_eq!(dialog.selected(), 0);
        assert_eq!(dialog.handle(&click(column, list_row)), None);
        assert!(!dialog.is_expanded(0));
        let shown = text(&mut dialog, 120, 36);
        let row_of_second = shown
            .lines()
            .position(|line| line.contains("config/waybar/style.css"))
            .unwrap() as u16;
        assert_eq!(dialog.handle(&click(column, row_of_second)), None);
        assert_eq!(dialog.selected(), 1);
        assert!(dialog.is_expanded(1));
    }

    #[test]
    fn the_wheel_scrolls_the_list_and_stops_at_the_ends() {
        let mut dialog = PlanDialog::new(many_files_view(30));
        let _ = text(&mut dialog, 80, 24);
        let first = text(&mut dialog, 80, 24);
        assert!(first.contains("file00.conf"), "{first}");
        assert!(!first.contains("file29.conf"), "{first}");
        let wheel = |scroll| Event::Mouse(MouseEvent::new(MouseKind::Scroll(scroll), 40, 10));
        for _ in 0..40 {
            dialog.handle(&wheel(Scroll::Down));
        }
        let last = text(&mut dialog, 80, 24);
        assert!(last.contains("file29.conf"), "{last}");
        assert!(!last.contains("file00.conf"), "{last}");
        for _ in 0..40 {
            dialog.handle(&wheel(Scroll::Up));
        }
        let top = text(&mut dialog, 80, 24);
        assert!(top.contains("file00.conf"), "{top}");
        assert_eq!(dialog.decision(), None);
    }

    #[test]
    fn keyboard_selection_keeps_the_selected_file_on_screen() {
        let mut dialog = PlanDialog::new(many_files_view(30));
        let _ = text(&mut dialog, 80, 24);
        for _ in 0..20 {
            dialog.handle(&key(KeyCode::Down));
        }
        let shown = text(&mut dialog, 80, 24);
        let marked = shown.lines().find(|line| line.contains("> [+]")).unwrap();
        assert!(marked.contains("file20.conf"), "{shown}");
        dialog.handle(&key(KeyCode::Home));
        let shown = text(&mut dialog, 80, 24);
        assert!(
            shown
                .lines()
                .any(|line| line.contains("> [+]") && line.contains("file00.conf")),
            "{shown}"
        );
    }

    #[test]
    fn long_text_is_clipped_and_never_leaves_the_dialog() {
        let mut dialog = opened(&[]);
        let shown = text(&mut dialog, 40, 12);
        for line in shown.lines() {
            assert!(width(line) <= 40, "{line:?}");
        }
        assert!(shown.contains("Cancel"), "{shown}");
    }

    struct World {
        root: drift_fixtures::MaterializedRoot,
    }

    impl World {
        fn new() -> Self {
            Self::build(false)
        }

        fn build(locked: bool) -> Self {
            let mut builder = RootBuilder::new();
            if locked {
                builder.file("runtime/omarchy-update.lock", b"").unwrap();
            }
            builder
                .file("live/config/a.lua", b"mine\n")
                .unwrap()
                .file("live/config/b.lua", b"keep\n")
                .unwrap()
                .file("state/.keep", b"")
                .unwrap()
                .file("runtime/.keep", b"")
                .unwrap();
            Self {
                root: builder.materialize().unwrap(),
            }
        }

        fn roots(&self) -> Roots {
            Roots {
                live: self.root.path().join("live"),
                state: self.root.path().join("state"),
                omarchy_lock: self.root.path().join("runtime/omarchy-update.lock"),
                pacman_lock: self.root.path().join("runtime/db.lck"),
            }
        }
    }

    fn apply_plan() -> (Plan, Vec<(String, Vec<u8>)>) {
        let theirs: &[u8] = b"theirs\n";
        let files = vec![
            PlannedFile::new(
                "config/a.lua",
                Action::TakeUpstream,
                Some(&sha(b"mine\n")),
                Some(&sha(theirs)),
                Some(0o600),
                None,
            )
            .unwrap(),
            PlannedFile::new(
                "config/c.lua",
                Action::TakeUpstream,
                None,
                Some(&sha(theirs)),
                None,
                None,
            )
            .unwrap(),
            PlannedFile::new(
                "config/b.lua",
                Action::KeepMine,
                Some(&sha(b"keep\n")),
                Some(&sha(b"keep\n")),
                None,
                None,
            )
            .unwrap(),
        ];
        let plan = Plan::new("txn-1", "2026-06-01T09:30:00Z", "4.5.0-3", files, &[]).unwrap();
        let bytes = vec![
            ("config/a.lua".to_string(), theirs.to_vec()),
            ("config/c.lua".to_string(), theirs.to_vec()),
            ("config/b.lua".to_string(), b"keep\n".to_vec()),
        ];
        (plan, bytes)
    }

    fn run_decision(world: &World, decision: Decision) -> Option<Vec<String>> {
        let (plan, bytes) = apply_plan();
        let payloads: Vec<Payload<'_>> = bytes
            .iter()
            .map(|(path, bytes)| Payload {
                path,
                bytes,
                volatile: false,
                allow_volatile: false,
                allow_symlink: false,
            })
            .collect();
        let request = Request {
            plan: &plan,
            files: &payloads,
            fail_at: None,
        };
        apply_decision(decision, &world.roots(), &request)
            .unwrap()
            .map(|report| report.applied)
    }

    fn outcome(world: &World) -> (Vec<u8>, Vec<u8>, Vec<u8>, String, Vec<u8>) {
        let live = world.root.path().join("live/config");
        let state = world.root.path().join("state/backups/txn-1");
        (
            std::fs::read(live.join("a.lua")).unwrap(),
            std::fs::read(live.join("c.lua")).unwrap(),
            std::fs::read(live.join("b.lua")).unwrap(),
            std::fs::read_to_string(state.join("manifest.json")).unwrap(),
            std::fs::read(state.join("config/a.lua")).unwrap(),
        )
    }

    #[test]
    fn keyboard_and_mouse_confirm_produce_the_same_transaction() {
        let area = Rect::new(0, 0, 120, 36);
        let mut results = Vec::new();
        for method in ["y", "tab-enter", "click"] {
            let world = World::new();
            let mut dialog = PlanDialog::new(sample_view(&[]));
            let _ = text(&mut dialog, 120, 36);
            let decision = match method {
                "y" => dialog.handle(&character('y')),
                "tab-enter" => {
                    dialog.handle(&key(KeyCode::Tab));
                    dialog.handle(&key(KeyCode::Enter))
                }
                _ => {
                    let (column, row) = centre(dialog.confirm_bounds(area).unwrap());
                    dialog.handle(&click(column, row))
                }
            }
            .unwrap();
            assert_eq!(decision, Decision::Confirm, "{method}");
            let applied = run_decision(&world, decision).unwrap();
            assert_eq!(
                applied,
                ["config/a.lua", "config/c.lua", "config/b.lua"],
                "{method}"
            );
            results.push(outcome(&world));
        }
        assert_eq!(results[0], results[1]);
        assert_eq!(results[0], results[2]);
        assert_eq!(results[0].0, b"theirs\n");
        assert_eq!(results[0].2, b"keep\n");
        assert_eq!(results[0].4, b"mine\n");
    }

    #[test]
    fn every_way_of_cancelling_leaves_the_whole_tree_unchanged() {
        let area = Rect::new(0, 0, 120, 36);
        for method in ["enter", "n", "esc", "click"] {
            let world = World::new();
            let before = hash_tree(world.root.path()).unwrap();
            let mut dialog = PlanDialog::new(sample_view(&[]));
            let _ = text(&mut dialog, 120, 36);
            let decision = match method {
                "enter" => dialog.handle(&key(KeyCode::Enter)),
                "n" => dialog.handle(&character('n')),
                "esc" => dialog.handle(&key(KeyCode::Esc)),
                _ => {
                    let (column, row) = centre(dialog.cancel_bounds(area).unwrap());
                    dialog.handle(&click(column, row))
                }
            }
            .unwrap();
            assert_eq!(decision, Decision::Cancel, "{method}");
            assert_eq!(run_decision(&world, decision), None);
            assert_eq!(hash_tree(world.root.path()).unwrap(), before, "{method}");
            assert!(
                !world.root.path().join("state/backups").exists(),
                "{method}"
            );
        }
    }

    #[test]
    fn a_dry_run_prints_the_plan_and_writes_nothing() {
        let world = World::new();
        let before = hash_tree(world.root.path()).unwrap();
        let printed = sample_view(&["old-1"]).render_text();
        assert!(
            printed.contains("take upstream  config/waybar/style.css  (new file, 3 lines)"),
            "{printed}"
        );
        assert!(printed.contains("prune  old-1"), "{printed}");
        assert!(
            printed.ends_with("Dry run: nothing was written.\n"),
            "{printed}"
        );
        assert_eq!(hash_tree(world.root.path()).unwrap(), before);
    }

    #[test]
    fn a_failed_apply_after_confirm_is_reported_and_not_hidden() {
        let world = World::build(true);
        let (plan, bytes) = apply_plan();
        let payloads: Vec<Payload<'_>> = bytes
            .iter()
            .map(|(path, bytes)| Payload {
                path,
                bytes,
                volatile: false,
                allow_volatile: false,
                allow_symlink: false,
            })
            .collect();
        let request = Request {
            plan: &plan,
            files: &payloads,
            fail_at: None,
        };
        let error = apply_decision(Decision::Confirm, &world.roots(), &request).unwrap_err();
        assert!(error.message.contains("omarchy"), "{}", error.message);
    }

    #[test]
    fn frames_stay_under_the_budget() {
        for (columns, rows, _) in [(80, 24, ""), (200, 60, "")] {
            let mut dialog = PlanDialog::new(many_files_view(40));
            dialog.handle(&character('d'));
            let mut worst = Duration::ZERO;
            with_zone("UTC0", || {
                for step in 0..32 {
                    if step % 2 == 0 {
                        dialog.handle(&key(KeyCode::Down));
                    }
                    worst = worst.max(cpu_cost(|| {
                        let _ = paint(&mut dialog, columns, rows);
                    }));
                }
            });
            assert!(
                worst < Duration::from_millis(8),
                "{columns}x{rows} worst {worst:?}"
            );
        }
    }

    #[test]
    fn the_plan_dialog_shows_local_time_and_keeps_the_stored_utc_time() {
        assert_eq!(utc_unix("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(utc_unix("2026-06-01T09:30:00Z"), Some(1_780_306_200));
        assert_eq!(utc_unix("2026-11-13T03:02:30Z"), Some(1_794_538_950));
        assert_eq!(utc_unix("2026-11-13T03:02:30+00:00"), None);
        let stored = "2026-11-13T03:02:30Z";
        with_zone("FOO7", || {
            let mut dialog = PlanDialog::new(timed_view(stored));
            assert_eq!(dialog.view().time, stored);
            let shown = paint(&mut dialog, 120, 36);
            assert!(!shown.contains(stored), "{shown}");
            assert!(shown.contains("2026-11-12 20:02:30 -0700"), "{shown}");
        });
        let mut raw = PlanDialog::new(timed_view("not-utc"));
        let shown = paint(&mut raw, 80, 24);
        assert!(shown.contains("not-utc - package"), "{shown}");
    }

    fn timed_view(time: &str) -> PlanView {
        let file = PlannedFile::new("config/a", Action::KeepMine, None, None, None, None).unwrap();
        let plan = Plan::new("txn-1", time, "1", vec![file], &[]).unwrap();
        PlanView::build(&plan, STATE, &[])
    }
}
