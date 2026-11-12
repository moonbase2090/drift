use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};

use drift_core::{
    merge3, Action, Choice, DiffRows, FileContents, Hunk, HunkKind, Mark, Merge, Plan, PlanView,
    PlannedFile, Row, Span,
};
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
const QUIT: &str = "diff.quit";
const NEXT: &str = "diff.next";
const PREVIOUS: &str = "diff.previous";
const FOCUS_NEXT: &str = "diff.focus-next";
const FOCUS_PREVIOUS: &str = "diff.focus-previous";
const WIDEN: &str = "diff.widen";
const NARROW: &str = "diff.narrow";
const DOWN: &str = "diff.down";
const UP: &str = "diff.up";
const MINE: &str = "diff.mine";
const THEIRS: &str = "diff.theirs";
const BOTH: &str = "diff.both";
const EDIT: &str = "diff.edit";
const WRITE: &str = "diff.write";
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
const MINE_CHORDS: &[KeyChord] = &[KeyChord::new(KeyCode::Char('o'), Modifiers::new())];
const THEIRS_CHORDS: &[KeyChord] = &[KeyChord::new(KeyCode::Char('t'), Modifiers::new())];
const BOTH_CHORDS: &[KeyChord] = &[KeyChord::new(KeyCode::Char('b'), Modifiers::new())];
const EDIT_CHORDS: &[KeyChord] = &[KeyChord::new(KeyCode::Char('e'), Modifiers::new())];
const WRITE_CHORDS: &[KeyChord] = &[KeyChord::new(KeyCode::Char('w'), Modifiers::new())];

pub struct DiffScreen {
    merge: Merge,
    choices: Vec<Option<Choice>>,
    editor: Box<dyn FnMut(String) -> Result<String, String>>,
    notice: Option<String>,
    plan: Option<crate::PlanDialog>,
    chips: Vec<Chip>,
    aligned: Aligned,
    scroll: CodeViewState,
    tree: SplitTree,
    drag: SplitDrag,
    focus: usize,
    hunk: usize,
    keymap: Keymap,
    columns: u16,
    rows: u16,
    generation: u64,
    painted: Option<Painted>,
    force_redraw: AtomicBool,
}

