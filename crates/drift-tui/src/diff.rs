use std::ops::Range;

use drift_core::{merge3, DiffRows, Hunk, HunkKind, Mark, Merge, Row, Span};
use lunatui::{App, ControlFlow};
use lunatui_core::{clip_display_prefix, Attrs, Color, Grid, Rect, Style};
use lunatui_runtime::keymap::{Command, KeyChord, Keymap};
use lunatui_term::{Event, KeyCode, Modifiers, MouseButton, MouseEvent, MouseKind, Scroll};
use lunatui_theme::{Theme, Token, Variant};
use lunatui_widgets::{
    CodeHighlight, CodeMark, CodeView, CodeViewState, DividerId, KeyHintBar, RenderCx, Scrollbar,
    SplitAxis, SplitDrag, SplitTree, Widget, WidgetState,
};
use unicode_width::UnicodeWidthChar;

const WIDE: u16 = 160;
const QUIT: &str = "diff.quit";
const NEXT: &str = "diff.next";
const PREVIOUS: &str = "diff.previous";
const FOCUS_NEXT: &str = "diff.focus-next";
const FOCUS_PREVIOUS: &str = "diff.focus-previous";
const WIDEN: &str = "diff.widen";
const NARROW: &str = "diff.narrow";
const DOWN: &str = "diff.down";
const UP: &str = "diff.up";
const PANES: [&str; 5] = ["hunks", "base", "ours", "theirs", "result"];
const HIGHLIGHT: Color = Color::Rgb(220, 160, 40);

const QUIT_CHORDS: &[KeyChord] = &[KeyChord::new(KeyCode::Char('q'), Modifiers::new())];
const NEXT_CHORDS: &[KeyChord] = &[KeyChord::new(KeyCode::Char('n'), Modifiers::new())];
const PREVIOUS_CHORDS: &[KeyChord] = &[KeyChord::new(KeyCode::Char('p'), Modifiers::new())];
const FOCUS_NEXT_CHORDS: &[KeyChord] = &[KeyChord::new(KeyCode::Tab, Modifiers::new())];
const FOCUS_PREVIOUS_CHORDS: &[KeyChord] = &[KeyChord::new(KeyCode::Tab, Modifiers::new().shift())];
const WIDEN_CHORDS: &[KeyChord] = &[KeyChord::new(KeyCode::Char(']'), Modifiers::new())];
const NARROW_CHORDS: &[KeyChord] = &[KeyChord::new(KeyCode::Char('['), Modifiers::new())];
const DOWN_CHORDS: &[KeyChord] = &[KeyChord::new(KeyCode::Down, Modifiers::new())];
const UP_CHORDS: &[KeyChord] = &[KeyChord::new(KeyCode::Up, Modifiers::new())];

pub struct DiffScreen {
    aligned: Aligned,
    scroll: CodeViewState,
    tree: SplitTree,
    drag: SplitDrag,
    focus: usize,
    hunk: usize,
    keymap: Keymap,
    columns: u16,
    rows: u16,
}

#[derive(Clone)]
struct Line {
    text: String,
    spans: Vec<Range<usize>>,
    mark: Option<CodeMark>,
}

impl AsRef<str> for Line {
    fn as_ref(&self) -> &str {
        &self.text
    }
}

struct Aligned {
    base: Vec<Line>,
    ours: Vec<Line>,
    theirs: Vec<Line>,
    result: Vec<Line>,
    hunk_rows: Vec<usize>,
    hunk_labels: Vec<String>,
}

struct Hits<'a> {
    lines: &'a [Line],
}

impl CodeHighlight for Hits<'_> {
    fn highlight(&self, line: usize, _text: &str, paint: &mut dyn FnMut(Range<usize>, Style)) {
        let Some(line) = self.lines.get(line) else {
            return;
        };
        let style = Style::new().fg(HIGHLIGHT);
        for span in &line.spans {
            paint(span.clone(), style);
        }
    }
}

impl DiffScreen {
    pub fn new(base: &str, ours: &str, theirs: &str) -> Self {
        let merge = merge3(base, ours, theirs);
        Self::from_merge(&merge)
    }

