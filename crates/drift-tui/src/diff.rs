use std::ops::Range;

use drift_core::{merge3, DiffContent, DiffRows, FileDiff, Hunk, HunkKind, Mark, Merge, Row, Span};
use lunatui::{App, ControlFlow};
use lunatui_core::{clip_display_prefix, Attrs, Color, Grid, Rect, Style, CLUSTER_TAG};
use lunatui_runtime::keymap::{Command, KeyChord, Keymap};
use lunatui_term::{Event, KeyCode, Modifiers, MouseButton, MouseEvent, MouseKind, Scroll};
use lunatui_theme::{Theme, Token, Variant};
use lunatui_widgets::{
    CodeHighlight, CodeMark, CodeView, CodeViewState, DividerId, KeyHintBar, RenderCx, SplitAxis,
    SplitDrag, SplitLayout, SplitTree, Widget, WidgetState,
};
use unicode_width::UnicodeWidthChar;

const WIDE: u16 = 160;
const MEDIUM: u16 = 100;
const QUIT: &str = "diff.quit";
const LAYOUT: &str = "diff.layout";
const NEXT: &str = "diff.next";
const PREVIOUS: &str = "diff.previous";
const FOCUS_NEXT: &str = "diff.focus-next";
const FOCUS_PREVIOUS: &str = "diff.focus-previous";
const WIDEN: &str = "diff.widen";
const NARROW: &str = "diff.narrow";
const DOWN: &str = "diff.down";
const UP: &str = "diff.up";
const PANES: [&str; 5] = ["hunks", "base", "ours", "theirs", "result"];
const LAYOUT_CHORDS: &[KeyChord] = &[KeyChord::new(KeyCode::Char('v'), Modifiers::new())];
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum Layout {
    Four,
    Two,
    Stacked,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum WidthClass {
    Wide,
    Medium,
    Narrow,
}

pub struct DiffScreen {
    aligned: Aligned,
    scroll: CodeViewState,
    tree: SplitTree,
    drag: SplitDrag,
    focus: usize,
    hunk: usize,
    pinned: Option<Layout>,
    class: Option<WidthClass>,
    two_way: bool,
    title: Option<String>,
    keymap: Keymap,
    columns: u16,
    rows: u16,
    generation: u64,
    stacked_scroll: Option<bool>,
    painted: Option<Painted>,
}

struct Painted {
    columns: u16,
    rows: u16,
    generation: u64,
    grid: Grid,
}

#[derive(Clone)]
struct Line {
    text: String,
    spans: Vec<Range<usize>>,
    mark: Option<CodeMark>,
    ascii: bool,
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
    stacked: Vec<String>,
    stacked_hunk_rows: Vec<usize>,
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

    pub fn without_base(ours: &str, theirs: &str) -> Self {
        let mut screen = Self::from_merge(&merge3(ours, ours, theirs));
        screen.two_way = true;
        screen
    }

    pub fn from_file_diff(diff: &FileDiff) -> Result<Self, String> {
        let text = |content: &DiffContent, what: &str| match content {
            DiffContent::Text { text, .. } => Ok(Some(text.clone())),
            DiffContent::Absent => Ok(None),
            DiffContent::Unreadable => Err(format!("{}: the {what} side is unreadable", diff.path)),
            DiffContent::HashOnly { .. } => Err(format!(
                "{}: the {what} side is not shown as text (not UTF-8 or over 1 MiB)",
                diff.path
            )),
        };
        let ours = text(&diff.ours, "live")?.unwrap_or_default();
        let theirs = text(&diff.theirs, "template")?.unwrap_or_default();
        let screen = match &diff.base {
            DiffContent::Text { text, .. } => Self::new(text, &ours, &theirs),
            DiffContent::Absent => Self::without_base(&ours, &theirs),
            DiffContent::Unreadable | DiffContent::HashOnly { .. } => {
                text(&diff.base, "base")?;
                Self::without_base(&ours, &theirs)
            }
        };
        Ok(screen.with_title(&diff.path))
    }

    pub fn with_title(mut self, title: &str) -> Self {
        self.title = Some(title.to_string());
        self
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
            pinned: None,
            class: None,
            two_way: false,
            title: None,
            keymap: keymap(),
            columns: 0,
            rows: 0,
            generation: 0,
            stacked_scroll: None,
            painted: None,
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
        self.effective() == Layout::Four
    }

    fn note_class(&mut self) {
        let class = width_class(self.columns, self.rows);
        if self.class.is_some() && self.class != Some(class) {
            self.pinned = None;
        }
        self.class = Some(class);
    }

    fn effective(&self) -> Layout {
        let auto = auto_layout(self.class.unwrap_or(WidthClass::Narrow), self.two_way);
        match self.pinned {
            Some(Layout::Four) if self.two_way => Layout::Two,
            Some(layout) => layout,
            None => auto,
        }
    }

    fn cycle_layout(&mut self) {
        self.revise();
        self.pinned = Some(next_layout(self.effective(), self.two_way));
    }

    fn header_text(&self) -> String {
        let name = match &self.title {
            Some(title) => format!("Diff  {title}"),
            None => "Diff".to_string(),
        };
        if self.two_way {
            return match self.aligned.hunk_labels.get(self.hunk) {
                Some(label) => format!("{name}  base unknown  {label}"),
                None => format!("{name}  base unknown"),
            };
        }
        if self.effective() == Layout::Four {
            return name;
        }
        match self.aligned.hunk_labels.get(self.hunk) {
            Some(label) => format!("{name}  {label}"),
            None => name,
        }
    }

    fn revise(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }

    fn scroll_by(&mut self, lines: isize) {
        self.revise();
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
        self.revise();
        let next = self.hunk as isize + delta;
        self.hunk = next.clamp(0, count as isize - 1) as usize;
        self.scroll.scroll_to(self.hunk_scroll_row());
    }

    fn stacked_unified(&self) -> bool {
        self.effective() == Layout::Stacked && !self.two_way
    }

    fn hunk_scroll_row(&self) -> usize {
        let rows = if self.stacked_unified() {
            &self.aligned.stacked_hunk_rows
        } else {
            &self.aligned.hunk_rows
        };
        rows.get(self.hunk).copied().unwrap_or(0)
    }

    fn move_focus(&mut self, delta: isize) {
        self.revise();
        let count = PANES.len() as isize;
        self.focus = (self.focus as isize + delta).rem_euclid(count) as usize;
    }

    fn resize_focused(&mut self, delta: i16) {
        if !self.wide() {
            return;
        }
        self.revise();
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
    map.register(Command::new(LAYOUT, "Layout", LAYOUT_CHORDS));
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

fn width_class(columns: u16, rows: u16) -> WidthClass {
    if rows <= 2 {
        return WidthClass::Narrow;
    }
    if columns >= WIDE {
        WidthClass::Wide
    } else if columns >= MEDIUM {
        WidthClass::Medium
    } else {
        WidthClass::Narrow
    }
}

fn auto_layout(class: WidthClass, two_way: bool) -> Layout {
    if two_way {
        return if class == WidthClass::Narrow {
            Layout::Stacked
        } else {
            Layout::Two
        };
    }
    match class {
        WidthClass::Wide => Layout::Four,
        WidthClass::Medium => Layout::Two,
        WidthClass::Narrow => Layout::Stacked,
    }
}

fn next_layout(current: Layout, two_way: bool) -> Layout {
    if two_way {
        return match current {
            Layout::Stacked => Layout::Two,
            Layout::Two | Layout::Four => Layout::Stacked,
        };
    }
    match current {
        Layout::Four => Layout::Two,
        Layout::Two => Layout::Stacked,
        Layout::Stacked => Layout::Four,
    }
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
        stacked: Vec::new(),
        stacked_hunk_rows: Vec::new(),
    };
    let mut hunk_index = 0;
    for segment in merge.segments() {
        match segment {
            drift_core::Segment::Stable(text) => {
                for line in split_lines(text) {
                    aligned.stacked.push(format!("  {line}"));
                    let shown = if at < rows.len() && rows[at].hunk.is_none() {
                        let text = rows[at].text.clone();
                        at += 1;
                        text
                    } else {
                        line
                    };
                    let plain = owned_line(shown, Vec::new(), None);
                    aligned.base.push(plain.clone());
                    aligned.ours.push(plain.clone());
                    aligned.theirs.push(plain.clone());
                    aligned.result.push(plain);
                }
            }
            drift_core::Segment::Hunk(hunk) => {
                push_stacked(&mut aligned, hunk);
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
            let result = vec![owned_line(
                "unresolved".to_string(),
                Vec::new(),
                Some(CodeMark::Error),
            )];
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
            ascii: line.ascii,
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
    owned_line(row.text.clone(), spans, mark)
}

fn raw_lines(text: &str) -> Vec<Line> {
    split_lines(text)
        .into_iter()
        .map(|text| owned_line(text, Vec::new(), None))
        .collect()
}

fn extend(into: &mut Vec<Line>, mut lines: Vec<Line>, height: usize) {
    while lines.len() < height {
        lines.push(owned_line(String::new(), Vec::new(), None));
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

fn owned_line(text: String, spans: Vec<Range<usize>>, mark: Option<CodeMark>) -> Line {
    let ascii = ascii_line(&text);
    Line {
        text,
        spans,
        mark,
        ascii,
    }
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

fn split_strip(body: Rect) -> (Rect, Rect) {
    let strip_height = if body.height <= 4 {
        1.min(body.height)
    } else {
        (body.height / 3).clamp(3, body.height - 3)
    };
    let top_height = body.height.saturating_sub(strip_height);
    let top = Rect::new(body.x, body.y, body.width, top_height);
    let strip = Rect::new(
        body.x,
        body.y.saturating_add(top_height),
        body.width,
        strip_height,
    );
    (top, strip)
}

fn split_pair(area: Rect) -> (Rect, Rect) {
    if area.width < 3 {
        return (
            area,
            Rect::new(area.x.saturating_add(area.width), area.y, 0, area.height),
        );
    }
    let left_width = (area.width - 1) / 2;
    let left = Rect::new(area.x, area.y, left_width, area.height);
    let right = Rect::new(
        area.x.saturating_add(left_width).saturating_add(1),
        area.y,
        area.width - left_width - 1,
        area.height,
    );
    (left, right)
}

fn push_stacked(aligned: &mut Aligned, hunk: &Hunk) {
    aligned.stacked_hunk_rows.push(aligned.stacked.len());
    match hunk.kind {
        HunkKind::Conflict => {
            push_marked(&mut aligned.stacked, "- ", &hunk.base);
            push_marked(&mut aligned.stacked, "< ours ", &hunk.ours);
            push_marked(&mut aligned.stacked, "> theirs ", &hunk.theirs);
        }
        HunkKind::TheirsOnly => {
            push_marked(&mut aligned.stacked, "- ", &hunk.base);
            push_marked(&mut aligned.stacked, "+ ", &hunk.theirs);
        }
        HunkKind::OursOnly | HunkKind::BothSame => {
            push_marked(&mut aligned.stacked, "- ", &hunk.base);
            push_marked(&mut aligned.stacked, "+ ", &hunk.ours);
        }
    }
}

fn push_marked(into: &mut Vec<String>, mark: &str, text: &str) {
    for line in split_lines(text) {
        into.push(format!("{mark}{line}"));
    }
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
            Some(LAYOUT) => {
                self.cycle_layout();
                ControlFlow::Continue
            }
            _ => ControlFlow::Continue,
        }
    }

    fn render(&mut self, grid: &mut Grid, area: Rect, clusters: &mut lunatui_core::ClusterTable) {
        self.columns = area.width;
        self.rows = area.height;
        self.note_class();
        if let Some(painted) = &self.painted {
            if painted.columns == area.width
                && painted.rows == area.height
                && painted.generation == self.generation
            {
                grid.blit(&painted.grid, area, area.x, area.y);
                return;
            }
        }
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
            let title = self.header_text();
            cx.put_str(
                header.x,
                header.y,
                clip_display_prefix(&title, header.width),
                accent,
            );
        }
        if body.width > 0 && body.height > 0 {
            match self.effective() {
                Layout::Four => self.render_wide(body, &mut cx, accent, text),
                Layout::Two => self.render_two(body, &mut cx, accent, text),
                Layout::Stacked => self.render_stacked(body, &mut cx, accent, text),
            }
        }
        if hint.width > 0 && hint.height > 0 {
            KeyHintBar::new(&self.keymap).render(hint, &mut cx);
        }
        if inline_only(grid) {
            self.painted = Some(Painted {
                columns: area.width,
                rows: area.height,
                generation: self.generation,
                grid: grid.clone(),
            });
        } else {
            self.painted = None;
        }
    }
}

impl DiffScreen {
    fn pointer(&mut self, mouse: &MouseEvent) {
        let body = self.body();
        let layout = self.tree.layout(body);
        match mouse.kind {
            MouseKind::Down(MouseButton::Left) => {
                self.revise();
                self.drag
                    .press(&self.tree, &layout, mouse.column, mouse.row);
            }
            MouseKind::Drag(MouseButton::Left) => {
                self.revise();
                self.drag.drag(&mut self.tree, mouse.column, mouse.row);
            }
            MouseKind::Up(MouseButton::Left) => {
                self.revise();
                self.drag.release();
            }
            MouseKind::Scroll(Scroll::Down) => self.scroll_by(3),
            MouseKind::Scroll(Scroll::Up) => self.scroll_by(-3),
            _ => {}
        }
    }

    fn render_wide(&mut self, body: Rect, cx: &mut RenderCx<'_>, accent: Style, text: Style) {
        let scroll = self.prepare_scroll(body.height.saturating_sub(1));
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
        paint_dividers(&layout, text, accent, active, cx);
    }

    fn render_two(&mut self, body: Rect, cx: &mut RenderCx<'_>, accent: Style, text: Style) {
        let (top, strip) = split_strip(body);
        let scroll = self.prepare_scroll(top.height.saturating_sub(1));
        let (left, right) = split_pair(top);
        let ours = if self.focused() == "ours" {
            accent
        } else {
            text
        };
        let theirs = if self.focused() == "theirs" {
            accent
        } else {
            text
        };
        self.render_code(left, "ours", &self.aligned.ours, ours, scroll, cx);
        self.render_code(right, "theirs", &self.aligned.theirs, theirs, scroll, cx);
        if top.width > 2 && top.height > 0 {
            let x = left.x.saturating_add(left.width);
            cx.fill(Rect::new(x, top.y, 1, top.height), text, '│');
        }
        if self.two_way || strip.height == 0 {
            return;
        }
        let result = if self.focused() == "result" {
            accent
        } else {
            text
        };
        self.render_code(strip, "result", &self.aligned.result, result, scroll, cx);
    }

    fn render_stacked(&mut self, body: Rect, cx: &mut RenderCx<'_>, accent: Style, text: Style) {
        if self.two_way {
            let top_height = (body.height / 2).max(1).min(body.height);
            let top = Rect::new(body.x, body.y, body.width, top_height);
            let bottom = Rect::new(
                body.x,
                body.y.saturating_add(top_height),
                body.width,
                body.height.saturating_sub(top_height),
            );
            let scroll = self.prepare_scroll(top.height.saturating_sub(1));
            let ours = if self.focused() == "ours" {
                accent
            } else {
                text
            };
            let theirs = if self.focused() == "theirs" {
                accent
            } else {
                text
            };
            self.render_code(top, "ours", &self.aligned.ours, ours, scroll, cx);
            self.render_code(bottom, "theirs", &self.aligned.theirs, theirs, scroll, cx);
            return;
        }
        cx.put_str(
            body.x,
            body.y,
            clip_display_prefix("stacked", body.width),
            accent,
        );
        if body.height <= 1 {
            return;
        }
        let view = Rect::new(body.x, body.y + 1, body.width, body.height - 1);
        let scroll = self.prepare_scroll(view.height);
        let top = scroll.top();
        let rows = usize::from(view.height);
        for (offset, shown) in self.aligned.stacked.iter().skip(top).take(rows).enumerate() {
            let y = view.y.saturating_add(u16::try_from(offset).unwrap_or(0));
            cx.put_str(view.x, y, clip_display_prefix(shown, view.width), text);
        }
    }

    fn prepare_scroll(&mut self, rows: u16) -> CodeViewState {
        let mut scroll = self.scroll;
        let area = Rect::new(0, 0, 8, rows);
        let stacked = self.stacked_unified();
        if stacked {
            CodeView::new(self.aligned.stacked.as_slice()).sync(area, &mut scroll);
        } else {
            CodeView::new(self.aligned.base.as_slice()).sync(area, &mut scroll);
        }
        if self.stacked_scroll != Some(stacked) {
            scroll.scroll_to(self.hunk_scroll_row());
            self.stacked_scroll = Some(stacked);
        }
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
        let fast = scroll.left() == 0 && lines.iter().skip(top).take(rows).all(|line| line.ascii);
        if fast {
            paint_ascii(view_area, lines, top, rows, cx);
            return;
        }
        let mut marks = Vec::new();
        for (offset, line) in lines.iter().skip(top).take(rows).enumerate() {
            if let Some(mark) = line.mark {
                marks.push((top + offset, mark));
            }
        }
        marks.push((usize::MAX, CodeMark::Changed));
        let state = if self.focused() == title {
            WidgetState::Focused
        } else {
            WidgetState::Normal
        };
        let hits = Hits { lines };
        CodeView::new(lines)
            .view(&scroll)
            .marks(&marks)
            .highlight(&hits)
            .state(state)
            .render(view_area, cx);
    }
}

struct MarkRun {
    x: u16,
    y: u16,
    height: u16,
    glyph: char,
    style: Style,
}

impl MarkRun {
    fn new() -> Self {
        Self {
            x: 0,
            y: 0,
            height: 0,
            glyph: ' ',
            style: Style::new(),
        }
    }

    fn push(&mut self, cx: &mut RenderCx<'_>, x: u16, y: u16, glyph: char, style: Style) {
        if self.height > 0
            && self.x == x
            && self.glyph == glyph
            && self.style == style
            && self.y.saturating_add(self.height) == y
        {
            self.height = self.height.saturating_add(1);
            return;
        }
        self.flush(cx);
        self.x = x;
        self.y = y;
        self.height = 1;
        self.glyph = glyph;
        self.style = style;
    }

    fn flush(&mut self, cx: &mut RenderCx<'_>) {
        if self.height == 0 {
            return;
        }
        cx.fill(
            Rect::new(self.x, self.y, 1, self.height),
            self.style,
            self.glyph,
        );
        self.height = 0;
    }
}

struct DimRun {
    x: u16,
    y: u16,
    width: u16,
    height: u16,
}

impl DimRun {
    fn new() -> Self {
        Self {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
        }
    }

    fn push(&mut self, cx: &mut RenderCx<'_>, style: Style, x: u16, y: u16, width: u16) {
        if width == 0 {
            return;
        }
        if self.height > 0
            && self.x == x
            && self.width == width
            && self.y.saturating_add(self.height) == y
        {
            self.height = self.height.saturating_add(1);
            return;
        }
        self.flush(cx, style);
        self.x = x;
        self.y = y;
        self.width = width;
        self.height = 1;
    }

    fn flush(&mut self, cx: &mut RenderCx<'_>, style: Style) {
        if self.height == 0 || self.width == 0 {
            return;
        }
        cx.set_style(Rect::new(self.x, self.y, self.width, self.height), style);
        self.height = 0;
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
    let text_x = view_area.x.saturating_add(gutter.min(view_area.width));
    let limit = usize::from(text_width);
    let shown_rows = lines.len().saturating_sub(top).min(rows);
    let repeated = repeated_byte(lines, top, shown_rows, limit);
    if view_area.width > 0 && view_area.height > 0 {
        if repeated.is_some() {
            let gutter_width = gutter.min(view_area.width);
            if gutter_width > 0 {
                cx.fill(
                    Rect::new(view_area.x, view_area.y, gutter_width, view_area.height),
                    plain,
                    ' ',
                );
            }
            let rest = rows.saturating_sub(shown_rows);
            if rest > 0 && text_width > 0 {
                let y = view_area
                    .y
                    .saturating_add(u16::try_from(shown_rows).unwrap_or(0));
                let height = u16::try_from(rest).unwrap_or(0);
                cx.fill(Rect::new(text_x, y, text_width, height), plain, ' ');
            }
        } else {
            cx.fill(view_area, plain, ' ');
        }
    }
    let mut dims = DimRun::new();
    let mut marks = MarkRun::new();
    for (offset, line) in lines.iter().skip(top).take(rows).enumerate() {
        let y = view_area
            .y
            .saturating_add(u16::try_from(offset).unwrap_or(0));
        if let Some(mark) = line.mark {
            let (glyph, style) = mark_paint(mark, cx);
            if view_area.width > 0 {
                marks.push(cx, view_area.x, y, glyph, style);
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
            cx.put_str(x, y, shown, plain);
            dims.push(cx, numbers, x, y, u16::try_from(shown.len()).unwrap_or(0));
        }
        if repeated.is_some() || limit == 0 || line.text.is_empty() {
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
    marks.flush(cx);
    dims.flush(cx, numbers);
    if let Some(byte) = repeated {
        let height = u16::try_from(shown_rows).unwrap_or(0);
        if text_width > 0 && height > 0 {
            cx.fill(
                Rect::new(text_x, view_area.y, text_width, height),
                plain,
                char::from(byte),
            );
        }
    }
    if bar == 1 {
        paint_bar(
            Rect::new(
                view_area.x + view_area.width - 1,
                view_area.y,
                1,
                view_area.height,
            ),
            lines.len(),
            rows,
            top,
            cx,
        );
    }
}

fn paint_dividers(
    layout: &SplitLayout,
    text: Style,
    accent: Style,
    active: Option<DividerId>,
    cx: &mut RenderCx<'_>,
) {
    for divider in &layout.dividers {
        let on = active == Some(divider.id);
        let (glyph, style) = match (divider.axis, on) {
            (SplitAxis::Horizontal, false) => ('│', text),
            (SplitAxis::Horizontal, true) => ('┃', accent),
            (SplitAxis::Vertical, false) => ('─', text),
            (SplitAxis::Vertical, true) => ('━', accent),
        };
        if divider.rect.width > 0 && divider.rect.height > 0 {
            cx.fill(divider.rect, style, glyph);
        }
    }
}

fn paint_bar(area: Rect, content: usize, viewport: usize, offset: usize, cx: &mut RenderCx<'_>) {
    if area.width == 0 || area.height == 0 || content == 0 {
        return;
    }
    let track = usize::from(area.height);
    cx.fill(Rect::new(area.x, area.y, 1, area.height), Style::new(), '│');
    let thumb_len = thumb_size(content, track, viewport);
    let thumb_at = thumb_position(offset, content, viewport, track, thumb_len);
    if thumb_len > 0 {
        cx.fill(
            Rect::new(area.x, area.y.saturating_add(thumb_at), 1, thumb_len),
            Style::new(),
            '█',
        );
    }
}

fn repeated_byte(lines: &[Line], top: usize, rows: usize, limit: usize) -> Option<u8> {
    if limit == 0 || rows == 0 {
        return None;
    }
    let mut matched = None;
    let mut seen = 0usize;
    for line in lines.iter().skip(top).take(rows) {
        if line.mark.is_some() || !line.spans.is_empty() {
            return None;
        }
        let bytes = line.text.as_bytes();
        if bytes.len() < limit {
            return None;
        }
        let sample = bytes[0];
        if bytes[..limit].iter().any(|byte| *byte != sample) {
            return None;
        }
        if let Some(previous) = matched {
            if previous != sample {
                return None;
            }
        } else {
            matched = Some(sample);
        }
        seen += 1;
    }
    if seen == 0 {
        None
    } else {
        matched
    }
}

fn thumb_size(content: usize, track: usize, viewport: usize) -> u16 {
    if content == 0 {
        return 1;
    }
    let max_thumb = track.max(1);
    let proportional = track.saturating_mul(viewport).div_ceil(content).max(1);
    u16::try_from(proportional.min(max_thumb)).unwrap_or(1)
}

fn thumb_position(offset: usize, content: usize, viewport: usize, track: usize, thumb: u16) -> u16 {
    if content <= viewport {
        return 0;
    }
    let scrollable = content.saturating_sub(viewport);
    let max_y = track.saturating_sub(usize::from(thumb));
    if scrollable == 0 {
        return 0;
    }
    let offset = offset.min(scrollable);
    if offset >= scrollable {
        return u16::try_from(max_y).unwrap_or(0);
    }
    let y = (offset as u128) * (max_y as u128) / (scrollable as u128);
    u16::try_from(usize::try_from(y).unwrap_or(max_y)).unwrap_or(0)
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

fn mark_paint(mark: CodeMark, cx: &RenderCx<'_>) -> (char, Style) {
    match mark {
        CodeMark::Error => ('●', Style::new().fg(cx.token(Token::Fail))),
        _ => ('▎', Style::new().fg(cx.token(Token::Warn))),
    }
}

fn inline_only(grid: &Grid) -> bool {
    for row in 0..grid.height() {
        for word in grid.row_bits(row) {
            if (*word as u32) & CLUSTER_TAG != 0 {
                return false;
            }
        }
    }
    true
}

fn mouse_quits(columns: u16, rows: u16, mouse: &MouseEvent) -> bool {
    if !matches!(mouse.kind, MouseKind::Down(MouseButton::Left)) {
        return false;
    }
    crate::quit_bounds(columns, rows).is_some_and(|hit| contains(hit, mouse.column, mouse.row))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use drift_core::MAX_DIFF_BYTES;
    use drift_fixtures::{busy_for, cpu_cost};
    use lunatui::render_once;
    use lunatui_term::KeyEvent;
    use lunatui_test::styled_snapshot;

    use super::*;

    fn timed_frame(
        screen: &mut DiffScreen,
        columns: u16,
        rows: u16,
    ) -> (Duration, lunatui_core::Grid, lunatui_core::ClusterTable) {
        let mut rendered = None;
        let took = cpu_cost(|| {
            rendered = Some(render_once(screen, columns, rows));
        });
        let (grid, clusters) = rendered.expect("frame");
        (took, grid, clusters)
    }

    fn sample_near_the_cap() -> (String, usize) {
        let line = format!("{}\n", "a".repeat(63));
        let count = (MAX_DIFF_BYTES as usize - 1) / line.len();
        (line.repeat(count), count)
    }

    fn changed_near_the_cap(text: &str) -> DiffScreen {
        let mut ours = text.as_bytes().to_vec();
        let line = 64;
        for row in 0..48 {
            ours[row * line] = b'B';
        }
        ours[120 * line] = b'C';
        let ours = String::from_utf8(ours).unwrap();
        let screen = DiffScreen::new(text, &ours, text);
        assert!(screen.aligned.hunk_rows.len() >= 2);
        screen
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
    fn snapshots_show_narrower_layouts() {
        for (columns, rows) in [(80_u16, 24_u16), (100, 30), (120, 36)] {
            let mut screen = changed();
            let (grid, clusters) = render_once(&mut screen, columns, rows);
            let text = grid.display_text(&clusters);
            assert!(text.lines().next().unwrap().contains("Diff"), "{columns}");
            match (columns, rows) {
                (80, 24) => {
                    assert!(text.contains("stacked"), "{columns}");
                    assert!(text.contains("hello WORLD"), "{columns}");
                }
                (100, 30) | (120, 36) => {
                    let body = text.lines().nth(1).unwrap();
                    assert!(
                        body.contains("ours") && body.contains("theirs"),
                        "{columns}"
                    );
                    assert!(text.contains("result"), "{columns}");
                    assert!(text.contains("hello WORLD"), "{columns}");
                }
                _ => unreachable!(),
            }
            let snap = styled_snapshot(&grid, &clusters);
            let expected = match (columns, rows) {
                (80, 24) => include_str!("../snapshots/diff-80x24.txt"),
                (100, 30) => include_str!("../snapshots/diff-100x30.txt"),
                (120, 36) => include_str!("../snapshots/diff-120x36.txt"),
                _ => unreachable!(),
            };
            assert_eq!(snap, expected, "{columns}x{rows}");
        }
    }

    #[test]
    fn a_narrow_window_stacks_the_change() {
        let mut screen = changed();
        let (grid, clusters) = render_once(&mut screen, 80, 24);
        let text = grid.display_text(&clusters);
        assert!(text.contains("stacked"), "{text}");
        assert!(text.contains("- hello world"), "{text}");
        assert!(text.contains("+ hello WORLD"), "{text}");
        assert_eq!(
            screen.update(&Event::Key(KeyEvent::press(KeyCode::Char('q')))),
            ControlFlow::Break
        );
    }

    fn frame(screen: &mut DiffScreen) -> String {
        let (grid, clusters) = render_once(screen, 80, 24);
        grid.display_text(&clusters)
    }

    fn stacked_body(text: &str) -> Vec<&str> {
        let lines: Vec<&str> = text.lines().collect();
        if lines.len() < 3 {
            return Vec::new();
        }
        lines[2..lines.len() - 1].to_vec()
    }

    #[test]
    fn stacked_keeps_a_conflict_a_replacement_and_a_deletion() {
        let mut conflict = DiffScreen::new("BASE_OLD\n", "OURS_NEW\n", "THEIRS_NEW\n");
        let text = frame(&mut conflict);
        assert!(text.contains("conflict"), "{text}");
        let body = stacked_body(&text);
        assert!(
            body.iter().any(|line| line.contains("- BASE_OLD")),
            "{text}"
        );
        assert!(
            body.iter().any(|line| line.contains("< ours OURS_NEW")),
            "{text}"
        );
        assert!(
            body.iter().any(|line| line.contains("> theirs THEIRS_NEW")),
            "{text}"
        );

        let mut replacement = DiffScreen::new("BASE_OLD\n", "BASE_OLD\n", "THEIRS_NEW\n");
        let text = frame(&mut replacement);
        let body = stacked_body(&text);
        assert!(
            body.iter().any(|line| line.contains("- BASE_OLD")),
            "{text}"
        );
        assert!(
            body.iter().any(|line| line.contains("+ THEIRS_NEW")),
            "{text}"
        );

        let mut deletion = DiffScreen::new("BASE_OLD\n", "BASE_OLD\n", "");
        let text = frame(&mut deletion);
        let body = stacked_body(&text);
        assert!(
            body.iter().any(|line| line.contains("- BASE_OLD")),
            "{text}"
        );
        assert!(body.iter().all(|line| !line.contains("+ ")), "{text}");
    }

    #[test]
    fn stacked_navigation_and_scrolling_follow_expanded_rows() {
        let mut base = String::from("alpha\n");
        let mut ours = String::from("ALPHA\n");
        let mut theirs = String::from("alpha\n");
        for index in 0..30 {
            let line = format!("c{index:02}\n");
            base.push_str(&line);
            ours.push_str(&line);
            theirs.push_str(&line);
        }
        base.push_str("old\n");
        ours.push_str("old\n");
        theirs.push_str("new\n");
        for index in 0..25 {
            let line = format!("d{index:02}\n");
            base.push_str(&line);
            ours.push_str(&line);
            theirs.push_str(&line);
        }
        let mut screen = DiffScreen::new(&base, &ours, &theirs);
        let text = frame(&mut screen);
        assert!(text.contains("- alpha"), "{text}");
        assert!(text.contains("+ ALPHA"), "{text}");
        assert!(!text.contains("+ new"), "{text}");
        screen.update(&Event::Key(KeyEvent::press(KeyCode::Char('n'))));
        let jumped = frame(&mut screen);
        let body = stacked_body(&jumped);
        assert!(
            body.first().is_some_and(|line| line.contains("- old")),
            "{jumped}"
        );
        assert!(jumped.contains("+ new"), "{jumped}");
        let top = screen.scroll_top();
        screen.update(&Event::Key(KeyEvent::press(KeyCode::Down)));
        let scrolled = frame(&mut screen);
        let next = stacked_body(&scrolled);
        assert!(screen.scroll_top() > top);
        assert!(
            next.first().is_some_and(|line| line.contains("+ new")),
            "{scrolled}"
        );
        screen.update(&Event::Key(KeyEvent::press(KeyCode::Char('p'))));
        let back = frame(&mut screen);
        let home = stacked_body(&back);
        assert_eq!(screen.scroll_top(), 0);
        assert!(
            home.first().is_some_and(|line| line.contains("- alpha")),
            "{back}"
        );
    }

    #[test]
    fn width_picks_the_layout_and_keeps_the_hunk() {
        let mut screen = two_hunks();
        let _ = render_once(&mut screen, 160, 40);
        screen.update(&Event::Key(KeyEvent::press(KeyCode::Char('n'))));
        assert_eq!(screen.selected_hunk(), 1);
        let (grid, clusters) = render_once(&mut screen, 120, 36);
        let medium = grid.display_text(&clusters);
        assert_eq!(screen.selected_hunk(), 1);
        assert!(medium.contains("ours"), "{medium}");
        assert!(medium.contains("theirs"), "{medium}");
        assert!(medium.contains("result"), "{medium}");
        assert!(!medium.lines().nth(1).unwrap_or("").contains("base"));
        let (grid, clusters) = render_once(&mut screen, 100, 30);
        let hundred = grid.display_text(&clusters);
        assert!(hundred.contains("result"), "{hundred}");
        assert_eq!(screen.selected_hunk(), 1);
        let (grid, clusters) = render_once(&mut screen, 80, 24);
        let narrow = grid.display_text(&clusters);
        assert!(narrow.contains("stacked"), "{narrow}");
        assert_eq!(screen.selected_hunk(), 1);
    }

    #[test]
    fn v_cycles_layouts_until_the_width_class_changes() {
        let mut screen = changed();
        let (grid, clusters) = render_once(&mut screen, 160, 40);
        let wide = grid.display_text(&clusters);
        assert!(wide.contains("hunks"), "{wide}");
        screen.update(&Event::Key(KeyEvent::press(KeyCode::Char('v'))));
        let (grid, clusters) = render_once(&mut screen, 160, 40);
        let two = grid.display_text(&clusters);
        assert!(two.contains("ours") && two.contains("result"), "{two}");
        assert!(!two.lines().nth(1).unwrap_or("").contains("hunks"));
        screen.update(&Event::Key(KeyEvent::press(KeyCode::Char('v'))));
        let (grid, clusters) = render_once(&mut screen, 160, 40);
        assert!(grid.display_text(&clusters).contains("stacked"));
        screen.update(&Event::Key(KeyEvent::press(KeyCode::Char('v'))));
        let (grid, clusters) = render_once(&mut screen, 180, 40);
        assert!(grid.display_text(&clusters).contains("hunks"));
        screen.update(&Event::Key(KeyEvent::press(KeyCode::Char('v'))));
        let (grid, clusters) = render_once(&mut screen, 168, 40);
        assert!(grid.display_text(&clusters).contains("result"));
        let (grid, clusters) = render_once(&mut screen, 80, 24);
        assert!(grid.display_text(&clusters).contains("stacked"));
    }

    #[test]
    fn an_unknown_base_shows_two_panes_and_a_note() {
        let mut screen = DiffScreen::without_base("alpha\nhello\n", "alpha\nHELLO\n");
        let (grid, clusters) = render_once(&mut screen, 160, 40);
        let text = grid.display_text(&clusters);
        assert!(text.contains("base unknown"), "{text}");
        assert!(text.contains("ours"), "{text}");
        assert!(text.contains("theirs"), "{text}");
        assert!(text.contains("HELLO"), "{text}");
        let body = text.lines().nth(1).unwrap_or("");
        assert!(!body.contains("result"), "{body}");
        assert!(!body.contains("hunks"), "{body}");
        let (grid, clusters) = render_once(&mut screen, 80, 24);
        let narrow = grid.display_text(&clusters);
        assert!(narrow.contains("base unknown"), "{narrow}");
        assert!(narrow.contains("ours"), "{narrow}");
        assert!(narrow.contains("theirs"), "{narrow}");
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
    fn a_second_frame_matches_the_first() {
        let mut text = String::new();
        for index in 0..80 {
            text.push_str(&format!("same{index:02}\n"));
        }
        let mut screen = DiffScreen::new(&text, &text, &text);
        let (first, first_clusters) = render_once(&mut screen, 200, 40);
        let (second, second_clusters) = render_once(&mut screen, 200, 40);
        assert_eq!(
            styled_snapshot(&first, &first_clusters),
            styled_snapshot(&second, &second_clusters)
        );
        screen.update(&Event::Key(KeyEvent::press(KeyCode::Down)));
        let (scrolled, scrolled_clusters) = render_once(&mut screen, 200, 40);
        assert_ne!(
            first.display_text(&first_clusters),
            scrolled.display_text(&scrolled_clusters)
        );
        let mut marked = changed();
        let (once, once_clusters) = render_once(&mut marked, 160, 40);
        let (again, again_clusters) = render_once(&mut marked, 160, 40);
        assert_eq!(
            styled_snapshot(&once, &once_clusters),
            styled_snapshot(&again, &again_clusters)
        );
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
        let mut changed = changed_near_the_cap(&text);
        changed.update(&Event::Key(KeyEvent::press(KeyCode::Down)));
        let (scrolled, grid, clusters) = timed_frame(&mut changed, 160, 40);
        assert!(grid.display_text(&clusters).contains('B'));
        changed.update(&Event::Key(KeyEvent::press(KeyCode::Tab)));
        let (focused, _, _) = timed_frame(&mut changed, 160, 40);
        changed.update(&Event::Key(KeyEvent::press(KeyCode::Char('n'))));
        let (jumped, grid, clusters) = timed_frame(&mut changed, 160, 40);
        assert!(grid.display_text(&clusters).contains('C'));
        assert!(changed.scroll_top() > 0);
        let worst_cold = scrolled.max(focused).max(jumped);
        assert!(
            worst_cold < Duration::from_millis(8),
            "160x40 cold changed worst {worst_cold:?} scroll {scrolled:?} focus {focused:?} hunk {jumped:?}"
        );
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
                samples.push(cpu_cost(|| {
                    let _ = render_once(&mut screen, columns, rows);
                }));
            }
            let worst = samples.iter().copied().max().unwrap();
            assert!(
                worst < Duration::from_millis(8),
                "{columns}x{rows} worst {worst:?}"
            );
        }
    }

    #[test]
    fn the_cpu_clock_ignores_waiting_and_counts_work() {
        let waiting = cpu_cost(|| std::thread::sleep(Duration::from_millis(40)));
        assert!(waiting < Duration::from_millis(5), "{waiting:?}");
        let working = cpu_cost(|| busy_for(Duration::from_millis(30)));
        assert!(working >= Duration::from_millis(20), "{working:?}");
    }

    #[test]
    fn a_render_that_burns_cpu_over_the_budget_is_caught() {
        let mut screen = DiffScreen::new("same\n", "same\n", "same\n");
        let slow = cpu_cost(|| {
            busy_for(Duration::from_millis(12));
            let _ = render_once(&mut screen, 160, 40);
        });
        assert!(slow >= Duration::from_millis(8), "{slow:?}");
    }

    #[test]
    fn column_spans_follow_wide_characters() {
        assert_eq!(columns_to_bytes("abc", Span { start: 1, end: 1 }), None);
        assert_eq!(
            columns_to_bytes("abc", Span { start: 1, end: 3 }),
            Some(1..3)
        );
        assert_eq!(columns_to_bytes("abc", Span { start: 1, end: 8 }), None);
        let text = "a你b";
        let got = columns_to_bytes(text, Span { start: 1, end: 3 }).unwrap();
        assert_eq!(&text[got], "你");
        assert_eq!(
            columns_to_bytes("a你", Span { start: 0, end: 1 }),
            Some(0..1)
        );
        let mixed = "e\u{0301}x";
        let got = columns_to_bytes(mixed, Span { start: 0, end: 2 }).unwrap();
        assert_eq!(&mixed[got], mixed);
        assert_eq!(columns_to_bytes("你", Span { start: 2, end: 4 }), None);
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

    fn content(text: &str) -> DiffContent {
        DiffContent::Text {
            sha256: "ab".repeat(32),
            text: text.to_string(),
        }
    }

    fn file_diff(base: DiffContent, ours: DiffContent, theirs: DiffContent) -> FileDiff {
        FileDiff {
            path: ".config/hypr/a.lua".to_string(),
            status: drift_core::Status::Conflict,
            badge: None,
            provenance: drift_core::BaseProvenance::Reviewed,
            note: String::new(),
            base,
            ours,
            theirs,
        }
    }

    #[test]
    fn a_file_diff_opens_with_its_path_in_the_header_and_all_three_sides() {
        let mut screen = DiffScreen::from_file_diff(&file_diff(
            content("alpha\nhello\n"),
            content("alpha\nHELLO\n"),
            content("alpha\nhola\n"),
        ))
        .unwrap();
        let (grid, clusters) = render_once(&mut screen, 160, 40);
        let text = grid.display_text(&clusters);
        assert!(
            text.lines()
                .next()
                .unwrap()
                .contains("Diff  .config/hypr/a.lua"),
            "{text}"
        );
        for needle in ["hello", "HELLO", "hola"] {
            assert!(text.contains(needle), "{needle}\n{text}");
        }
    }

    #[test]
    fn an_absent_base_is_two_way_and_an_absent_live_file_diffs_against_nothing() {
        let mut two = DiffScreen::from_file_diff(&file_diff(
            DiffContent::Absent,
            content("mine\n"),
            content("theirs\n"),
        ))
        .unwrap();
        let (grid, clusters) = render_once(&mut two, 160, 40);
        assert!(grid.display_text(&clusters).contains("base unknown"));
        let mut missing = DiffScreen::from_file_diff(&file_diff(
            DiffContent::Absent,
            DiffContent::Absent,
            content("new file\n"),
        ))
        .unwrap();
        let (grid, clusters) = render_once(&mut missing, 160, 40);
        assert!(grid.display_text(&clusters).contains("new file"));
    }

    #[test]
    fn sides_that_are_not_text_are_refused_with_the_side_and_the_path() {
        let hashed = DiffContent::HashOnly {
            sha256: "cd".repeat(32),
            len: 3,
            reason: drift_core::HashOnlyReason::NonUtf8,
        };
        let error = DiffScreen::from_file_diff(&file_diff(
            DiffContent::Absent,
            hashed.clone(),
            content("t\n"),
        ))
        .err()
        .unwrap();
        assert_eq!(
            error,
            ".config/hypr/a.lua: the live side is not shown as text (not UTF-8 or over 1 MiB)"
        );
        let error = DiffScreen::from_file_diff(&file_diff(
            DiffContent::Absent,
            content("o\n"),
            DiffContent::Unreadable,
        ))
        .err()
        .unwrap();
        assert_eq!(error, ".config/hypr/a.lua: the template side is unreadable");
        let error = DiffScreen::from_file_diff(&file_diff(hashed, content("o\n"), content("t\n")))
            .err()
            .unwrap();
        assert!(error.contains("the base side"), "{error}");
    }
}