struct Chip {
    command: &'static str,
    x: u16,
    y: u16,
    width: u16,
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
        Self::from_merge(&merge3(base, ours, theirs))
    }

    pub fn with_editor(
        base: &str,
        ours: &str,
        theirs: &str,
        editor: impl FnMut(String) -> Result<String, String> + 'static,
    ) -> Self {
        Self::assemble(merge3(base, ours, theirs), Box::new(editor))
    }

    pub fn from_merge(merge: &Merge) -> Self {
        Self::assemble(merge.clone(), Box::new(default_editor))
    }

    fn assemble(merge: Merge, editor: Box<dyn FnMut(String) -> Result<String, String>>) -> Self {
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
        let choices = vec![None; merge.hunk_count()];
        let aligned = align(&merge, &choices);
        Self {
            merge,
            choices,
            editor,
            notice: None,
            plan: None,
            chips: Vec::new(),
            aligned,
            scroll: CodeViewState::new(),
            tree,
            drag: SplitDrag::new(),
            focus: 0,
            hunk: 0,
            keymap: keymap(),
            columns: 0,
            rows: 0,
            generation: 0,
            painted: None,
            force_redraw: AtomicBool::new(false),
        }
    }

    pub fn result_text(&self) -> Result<String, drift_core::Error> {
        self.merge.render(&self.choices)
    }

    pub fn plan_open(&self) -> bool {
        self.plan.is_some()
    }

    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
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
        self.scroll.scroll_to(self.aligned.hunk_rows[self.hunk]);
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

    fn realign(&mut self) {
        let hunk = self.hunk;
        self.aligned = align(&self.merge, &self.choices);
        let count = self.aligned.hunk_rows.len();
        self.hunk = if count == 0 { 0 } else { hunk.min(count - 1) };
        if let Some(row) = self.aligned.hunk_rows.get(self.hunk).copied() {
            self.scroll.scroll_to(row);
        }
        self.revise();
    }

    fn choose(&mut self, choice: Choice) {
        if self.hunk >= self.choices.len() {
            return;
        }
        self.choices[self.hunk] = Some(choice);
        self.notice = None;
        self.realign();
    }

    fn edit_selected(&mut self) {
        if self.hunk >= self.choices.len() {
            return;
        }
        let current = self.choices[self.hunk].as_ref();
        let seed = self
            .merge
            .hunk_text(self.hunk, current)
            .or_else(|| {
                self.merge
                    .hunks()
                    .nth(self.hunk)
                    .map(|hunk| hunk.ours.clone())
            })
            .unwrap_or_default();
        match (self.editor)(seed) {
            Ok(text) => {
                self.choices[self.hunk] = Some(Choice::Custom(text));
                self.notice = None;
                self.realign();
            }
            Err(error) => {
                self.notice = Some(error);
                self.revise();
            }
        }
        self.force_redraw.store(true, Ordering::Relaxed);
    }

    fn write_plan(&mut self) {
        self.revise();
        let after = match self.merge.render(&self.choices) {
            Ok(text) => text,
            Err(_) => {
                self.notice = Some(String::from("unresolved conflict blocks write"));
                self.plan = None;
                return;
            }
        };
        let file = PlannedFile::new(
            "config",
            Action::Merge,
            None,
            None,
            None,
            Some("backup/config"),
        )
        .expect("plan path");
        let plan = Plan::new("preview", "now", "0", vec![file], &[]).expect("plan");
        let contents = [FileContents {
            path: String::from("config"),
            before: Some(ours_bytes(&self.merge)),
            after: Some(after.into_bytes()),
        }];
        let view = PlanView::build(&plan, "state", &contents);
        self.notice = None;
        self.plan = Some(crate::PlanDialog::new(view));
    }

    fn header_line(&mut self, area: Rect) -> String {
        self.chips.clear();
        let mut line = String::from("Diff");
        if let Some(notice) = &self.notice {
            line.push_str("  ");
            line.push_str(notice);
        }
        let chips = [
            (MINE, " [o] mine"),
            (THEIRS, " [t] theirs"),
            (BOTH, " [b] both"),
            (EDIT, " [e] edit"),
        ];
        let extra: usize = chips.iter().map(|(_, label)| label.len()).sum();
        if line.len() + extra <= usize::from(area.width) {
            for (command, label) in chips {
                let start = line.len();
                line.push_str(label);
                let bracket = label.find('[').unwrap_or(0);
                self.chips.push(Chip {
                    command,
                    x: area
                        .x
                        .saturating_add(u16::try_from(start + bracket).unwrap_or(0)),
                    y: area.y,
                    width: 3,
                });
            }
        }
        line
    }

    fn apply_chip(&mut self, column: u16, row: u16) -> bool {
        let command = self
            .chips
            .iter()
            .find(|chip| {
                row == chip.y && column >= chip.x && column < chip.x.saturating_add(chip.width)
            })
            .map(|chip| chip.command);
        match command {
            Some(MINE) => self.choose(Choice::Mine),
            Some(THEIRS) => self.choose(Choice::Theirs),
            Some(BOTH) => self.choose(Choice::Both),
            Some(EDIT) => self.edit_selected(),
            Some(_) => return false,
            None => return false,
        }
        true
    }

    fn select_hunk_at(&mut self, area: Rect, column: u16, row: u16) {
        if !contains(area, column, row) || row <= area.y {
            return;
        }
        let start = self.hunk.saturating_sub(1);
        let index = start + usize::from(row - area.y - 1);
        if index >= self.aligned.hunk_labels.len() {
            return;
        }
        self.hunk = index;
        if let Some(line) = self.aligned.hunk_rows.get(index).copied() {
            self.scroll.scroll_to(line);
        }
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
    map.register(Command::new(MINE, "Mine", MINE_CHORDS));
    map.register(Command::new(THEIRS, "Theirs", THEIRS_CHORDS));
    map.register(Command::new(BOTH, "Both", BOTH_CHORDS));
    map.register(Command::new(EDIT, "Edit", EDIT_CHORDS));
    map.register(Command::new(WRITE, "Write", WRITE_CHORDS));
    map
}