    pub fn from_merge(merge: &Merge) -> Self {
        let (mut tree, hunks) = SplitTree::new();
        let base = tree
            .split(hunks, SplitAxis::Horizontal, 160)
            .expect("hunk list");
        let ours = tree.split(base, SplitAxis::Horizontal, 250).expect("base");
        let theirs = tree.split(ours, SplitAxis::Horizontal, 333).expect("ours");
        tree.split(theirs, SplitAxis::Horizontal, 500)
            .expect("theirs");
        for index in 0..4 {
            tree.set_min(DividerId(index), 6, 6);
        }
        Self {
            aligned: align(merge),
            scroll: CodeViewState::new(),
            tree,
            drag: SplitDrag::new(),
            focus: 0,
            hunk: 0,
            keymap: keymap(),
            columns: 0,
            rows: 0,
        }
    }

    pub fn scroll_top(&self) -> usize {
        self.scroll.top()
    }

    pub fn focused(&self) -> &'static str {
        PANES[self.focus]
    }

    pub fn selected_hunk(&self) -> usize {
        self.hunk
    }

    pub fn pane_widths(&self) -> Vec<u16> {
        self.tree
            .layout(self.body())
            .panes
            .iter()
            .map(|pane| pane.rect.width)
            .collect()
    }

    pub fn divider_x(&self, index: usize) -> Option<u16> {
        self.tree
            .layout(self.body())
            .dividers
            .get(index)
            .map(|divider| divider.rect.x)
    }

    fn body(&self) -> Rect {
        regions(Rect::new(0, 0, self.columns, self.rows)).1
    }

    fn wide(&self) -> bool {
        self.columns >= WIDE && self.rows > 2
    }

    fn scroll_by(&mut self, lines: isize) {
        if lines > 0 {
            self.scroll.down(lines as usize);
        } else {
            self.scroll.up(lines.unsigned_abs());
        }
    }

    fn move_hunk(&mut self, delta: isize) {
        let count = self.aligned.hunk_rows.len();
        if count == 0 {
            return;
        }
        let next = self.hunk as isize + delta;
        self.hunk = next.clamp(0, count as isize - 1) as usize;
        self.scroll.scroll_to(self.aligned.hunk_rows[self.hunk]);
    }

    fn move_focus(&mut self, delta: isize) {
        let count = PANES.len() as isize;
        self.focus = (self.focus as isize + delta).rem_euclid(count) as usize;
    }

    fn resize_focused(&mut self, delta: i16) {
        if !self.wide() {
            return;
        }
        let panes = self.tree.panes();
        let Some(pane) = panes.get(self.focus) else {
            return;
        };
        let Some(divider) = self.tree.divider_for(*pane, SplitAxis::Horizontal) else {
            return;
        };
        self.tree.resize(divider, delta, self.body());
    }
}

fn keymap() -> Keymap {
    let mut map = Keymap::new();
    map.register(Command::new(QUIT, "Quit", QUIT_CHORDS));
    map.register(Command::new(NEXT, "Next hunk", NEXT_CHORDS));
    map.register(Command::new(PREVIOUS, "Previous hunk", PREVIOUS_CHORDS));
    map.register(Command::new(FOCUS_NEXT, "Next pane", FOCUS_NEXT_CHORDS));
    map.register(Command::new(
        FOCUS_PREVIOUS,
        "Previous pane",
        FOCUS_PREVIOUS_CHORDS,
    ));
    map.register(Command::new(WIDEN, "Widen pane", WIDEN_CHORDS));
    map.register(Command::new(NARROW, "Narrow pane", NARROW_CHORDS));
    map.register(Command::new(DOWN, "Scroll down", DOWN_CHORDS));
    map.register(Command::new(UP, "Scroll up", UP_CHORDS));
    map
}

fn align(merge: &Merge) -> Aligned {
    let built = DiffRows::build(merge);
    let rows = built.rows();
    let mut at = 0;
    let mut aligned = Aligned {
        base: Vec::new(),
        ours: Vec::new(),
        theirs: Vec::new(),
        result: Vec::new(),
        hunk_rows: Vec::new(),
        hunk_labels: Vec::new(),
    };
    let mut hunk_index = 0;
    for segment in merge.segments() {
        match segment {
            drift_core::Segment::Stable(text) => {
                for line in split_lines(text) {
                    let shown = if at < rows.len() && rows[at].hunk.is_none() {
                        let text = rows[at].text.clone();
                        at += 1;
                        text
                    } else {
                        line
                    };
                    let plain = Line {
                        text: shown,
                        spans: Vec::new(),
                        mark: None,
                    };
                    aligned.base.push(plain.clone());
                    aligned.ours.push(plain.clone());
                    aligned.theirs.push(plain.clone());
                    aligned.result.push(plain);
                }
            }
            drift_core::Segment::Hunk(hunk) => {
                let start = at;
                while at < rows.len() && rows[at].hunk == Some(hunk_index) {
                    at += 1;
                }
                push_hunk(&mut aligned, hunk, hunk_index, &rows[start..at]);
                hunk_index += 1;
            }
        }
    }
    aligned
}

fn push_hunk(aligned: &mut Aligned, hunk: &Hunk, index: usize, rows: &[Row]) {
    aligned.hunk_rows.push(aligned.base.len());
    aligned
        .hunk_labels
        .push(format!("{} {}", index + 1, hunk_name(hunk.kind)));
    let (base, ours, theirs, result) = match hunk.kind {
        HunkKind::Conflict => {
            let ours = from_mark(rows, Mark::Mine);
            let theirs = from_mark(rows, Mark::Theirs);
            let base = raw_lines(&hunk.base);
            let result = vec![Line {
                text: "unresolved".to_string(),
                spans: Vec::new(),
                mark: Some(CodeMark::Error),
            }];
            (base, ours, theirs, result)
        }
        HunkKind::TheirsOnly => {
            let base = or_raw(from_mark(rows, Mark::Removed), &hunk.base);
            let theirs = or_raw(from_mark(rows, Mark::Added), &hunk.theirs);
            let ours = plain(&base);
            (base, ours, theirs.clone(), theirs)
        }
        HunkKind::OursOnly => {
            let base = or_raw(from_mark(rows, Mark::Removed), &hunk.base);
            let ours = or_raw(from_mark(rows, Mark::Added), &hunk.ours);
            let theirs = plain(&base);
            (base, ours.clone(), theirs, ours)
        }
        HunkKind::BothSame => {
            let base = or_raw(from_mark(rows, Mark::Removed), &hunk.base);
            let ours = or_raw(from_mark(rows, Mark::Added), &hunk.ours);
            (base, ours.clone(), ours.clone(), ours)
        }
    };
    let height = [base.len(), ours.len(), theirs.len(), result.len()]
        .into_iter()
        .max()
        .unwrap_or(0);
    extend(&mut aligned.base, base, height);
    extend(&mut aligned.ours, ours, height);
    extend(&mut aligned.theirs, theirs, height);
    extend(&mut aligned.result, result, height);
}

fn hunk_name(kind: HunkKind) -> &'static str {
    match kind {
        HunkKind::OursOnly => "ours",
        HunkKind::TheirsOnly => "theirs",
        HunkKind::BothSame => "both",
        HunkKind::Conflict => "conflict",
    }
}

fn from_mark(rows: &[Row], mark: Mark) -> Vec<Line> {
    rows.iter()
        .filter(|row| row.mark == mark)
        .map(line_from_row)
        .collect()
}

fn or_raw(lines: Vec<Line>, text: &str) -> Vec<Line> {
    if lines.is_empty() {
        raw_lines(text)
    } else {
        lines
    }
}

fn plain(lines: &[Line]) -> Vec<Line> {
    lines
        .iter()
        .map(|line| Line {
            text: line.text.clone(),
            spans: Vec::new(),
            mark: None,
        })
        .collect()
}

fn line_from_row(row: &Row) -> Line {
    let spans = row
        .spans
        .iter()
        .filter_map(|span| columns_to_bytes(&row.text, *span))
        .collect::<Vec<_>>();
    let mark = if spans.is_empty() {
        None
    } else {
        Some(CodeMark::Changed)
    };
    Line {
        text: row.text.clone(),
        spans,
        mark,
    }
}