fn default_editor(text: String) -> Result<String, String> {
    let Some(program) = std::env::var_os("EDITOR").filter(|program| !program.is_empty()) else {
        return Err(String::from("editor is not set"));
    };
    crate::suspend::around_editor(|| drift_apply::edit_text(&text, &program))
}

fn ours_bytes(merge: &Merge) -> Vec<u8> {
    let mut text = String::new();
    for segment in merge.segments() {
        match segment {
            drift_core::Segment::Stable(part) => text.push_str(part),
            drift_core::Segment::Hunk(hunk) => text.push_str(&hunk.ours),
        }
    }
    text.into_bytes()
}

fn align(merge: &Merge, choices: &[Option<Choice>]) -> Aligned {
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
                    let plain = owned_line(shown, Vec::new(), None);
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
                let choice = choices.get(hunk_index).and_then(Option::as_ref);
                push_hunk(
                    &mut aligned,
                    merge,
                    hunk,
                    hunk_index,
                    &rows[start..at],
                    choice,
                );
                hunk_index += 1;
            }
        }
    }
    aligned
}

fn push_hunk(
    aligned: &mut Aligned,
    merge: &Merge,
    hunk: &Hunk,
    index: usize,
    rows: &[Row],
    choice: Option<&Choice>,
) {
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
    let result = if choice.is_some() {
        raw_lines(merge.hunk_text(index, choice).as_deref().unwrap_or(""))
    } else {
        result
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
        if self.plan.is_some() {
            if let Event::Mouse(mouse) = event {
                if mouse_quits(self.columns, self.rows, mouse) {
                    return ControlFlow::Break;
                }
            }
            let done = self
                .plan
                .as_mut()
                .is_some_and(|plan| plan.handle(event).is_some());
            if done {
                self.plan = None;
                self.revise();
            }
            return ControlFlow::Continue;
        }
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
            Some(MINE) => {
                self.choose(Choice::Mine);
                ControlFlow::Continue
            }
            Some(THEIRS) => {
                self.choose(Choice::Theirs);
                ControlFlow::Continue
            }
            Some(BOTH) => {
                self.choose(Choice::Both);
                ControlFlow::Continue
            }
            Some(EDIT) => {
                self.edit_selected();
                ControlFlow::Continue
            }
            Some(WRITE) => {
                self.write_plan();
                ControlFlow::Continue
            }
            _ => ControlFlow::Continue,
        }
    }

    fn render(&mut self, grid: &mut Grid, area: Rect, clusters: &mut lunatui_core::ClusterTable) {
        self.columns = area.width;
        self.rows = area.height;
        if let Some(plan) = self.plan.as_mut() {
            plan.render(grid, area, clusters);
            self.painted = None;
            return;
        }
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
            let title = self.header_line(header);
            cx.put_str(
                header.x,
                header.y,
                clip_display_prefix(&title, header.width),
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

    fn token_palette(&self) -> Option<&[Color]> {
        if self.force_redraw.swap(false, Ordering::Relaxed) {
            Some(&[])
        } else {
            None
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
                let pane = layout.panes.first().map(|pane| pane.rect);
                let divider = self
                    .drag
                    .press(&self.tree, &layout, mouse.column, mouse.row);
                if divider || self.apply_chip(mouse.column, mouse.row) {
                    return;
                }
                if let Some(area) = pane {
                    self.select_hunk_at(area, mouse.column, mouse.row);
                }
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

    fn prepare_scroll(&mut self, rows: u16) -> CodeViewState {
        let mut scroll = self.scroll;
        let area = Rect::new(0, 0, 8, rows);
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
        let started = Instant::now();
        let (grid, clusters) = render_once(&mut changed, 160, 40);
        let scrolled = started.elapsed();
        assert!(grid.display_text(&clusters).contains('B'));
        changed.update(&Event::Key(KeyEvent::press(KeyCode::Tab)));
        let started = Instant::now();
        let _ = render_once(&mut changed, 160, 40);
        let focused = started.elapsed();
        changed.update(&Event::Key(KeyEvent::press(KeyCode::Char('n'))));
        let started = Instant::now();
        let (grid, clusters) = render_once(&mut changed, 160, 40);
        let jumped = started.elapsed();
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

    fn press(screen: &mut DiffScreen, key: char) {
        screen.update(&Event::Key(KeyEvent::press(KeyCode::Char(key))));
    }

    fn conflict_files() -> (&'static str, &'static str, &'static str) {
        (
            include_str!("../../drift-fixtures/fixtures/status/conflict/base/hypr/looknfeel.lua"),
            include_str!("../../drift-fixtures/fixtures/status/conflict/ours/hypr/looknfeel.lua"),
            include_str!("../../drift-fixtures/fixtures/status/conflict/theirs/hypr/looknfeel.lua"),
        )
    }

    #[test]
    fn hunk_keys_build_the_fixture_result() {
        let (base, ours, theirs) = conflict_files();
        let mut screen = DiffScreen::new(base, ours, theirs);
        press(&mut screen, 'o');
        assert_eq!(screen.result_text().unwrap(), ours);
        press(&mut screen, 't');
        assert_eq!(screen.result_text().unwrap(), theirs);
        press(&mut screen, 'b');
        assert_eq!(
            screen.result_text().unwrap(),
            "alpha\nours-middle\ntheirs-middle\nomega\n"
        );
    }

    #[test]
    fn an_unresolved_conflict_blocks_write_and_says_why() {
        let (base, ours, theirs) = conflict_files();
        let mut screen = DiffScreen::new(base, ours, theirs);
        press(&mut screen, 'w');
        assert!(!screen.plan_open());
        assert_eq!(screen.notice(), Some("unresolved conflict blocks write"));
        let (grid, clusters) = render_once(&mut screen, 160, 40);
        let text = grid.display_text(&clusters);
        assert!(text.contains("unresolved conflict blocks write"), "{text}");
        assert!(!screen.plan_open());
    }

    #[test]
    fn a_clean_merge_opens_the_plan() {
        let mut screen = DiffScreen::new(
            include_str!(
                "../../drift-fixtures/fixtures/status/merges-cleanly/base/hypr/looknfeel.lua"
            ),
            include_str!(
                "../../drift-fixtures/fixtures/status/merges-cleanly/ours/hypr/looknfeel.lua"
            ),
            include_str!(
                "../../drift-fixtures/fixtures/status/merges-cleanly/theirs/hypr/looknfeel.lua"
            ),
        );
        assert_eq!(screen.result_text().unwrap(), "alpha\nours\ntheirs\n");
        press(&mut screen, 'w');
        assert!(screen.plan_open());
        let (grid, clusters) = render_once(&mut screen, 160, 40);
        let text = grid.display_text(&clusters);
        assert!(text.contains("merge"), "{text}");
        press(&mut screen, 'n');
        assert!(!screen.plan_open());
    }

    #[test]
    fn a_fake_editor_round_trips_into_the_result() {
        let (base, ours, theirs) = conflict_files();
        let mut screen =
            DiffScreen::with_editor(base, ours, theirs, |_| Ok(String::from("custom-middle\n")));
        press(&mut screen, 'e');
        assert_eq!(
            screen.result_text().unwrap(),
            "alpha\ncustom-middle\nomega\n"
        );
    }

    #[test]
    fn a_click_selects_a_hunk_and_a_chip_keeps_ours() {
        let mut screen = two_hunks();
        let _ = render_once(&mut screen, 200, 40);
        screen.update(&Event::Mouse(MouseEvent::new(
            MouseKind::Down(MouseButton::Left),
            2,
            3,
        )));
        assert_eq!(screen.selected_hunk(), 1);
        press(&mut screen, 'o');
        assert!(screen.result_text().unwrap().ends_with("z\n"));

        let mut replacement = DiffScreen::new("a\n", "a\n", "b\n");
        let (grid, clusters) = render_once(&mut replacement, 200, 40);
        let header = grid
            .display_text(&clusters)
            .lines()
            .next()
            .unwrap()
            .to_string();
        let column = u16::try_from(header.find("[o]").expect("chip")).unwrap();
        replacement.update(&Event::Mouse(MouseEvent::new(
            MouseKind::Down(MouseButton::Left),
            column,
            0,
        )));
        assert_eq!(replacement.result_text().unwrap(), "a\n");
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