fn raw_lines(text: &str) -> Vec<Line> {
    split_lines(text)
        .into_iter()
        .map(|text| Line {
            text,
            spans: Vec::new(),
            mark: None,
        })
        .collect()
}

fn extend(into: &mut Vec<Line>, mut lines: Vec<Line>, height: usize) {
    while lines.len() < height {
        lines.push(Line {
            text: String::new(),
            spans: Vec::new(),
            mark: None,
        });
    }
    into.extend(lines);
}

fn split_lines(text: &str) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    text.split_inclusive('\n')
        .map(|line| {
            line.strip_suffix("\r\n")
                .or_else(|| line.strip_suffix('\n'))
                .unwrap_or(line)
                .to_string()
        })
        .collect()
}

fn ascii_line(text: &str) -> bool {
    text.bytes().all(|byte| (0x20..0x7f).contains(&byte))
}

fn columns_to_bytes(text: &str, span: Span) -> Option<Range<usize>> {
    if span.start >= span.end {
        return None;
    }
    if text.is_ascii() && span.end <= text.len() {
        return Some(span.start..span.end);
    }
    let mut column = 0usize;
    let mut start = None;
    for (at, ch) in text.char_indices() {
        let width = UnicodeWidthChar::width(ch).unwrap_or(0);
        let next = column + width;
        if start.is_none() && next > span.start {
            start = Some(at);
        }
        if start.is_some() && next >= span.end {
            return Some(start?..at + ch.len_utf8());
        }
        column = next;
    }
    None
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
    let body = Rect::new(area.x, area.y + 1, area.width, area.height - 2);
    (header, body, hint)
}

fn contains(area: Rect, column: u16, row: u16) -> bool {
    column >= area.x
        && row >= area.y
        && column < area.x.saturating_add(area.width)
        && row < area.y.saturating_add(area.height)
}

impl App for DiffScreen {
    fn update(&mut self, event: &Event) -> ControlFlow {
        if let Event::Mouse(mouse) = event {
            if mouse_quits(self.columns, self.rows, mouse) {
                return ControlFlow::Break;
            }
            if self.wide() {
                self.pointer(mouse);
            }
            return ControlFlow::Continue;
        }
        match self.keymap.dispatch(event) {
            Some(QUIT) => ControlFlow::Break,
            Some(NEXT) => {
                self.move_hunk(1);
                ControlFlow::Continue
            }
            Some(PREVIOUS) => {
                self.move_hunk(-1);
                ControlFlow::Continue
            }
            Some(FOCUS_NEXT) => {
                self.move_focus(1);
                ControlFlow::Continue
            }
            Some(FOCUS_PREVIOUS) => {
                self.move_focus(-1);
                ControlFlow::Continue
            }
            Some(WIDEN) => {
                self.resize_focused(4);
                ControlFlow::Continue
            }
            Some(NARROW) => {
                self.resize_focused(-4);
                ControlFlow::Continue
            }
            Some(DOWN) => {
                self.scroll_by(1);
                ControlFlow::Continue
            }
            Some(UP) => {
                self.scroll_by(-1);
                ControlFlow::Continue
            }
            _ => ControlFlow::Continue,
        }
    }

    fn render(&mut self, grid: &mut Grid, area: Rect, clusters: &mut lunatui_core::ClusterTable) {
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
        let (header, body, hint) = regions(area);
        if header.width > 0 && header.height > 0 {
            cx.put_str(
                header.x,
                header.y,
                clip_display_prefix("Diff", header.width),
                accent,
            );
        }
        if body.width > 0 && body.height > 0 {
            if area.width >= WIDE {
                self.render_wide(body, &mut cx, accent, text);
            } else {
                cx.put_str(
                    body.x,
                    body.y,
                    clip_display_prefix("Four panes need a wider window", body.width),
                    text,
                );
            }
        }
        if hint.width > 0 && hint.height > 0 {
            KeyHintBar::new(&self.keymap).render(hint, &mut cx);
        }
    }
}

impl DiffScreen {
    fn pointer(&mut self, mouse: &MouseEvent) {
        let body = self.body();
        let layout = self.tree.layout(body);
        match mouse.kind {
            MouseKind::Down(MouseButton::Left) => {
                self.drag
                    .press(&self.tree, &layout, mouse.column, mouse.row);
            }
            MouseKind::Drag(MouseButton::Left) => {
                self.drag.drag(&mut self.tree, mouse.column, mouse.row);
            }
            MouseKind::Up(MouseButton::Left) => {
                self.drag.release();
            }
            MouseKind::Scroll(Scroll::Down) => self.scroll_by(3),
            MouseKind::Scroll(Scroll::Up) => self.scroll_by(-3),
            _ => {}
        }
    }

    fn render_wide(&mut self, body: Rect, cx: &mut RenderCx<'_>, accent: Style, text: Style) {
        let scroll = self.prepare_scroll(body);
        let layout = self.tree.layout(body);
        let panes = [
            &self.aligned.base,
            &self.aligned.ours,
            &self.aligned.theirs,
            &self.aligned.result,
        ];
        for (index, area) in layout.panes.iter().enumerate() {
            if index == 0 {
                self.render_hunks(area.rect, cx, accent, text);
                continue;
            }
            let lines = panes[index - 1];
            let style = if self.focus == index { accent } else { text };
            self.render_code(area.rect, PANES[index], lines, style, scroll, cx);
        }
        let active = self.drag.active().or_else(|| self.focused_divider());
        layout.render_dividers(text, active, accent, cx);
    }

    fn prepare_scroll(&mut self, body: Rect) -> CodeViewState {
        let layout = self.tree.layout(body);
        let height = layout
            .panes
            .get(1)
            .map(|pane| pane.rect.height.saturating_sub(1))
            .unwrap_or(0);
        let mut scroll = self.scroll;
        let area = Rect::new(0, 0, 8, height);
        CodeView::new(self.aligned.base.as_slice()).sync(area, &mut scroll);
        self.scroll = scroll;
        scroll
    }

    fn focused_divider(&self) -> Option<DividerId> {
        let panes = self.tree.panes();
        let pane = panes.get(self.focus)?;
        self.tree.divider_for(*pane, SplitAxis::Horizontal)
    }

    fn render_hunks(&self, area: Rect, cx: &mut RenderCx<'_>, accent: Style, text: Style) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let style = if self.focus == 0 { accent } else { text };
        cx.put_str(
            area.x,
            area.y,
            clip_display_prefix("hunks", area.width),
            style,
        );
        if area.height == 1 {
            return;
        }
        let labels = &self.aligned.hunk_labels;
        let start = self.hunk.saturating_sub(1);
        for row in 1..area.height {
            let index = start + usize::from(row - 1);
            let Some(label) = labels.get(index) else {
                break;
            };
            let shown = if index == self.hunk {
                format!("> {label}")
            } else {
                format!("  {label}")
            };
            let style = if index == self.hunk { accent } else { text };
            cx.put_str(
                area.x,
                area.y + row,
                clip_display_prefix(&shown, area.width),
                style,
            );
        }
    }

    fn render_code(
        &self,
        area: Rect,
        title: &str,
        lines: &[Line],
        title_style: Style,
        scroll: CodeViewState,
        cx: &mut RenderCx<'_>,
    ) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        cx.put_str(
            area.x,
            area.y,
            clip_display_prefix(title, area.width),
            title_style,
        );
        if area.height == 1 {
            return;
        }
        let view_area = Rect::new(area.x, area.y + 1, area.width, area.height - 1);
        let top = scroll.top();
        let rows = usize::from(view_area.height);
        let mut marks = Vec::new();
        let mut fast = scroll.left() == 0;
        for (offset, line) in lines.iter().skip(top).take(rows).enumerate() {
            if let Some(mark) = line.mark {
                marks.push((top + offset, mark));
            }
            fast = fast && ascii_line(&line.text);
        }
        marks.push((usize::MAX, CodeMark::Changed));
        let state = if self.focused() == title {
            WidgetState::Focused
        } else {
            WidgetState::Normal
        };
        if fast {
            paint_ascii(view_area, lines, top, rows, cx);
            return;
        }
        let hits = Hits { lines };
        CodeView::new(lines)
            .view(&scroll)
            .marks(&marks)
            .highlight(&hits)
            .state(state)
            .render(view_area, cx);
    }
}

fn paint_ascii(view_area: Rect, lines: &[Line], top: usize, rows: usize, cx: &mut RenderCx<'_>) {
    let plain = Style::new().fg(cx.token(Token::TextPrimary));
    let numbers = plain.with_attrs(Attrs::new().set_dim(true));
    let highlight = Style::new().fg(HIGHLIGHT);
    let places = digit_places(lines.len().max(1));
    let gutter = places.saturating_add(2);
    let bar = u16::from(view_area.width > gutter && lines.len() > rows);
    let text_width = view_area.width.saturating_sub(gutter).saturating_sub(bar);
    if view_area.width > 0 && view_area.height > 0 {
        cx.fill(view_area, plain, ' ');
    }
    let text_x = view_area.x.saturating_add(gutter.min(view_area.width));
    let limit = usize::from(text_width);
    for (offset, line) in lines.iter().skip(top).take(rows).enumerate() {
        let y = view_area
            .y
            .saturating_add(u16::try_from(offset).unwrap_or(0));
        if let Some(mark) = line.mark {
            let (glyph, style) = mark_paint(mark, cx);
            if view_area.width > 0 {
                cx.put_str(view_area.x, y, glyph, style);
            }
        }
        if places > 0 && gutter <= view_area.width {
            let mut buffer = [0_u8; 20];
            let shown = write_number(top + offset + 1, &mut buffer);
            let skip = shown.len().saturating_sub(usize::from(places));
            let shown = &shown[skip..];
            let x = view_area.x.saturating_add(1).saturating_add(
                u16::try_from(usize::from(places).saturating_sub(shown.len())).unwrap_or(0),
            );
            cx.put_str(x, y, shown, numbers);
        }
        if limit == 0 || line.text.is_empty() {
            continue;
        }
        let shown = if line.text.len() > limit {
            &line.text[..limit]
        } else {
            line.text.as_str()
        };
        cx.put_str(text_x, y, shown, plain);
        for span in &line.spans {
            if span.start >= shown.len() {
                continue;
            }
            let end = span.end.min(shown.len());
            if end <= span.start {
                continue;
            }
            let x = text_x.saturating_add(u16::try_from(span.start).unwrap_or(0));
            let width = u16::try_from(end - span.start).unwrap_or(0);
            cx.set_style(Rect::new(x, y, width, 1), highlight);
        }
    }
    if bar == 1 {
        Scrollbar::vertical(lines.len(), rows, top).render(
            Rect::new(
                view_area.x + view_area.width - 1,
                view_area.y,
                1,
                view_area.height,
            ),
            cx,
        );
    }
}

fn digit_places(mut value: usize) -> u16 {
    let mut count = 1_u16;
    while value >= 10 {
        value /= 10;
        count += 1;
    }
    count
}

fn write_number(mut number: usize, buffer: &mut [u8; 20]) -> &str {
    let mut at = buffer.len();
    loop {
        at -= 1;
        buffer[at] = b'0' + u8::try_from(number % 10).unwrap_or(0);
        number /= 10;
        if number == 0 {
            break;
        }
    }
    std::str::from_utf8(&buffer[at..]).unwrap_or("")
}

fn mark_paint(mark: CodeMark, cx: &RenderCx<'_>) -> (&'static str, Style) {
    match mark {
        CodeMark::Error => ("●", Style::new().fg(cx.token(Token::Fail))),
        _ => ("▎", Style::new().fg(cx.token(Token::Warn))),
    }
}

fn mouse_quits(columns: u16, rows: u16, mouse: &MouseEvent) -> bool {
    if !matches!(mouse.kind, MouseKind::Down(MouseButton::Left)) {
        return false;
    }
    crate::quit_bounds(columns, rows).is_some_and(|hit| contains(hit, mouse.column, mouse.row))
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use drift_core::MAX_DIFF_BYTES;
    use lunatui::render_once;
    use lunatui_term::KeyEvent;
    use lunatui_test::styled_snapshot;

    use super::*;

    fn sample_near_the_cap() -> (String, usize) {
        let line = format!("{}\n", "a".repeat(63));
        let count = (MAX_DIFF_BYTES as usize - 1) / line.len();
        (line.repeat(count), count)
    }

    fn cmp_row(text: &str, needle: &str) -> Option<usize> {
        text.lines().position(|line| line.contains(needle))
    }

    const SIZES: [(u16, u16); 2] = [(160, 40), (200, 60)];

    fn changed() -> DiffScreen {
        DiffScreen::new(
            "alpha\nhello world\nomega\n",
            "alpha\nhello WORLD\nomega\n",
            "alpha\nhello world\nomega\n",
        )
    }

    fn two_hunks() -> DiffScreen {
        let mut base = String::from("a\n");
        let mut ours = String::from("A\n");
        let mut theirs = String::from("a\n");
        for index in 0..50 {
            let line = format!("c{index:02}\n");
            base.push_str(&line);
            ours.push_str(&line);
            theirs.push_str(&line);
        }
        base.push_str("z\n");
        ours.push_str("z\n");
        theirs.push_str("Z\n");
        DiffScreen::new(&base, &ours, &theirs)
    }

    fn long_file() -> DiffScreen {
        let mut base = String::new();
        let mut ours = String::new();
        for index in 0..80 {
            base.push_str(&format!("b{index:02}\n"));
            ours.push_str(&format!("o{index:02}\n"));
        }
        let theirs = base.clone();
        DiffScreen::new(&base, &ours, &theirs)
    }

    #[test]
    fn snapshots_show_four_panes_at_wide_sizes() {
        for (columns, rows) in SIZES {
            let mut screen = changed();
            let (grid, clusters) = render_once(&mut screen, columns, rows);
            let text = grid.display_text(&clusters);
            assert!(text.lines().next().unwrap().contains("Diff"), "{columns}");
            let body = text.lines().nth(1).unwrap();
            assert!(body.contains("hunks"), "{columns}");
            assert!(body.contains("base"), "{columns}");
            assert!(body.contains("ours"), "{columns}");
            assert!(body.contains("theirs"), "{columns}");
            assert!(body.contains("result"), "{columns}");
            assert!(text.contains("hello WORLD"), "{columns}");
            assert!(text.contains("hello world"), "{columns}");
            let world = cmp_row(&text, "WORLD").unwrap();
            let snap = styled_snapshot(&grid, &clusters);
            let colors = fg_colors(&snap, world);
            assert!(colors.len() > 1, "{columns} {colors:?}");
            let expected = match (columns, rows) {
                (160, 40) => include_str!("../snapshots/diff-160x40.txt"),
                (200, 60) => include_str!("../snapshots/diff-200x60.txt"),
                _ => unreachable!(),
            };
            assert_eq!(snap, expected, "{columns}x{rows}");
        }
    }

    #[test]
    fn a_narrow_window_asks_for_width() {
        let mut screen = changed();
        let (grid, clusters) = render_once(&mut screen, 80, 24);
        let text = grid.display_text(&clusters);
        assert!(text.contains("Four panes need a wider window"));
        assert!(!text.contains("hello WORLD"));
        assert_eq!(
            screen.update(&Event::Key(KeyEvent::press(KeyCode::Char('q')))),
            ControlFlow::Break
        );
    }

    #[test]
    fn scroll_stays_on_the_same_row_in_every_pane() {
        let mut screen = long_file();
        let (grid, clusters) = render_once(&mut screen, 200, 40);
        let before = grid.display_text(&clusters);
        assert!(cmp_row(&before, "b79").is_none());
        for _ in 0..30 {
            screen.update(&Event::Mouse(MouseEvent::new(
                MouseKind::Scroll(Scroll::Down),
                40,
                10,
            )));
        }
        let (grid, clusters) = render_once(&mut screen, 200, 40);
        let after = grid.display_text(&clusters);
        let row = cmp_row(&after, "b79").expect("base tail");
        let line = after.lines().nth(row).unwrap();
        assert!(line.contains("o79"), "{line}");
        assert!(screen.scroll_top() > 0);
    }

    #[test]
    fn drag_and_split_keys_both_resize() {
        let mut screen = changed();
        let _ = render_once(&mut screen, 200, 40);
        let before = screen.pane_widths();
        let x = screen.divider_x(1).unwrap();
        let y = 10;
        screen.update(&Event::Mouse(MouseEvent::new(
            MouseKind::Down(MouseButton::Left),
            x,
            y,
        )));
        screen.update(&Event::Mouse(MouseEvent::new(
            MouseKind::Drag(MouseButton::Left),
            x + 10,
            y,
        )));
        screen.update(&Event::Mouse(MouseEvent::new(
            MouseKind::Up(MouseButton::Left),
            x + 10,
            y,
        )));
        let dragged = screen.pane_widths();
        assert_ne!(before, dragged);
        screen.update(&Event::Key(KeyEvent::press(KeyCode::Char(']'))));
        assert_ne!(dragged, screen.pane_widths());
    }

    #[test]
    fn n_and_p_move_hunks_and_tab_moves_focus() {
        let mut screen = two_hunks();
        let _ = render_once(&mut screen, 200, 40);
        assert_eq!(screen.selected_hunk(), 0);
        assert_eq!(screen.focused(), "hunks");
        screen.update(&Event::Key(KeyEvent::press(KeyCode::Char('n'))));
        assert_eq!(screen.selected_hunk(), 1);
        assert!(screen.scroll_top() > 0);
        screen.update(&Event::Key(KeyEvent::press(KeyCode::Char('p'))));
        assert_eq!(screen.selected_hunk(), 0);
        assert_eq!(screen.scroll_top(), 0);
        screen.update(&Event::Key(KeyEvent::press(KeyCode::Tab)));
        assert_eq!(screen.focused(), "base");
        screen.update(&Event::Key(
            KeyEvent::press(KeyCode::Tab).modifiers(Modifiers::new().shift()),
        ));
        assert_eq!(screen.focused(), "hunks");
    }

    #[test]
    fn a_conflict_shows_unresolved_in_the_result() {
        let mut screen = DiffScreen::new("same\n", "left\n", "right\n");
        let (grid, clusters) = render_once(&mut screen, 200, 40);
        let text = grid.display_text(&clusters);
        assert!(text.contains("conflict"));
        assert!(text.contains("unresolved"));
        assert!(text.contains("left"));
        assert!(text.contains("right"));
    }

    #[test]
    fn a_file_near_the_cap_stays_under_the_budget() {
        let (text, lines) = sample_near_the_cap();
        assert!(text.len() > MAX_DIFF_BYTES as usize - 128);
        assert!(text.len() <= MAX_DIFF_BYTES as usize);
        let mut screen = DiffScreen::new(&text, &text, &text);
        assert_eq!(screen.aligned.base.len(), lines);
        let (grid, clusters) = render_once(&mut screen, 200, 60);
        assert!(grid.display_text(&clusters).contains("aaa"));
        for (columns, rows) in SIZES {
            for _ in 0..4 {
                let _ = render_once(&mut screen, columns, rows);
            }
            let mut samples = Vec::new();
            for _ in 0..16 {
                let started = Instant::now();
                let _ = render_once(&mut screen, columns, rows);
                samples.push(started.elapsed());
            }
            let worst = samples.iter().copied().max().unwrap();
            assert!(
                worst < Duration::from_millis(8),
                "{columns}x{rows} worst {worst:?}"
            );
        }
    }

    fn fg_colors(snap: &str, row: usize) -> Vec<String> {
        let mut colors = Vec::new();
        let section = snap.split("@fg\n").nth(1).unwrap_or("");
        let section = section.split("@bg\n").next().unwrap_or("");
        for line in section.lines() {
            let Some((at, color)) = line.split_once(' ') else {
                continue;
            };
            let Some((got, _)) = at.split_once(':') else {
                continue;
            };
            if got == row.to_string() && !colors.iter().any(|seen| seen == color) {
                colors.push(color.to_string());
            }
        }
        colors
    }
}
