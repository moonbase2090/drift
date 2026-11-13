use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};

use drift_core::{
    merge3, Action, Choice, DiffContent, DiffRows, FileContents, FileDiff, Hunk, HunkKind, Mark,
    Merge, Plan, PlanView, PlannedFile, Row, Span,
};
use lunatui::{App, ControlFlow};
use lunatui_core::{clip_display_prefix, width, Attrs, Color, Grid, Rect, Style, CLUSTER_TAG};
use lunatui_runtime::keymap::{Command, KeyChord, Keymap};
use lunatui_term::{Event, KeyCode, Modifiers, MouseButton, MouseEvent, MouseKind, Scroll};
use lunatui_theme::{Theme, Token, Variant};
use lunatui_widgets::{
    CodeHighlight, CodeMark, CodeView, CodeViewState, DividerId, RenderCx, SplitAxis, SplitDrag,
    SplitLayout, SplitTree, Widget, WidgetState,
};
use unicode_width::UnicodeWidthChar;

use crate::diff_palette::{DiffPalette, RowKind, Tints};

const MIN_TEXT_PANE: u16 = 40;
const WIDE: u16 = MIN_TEXT_PANE * 5;
const MEDIUM: u16 = 100;
const QUIT: &str = "diff.quit";
const LAYOUT: &str = "diff.layout";
const NEXT: &str = "diff.next";
const PREVIOUS: &str = "diff.previous";
const NEXT_CONFLICT: &str = "diff.next-conflict";
const PREVIOUS_CONFLICT: &str = "diff.previous-conflict";
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
const LAYOUT_CHORDS: &[KeyChord] = &[KeyChord::new(KeyCode::Char('v'), Modifiers::new())];
const HIGHLIGHT: Color = Color::Rgb(220, 160, 40);

const QUIT_CHORDS: &[KeyChord] = &[KeyChord::new(KeyCode::Char('q'), Modifiers::new())];
const NEXT_CHORDS: &[KeyChord] = &[KeyChord::new(KeyCode::Char('n'), Modifiers::new())];
const NEXT_CONFLICT_CHORDS: &[KeyChord] = &[KeyChord::new(KeyCode::Char('N'), Modifiers::new())];
const PREVIOUS_CONFLICT_CHORDS: &[KeyChord] =
    &[KeyChord::new(KeyCode::Char('P'), Modifiers::new())];
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    palette: DiffPalette,
    force_redraw: AtomicBool,
    redraw_phase: AtomicBool,
}

pub(crate) const HINTS: &[(&str, KeyCode)] = &[
    ("[Q] Quit", KeyCode::Char('q')),
    ("[?] Help", KeyCode::Char('?')),
    ("[:] Palette", KeyCode::Char(':')),
    ("[W] Write", KeyCode::Char('w')),
    ("[N] Next hunk", KeyCode::Char('n')),
    ("[P] Prev hunk", KeyCode::Char('p')),
    ("[V] Layout", KeyCode::Char('v')),
    ("[Tab] Pane", KeyCode::Tab),
];

fn hint_cells(area: Rect) -> Vec<(Rect, usize)> {
    let end = area.x.saturating_add(area.width);
    let mut cells = Vec::new();
    let mut x = area.x;
    for (index, (label, _)) in HINTS.iter().enumerate() {
        let span = u16::try_from(width(label)).unwrap_or(u16::MAX);
        if x.saturating_add(span) > end {
            break;
        }
        cells.push((Rect::new(x, area.y, span, 1), index));
        x = x.saturating_add(span).saturating_add(2);
    }
    cells
}

fn render_hints(area: Rect, cx: &mut RenderCx<'_>) {
    let key = Style::new().fg(cx.token(Token::Accent));
    let title = Style::new().fg(cx.token(Token::TextPrimary));
    for (rect, index) in hint_cells(area) {
        let label = HINTS[index].0;
        let split = label.find(']').map_or(0, |at| at + 1);
        cx.put_str(rect.x, rect.y, &label[..split], key);
        let keys = u16::try_from(width(&label[..split])).unwrap_or(0);
        cx.put_str(rect.x + keys, rect.y, &label[split..], title);
    }
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
    kind: RowKind,
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
    duo_ours: Vec<Line>,
    duo_theirs: Vec<Line>,
    hunk_rows: Vec<usize>,
    hunk_labels: Vec<String>,
    stacked: Vec<Line>,
    stacked_hunk_rows: Vec<usize>,
}

struct Hits<'a> {
    lines: &'a [Line],
    tints: &'a Tints,
}

impl CodeHighlight for Hits<'_> {
    fn highlight(&self, line: usize, text: &str, paint: &mut dyn FnMut(Range<usize>, Style)) {
        let Some(line) = self.lines.get(line) else {
            return;
        };
        if let Some(row) = self.tints.row(line.kind) {
            if !text.is_empty() {
                paint(0..text.len(), Style::new().bg(row));
            }
        }
        let style = match self.tints.word(line.kind) {
            Some(word) => Style::new().bg(word),
            None => Style::new().fg(HIGHLIGHT),
        };
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

    pub fn with_palette(mut self, palette: DiffPalette) -> Self {
        self.palette = palette;
        self.painted = None;
        self
    }

    pub fn with_title(mut self, title: &str) -> Self {
        self.title = Some(title.to_string());
        self
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
            pinned: None,
            class: None,
            two_way: false,
            title: None,
            keymap: diff_keymap(),
            columns: 0,
            rows: 0,
            generation: 0,
            stacked_scroll: None,
            painted: None,
            palette: DiffPalette::fallback(),
            force_redraw: AtomicBool::new(false),
            redraw_phase: AtomicBool::new(false),
        }
    }

    pub fn result_text(&self) -> Result<String, drift_core::Error> {
        self.merge.render(&self.choices)
    }

    pub fn plan_open(&self) -> bool {
        self.plan.is_some()
    }

    pub(crate) fn set_notice(&mut self, text: &str) {
        self.revise();
        self.notice = Some(text.to_string());
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

    fn jump_conflict(&mut self, forward: bool) {
        let open: Vec<usize> = self
            .merge
            .hunks()
            .enumerate()
            .filter(|(index, hunk)| {
                hunk.kind == HunkKind::Conflict && self.choices[*index].is_none()
            })
            .map(|(index, _)| index)
            .collect();
        let target = if forward {
            open.iter().copied().find(|index| *index > self.hunk)
        } else {
            open.iter().copied().rev().find(|index| *index < self.hunk)
        };
        self.revise();
        match target {
            Some(index) => {
                self.notice = None;
                self.hunk = index;
                self.scroll.scroll_to(self.hunk_scroll_row());
            }
            None => {
                let side = if forward { "after" } else { "before" };
                self.notice = Some(format!("no unresolved conflict {side} this hunk"));
            }
        }
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

    fn realign(&mut self) {
        let hunk = self.hunk;
        self.aligned = align(&self.merge, &self.choices);
        let count = self.aligned.hunk_rows.len();
        self.hunk = if count == 0 { 0 } else { hunk.min(count - 1) };
        self.scroll.scroll_to(self.hunk_scroll_row());
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
        let mut line = self.header_text();
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

pub(crate) fn diff_keymap() -> Keymap {
    let mut map = Keymap::new();
    map.register(Command::new(QUIT, "Quit", QUIT_CHORDS));
    map.register(Command::new(NEXT, "Next hunk", NEXT_CHORDS));
    map.register(Command::new(PREVIOUS, "Previous hunk", PREVIOUS_CHORDS));
    map.register(Command::new(
        NEXT_CONFLICT,
        "Next unresolved conflict",
        NEXT_CONFLICT_CHORDS,
    ));
    map.register(Command::new(
        PREVIOUS_CONFLICT,
        "Previous unresolved conflict",
        PREVIOUS_CONFLICT_CHORDS,
    ));
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

fn align(merge: &Merge, choices: &[Option<Choice>]) -> Aligned {
    let built = DiffRows::build(merge);
    let rows = built.rows();
    let mut at = 0;
    let mut aligned = Aligned {
        base: Vec::new(),
        ours: Vec::new(),
        theirs: Vec::new(),
        result: Vec::new(),
        duo_ours: Vec::new(),
        duo_theirs: Vec::new(),
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
                    aligned
                        .stacked
                        .push(owned_line(format!("  {line}"), Vec::new(), None));
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
                    aligned.duo_ours.push(plain.clone());
                    aligned.duo_theirs.push(plain.clone());
                    aligned.result.push(plain);
                }
            }
            drift_core::Segment::Hunk(hunk) => {
                let choice = choices.get(hunk_index).and_then(Option::as_ref);
                let start = at;
                while at < rows.len() && rows[at].hunk == Some(hunk_index) {
                    at += 1;
                }
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

struct HunkLines {
    base: Vec<Line>,
    ours: Vec<Line>,
    theirs: Vec<Line>,
    result: Vec<Line>,
    duo_ours: Vec<Line>,
    duo_theirs: Vec<Line>,
    stacked: Vec<(&'static str, Vec<Line>)>,
}

fn hunk_lines(hunk: &Hunk, rows: &[Row]) -> HunkLines {
    let removed = or_raw(from_mark(rows, Mark::Removed), &hunk.base);
    match hunk.kind {
        HunkKind::Conflict => {
            let ours = from_mark(rows, Mark::Mine);
            let theirs = from_mark(rows, Mark::Theirs);
            let mut duo_ours = ours.clone();
            let mut duo_theirs = theirs.clone();
            pair_kinds(&mut duo_ours, &mut duo_theirs);
            HunkLines {
                base: kinded(raw_lines(&hunk.base), RowKind::Removed),
                ours: kinded(ours.clone(), RowKind::Changed),
                theirs: kinded(theirs.clone(), RowKind::Changed),
                result: vec![owned_line(
                    "unresolved".to_string(),
                    Vec::new(),
                    Some(CodeMark::Error),
                )],
                duo_ours,
                duo_theirs,
                stacked: vec![
                    ("- ", kinded(removed.clone(), RowKind::Removed)),
                    ("< ours ", kinded(ours, RowKind::Changed)),
                    ("> theirs ", kinded(theirs, RowKind::Changed)),
                ],
            }
        }
        HunkKind::TheirsOnly => {
            let added = or_raw(from_mark(rows, Mark::Added), &hunk.theirs);
            let mut duo_ours = removed.clone();
            let mut duo_theirs = added.clone();
            pair_kinds(&mut duo_ours, &mut duo_theirs);
            HunkLines {
                base: kinded(removed.clone(), RowKind::Removed),
                ours: plain(&removed),
                theirs: kinded(added.clone(), RowKind::Added),
                result: kinded(added.clone(), RowKind::Added),
                duo_ours,
                duo_theirs,
                stacked: vec![
                    ("- ", kinded(removed.clone(), RowKind::Removed)),
                    ("+ ", kinded(added, RowKind::Added)),
                ],
            }
        }
        HunkKind::OursOnly => {
            let added = or_raw(from_mark(rows, Mark::Added), &hunk.ours);
            let mut duo_ours = added.clone();
            let mut duo_theirs = removed.clone();
            pair_kinds(&mut duo_ours, &mut duo_theirs);
            HunkLines {
                base: kinded(removed.clone(), RowKind::Removed),
                ours: kinded(added.clone(), RowKind::Added),
                theirs: plain(&removed),
                result: kinded(added.clone(), RowKind::Added),
                duo_ours,
                duo_theirs,
                stacked: vec![
                    ("- ", kinded(removed.clone(), RowKind::Removed)),
                    ("+ ", kinded(added, RowKind::Added)),
                ],
            }
        }
        HunkKind::BothSame => {
            let added = or_raw(from_mark(rows, Mark::Added), &hunk.ours);
            HunkLines {
                base: kinded(removed.clone(), RowKind::Removed),
                ours: kinded(added.clone(), RowKind::Added),
                theirs: kinded(added.clone(), RowKind::Added),
                result: kinded(added.clone(), RowKind::Added),
                duo_ours: plain(&added),
                duo_theirs: plain(&added),
                stacked: vec![
                    ("- ", kinded(removed.clone(), RowKind::Removed)),
                    ("+ ", kinded(added, RowKind::Added)),
                ],
            }
        }
    }
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
    let lines = hunk_lines(hunk, rows);
    aligned.stacked_hunk_rows.push(aligned.stacked.len());
    let chosen = choice.and_then(|_| merge.hunk_text(index, choice));
    if let Some(text) = chosen.as_deref() {
        for line in kinded(raw_lines(text), RowKind::Added) {
            aligned.stacked.push(prefixed("= ", line));
        }
    }
    for (prefix, group) in lines.stacked {
        for line in group {
            aligned.stacked.push(prefixed(prefix, line));
        }
    }
    let result = if choice.is_some() {
        kinded(
            raw_lines(merge.hunk_text(index, choice).as_deref().unwrap_or("")),
            RowKind::Added,
        )
    } else {
        lines.result
    };
    let height = [
        lines.base.len(),
        lines.ours.len(),
        lines.theirs.len(),
        result.len(),
        lines.duo_ours.len(),
        lines.duo_theirs.len(),
    ]
    .into_iter()
    .max()
    .unwrap_or(0);
    extend(&mut aligned.base, lines.base, height);
    extend(&mut aligned.ours, lines.ours, height);
    extend(&mut aligned.theirs, lines.theirs, height);
    extend(&mut aligned.result, result, height);
    extend(&mut aligned.duo_ours, lines.duo_ours, height);
    extend(&mut aligned.duo_theirs, lines.duo_theirs, height);
}

fn kinded(mut lines: Vec<Line>, kind: RowKind) -> Vec<Line> {
    for line in &mut lines {
        line.kind = kind;
    }
    lines
}

fn pair_kinds(ours: &mut [Line], theirs: &mut [Line]) {
    let shared = ours.len().min(theirs.len());
    for (index, line) in ours.iter_mut().enumerate() {
        line.kind = if index < shared {
            RowKind::Changed
        } else {
            RowKind::Added
        };
    }
    for (index, line) in theirs.iter_mut().enumerate() {
        line.kind = if index < shared {
            RowKind::Changed
        } else {
            RowKind::Removed
        };
    }
}

fn prefixed(prefix: &str, line: Line) -> Line {
    let shift = prefix.len();
    let spans = line
        .spans
        .iter()
        .map(|span| span.start + shift..span.end + shift)
        .collect();
    let mut shown = owned_line(format!("{prefix}{}", line.text), spans, None);
    shown.kind = line.kind;
    shown
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
            kind: RowKind::Plain,
        })
        .collect()
}

fn line_from_row(row: &Row) -> Line {
    let spans = row
        .spans
        .iter()
        .filter_map(|span| columns_to_bytes(&row.text, *span))
        .collect::<Vec<_>>();
    owned_line(row.text.clone(), spans, None)
}

fn raw_lines(text: &str) -> Vec<Line> {
    split_lines(text)
        .into_iter()
        .map(|text| owned_line(text, Vec::new(), None))
        .collect()
}

fn extend(into: &mut Vec<Line>, mut lines: Vec<Line>, height: usize) {
    while lines.len() < height {
        let mut blank = owned_line(String::new(), Vec::new(), None);
        blank.kind = RowKind::Filler;
        lines.push(blank);
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
        kind: RowKind::Plain,
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
            if let Some(press) = self.hint_press(mouse) {
                return self.update(&press);
            }
            self.pointer(mouse);
            return ControlFlow::Continue;
        }
        match self.keymap.dispatch(event) {
            Some(id) => self.run(id).unwrap_or(ControlFlow::Continue),
            None => ControlFlow::Continue,
        }
    }

    fn render(&mut self, grid: &mut Grid, area: Rect, clusters: &mut lunatui_core::ClusterTable) {
        self.columns = area.width;
        self.rows = area.height;
        self.note_class();
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
            match self.effective() {
                Layout::Four => self.render_wide(body, &mut cx, accent, text),
                Layout::Two => self.render_two(body, &mut cx, accent, text),
                Layout::Stacked => self.render_stacked(body, &mut cx, accent, text),
            }
        }
        if hint.width > 0 && hint.height > 0 {
            render_hints(hint, &mut cx);
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
            self.redraw_phase.fetch_xor(true, Ordering::Relaxed);
        }
        if self.redraw_phase.load(Ordering::Relaxed) {
            Some(&[])
        } else {
            None
        }
    }
}

impl DiffScreen {
    pub(crate) fn run(&mut self, id: &str) -> Option<ControlFlow> {
        Some(match id {
            QUIT => ControlFlow::Break,
            NEXT => {
                self.move_hunk(1);
                ControlFlow::Continue
            }
            PREVIOUS => {
                self.move_hunk(-1);
                ControlFlow::Continue
            }
            NEXT_CONFLICT => {
                self.jump_conflict(true);
                ControlFlow::Continue
            }
            PREVIOUS_CONFLICT => {
                self.jump_conflict(false);
                ControlFlow::Continue
            }
            FOCUS_NEXT => {
                self.move_focus(1);
                ControlFlow::Continue
            }
            FOCUS_PREVIOUS => {
                self.move_focus(-1);
                ControlFlow::Continue
            }
            WIDEN => {
                self.resize_focused(4);
                ControlFlow::Continue
            }
            NARROW => {
                self.resize_focused(-4);
                ControlFlow::Continue
            }
            DOWN => {
                self.scroll_by(1);
                ControlFlow::Continue
            }
            UP => {
                self.scroll_by(-1);
                ControlFlow::Continue
            }
            MINE => {
                self.choose(Choice::Mine);
                ControlFlow::Continue
            }
            THEIRS => {
                self.choose(Choice::Theirs);
                ControlFlow::Continue
            }
            BOTH => {
                self.choose(Choice::Both);
                ControlFlow::Continue
            }
            EDIT => {
                self.edit_selected();
                ControlFlow::Continue
            }
            WRITE => {
                self.write_plan();
                ControlFlow::Continue
            }
            LAYOUT => {
                self.cycle_layout();
                ControlFlow::Continue
            }
            _ => return None,
        })
    }
}

impl DiffScreen {
    pub(crate) fn hint_press(&self, mouse: &MouseEvent) -> Option<Event> {
        if self.plan.is_some() || !matches!(mouse.kind, MouseKind::Down(MouseButton::Left)) {
            return None;
        }
        let hint = regions(Rect::new(0, 0, self.columns, self.rows)).2;
        if hint.height == 0 || mouse.row != hint.y {
            return None;
        }
        hint_cells(hint)
            .into_iter()
            .find(|(rect, _)| contains(*rect, mouse.column, mouse.row))
            .map(|(_, index)| crate::commands::press(HINTS[index].1))
    }

    fn pointer(&mut self, mouse: &MouseEvent) {
        let wide = self.wide();
        let body = self.body();
        let layout = self.tree.layout(body);
        match mouse.kind {
            MouseKind::Down(MouseButton::Left) => {
                self.revise();
                if self.apply_chip(mouse.column, mouse.row) || !wide {
                    return;
                }
                let pane = layout.panes.first().map(|pane| pane.rect);
                let divider = self
                    .drag
                    .press(&self.tree, &layout, mouse.column, mouse.row);
                if divider {
                    return;
                }
                if let Some(area) = pane {
                    self.select_hunk_at(area, mouse.column, mouse.row);
                }
            }
            MouseKind::Drag(MouseButton::Left) if wide => {
                self.revise();
                self.drag.drag(&mut self.tree, mouse.column, mouse.row);
            }
            MouseKind::Up(MouseButton::Left) if wide => {
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
        let tints = self.palette.tints(cx.token(Token::Surface0));
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
            self.render_code(area.rect, (PANES[index], style), lines, scroll, &tints, cx);
        }
        let active = self.drag.active().or_else(|| self.focused_divider());
        paint_dividers(&layout, text, accent, active, cx);
    }

    fn render_two(&mut self, body: Rect, cx: &mut RenderCx<'_>, accent: Style, text: Style) {
        let (top, strip) = split_strip(body);
        let scroll = self.prepare_scroll(top.height.saturating_sub(1));
        let tints = self.palette.tints(cx.token(Token::Surface0));
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
        self.render_code(
            left,
            ("ours", ours),
            &self.aligned.duo_ours,
            scroll,
            &tints,
            cx,
        );
        self.render_code(
            right,
            ("theirs", theirs),
            &self.aligned.duo_theirs,
            scroll,
            &tints,
            cx,
        );
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
        self.render_code(
            strip,
            ("result", result),
            &self.aligned.result,
            scroll,
            &tints,
            cx,
        );
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
            let tints = self.palette.tints(cx.token(Token::Surface0));
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
            self.render_code(
                top,
                ("ours", ours),
                &self.aligned.duo_ours,
                scroll,
                &tints,
                cx,
            );
            self.render_code(
                bottom,
                ("theirs", theirs),
                &self.aligned.duo_theirs,
                scroll,
                &tints,
                cx,
            );
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
        let tints = self.palette.tints(cx.token(Token::Surface0));
        let top = scroll.top();
        let rows = usize::from(view.height);
        for (offset, shown) in self.aligned.stacked.iter().skip(top).take(rows).enumerate() {
            let y = view.y.saturating_add(u16::try_from(offset).unwrap_or(0));
            let row = tints.row(shown.kind);
            let body = row.map_or(text, |color| text.bg(color));
            if row.is_some() {
                cx.fill(Rect::new(view.x, y, view.width, 1), body, ' ');
            }
            cx.put_str(
                view.x,
                y,
                clip_display_prefix(&shown.text, view.width),
                body,
            );
            if let Some((_, hue)) = tints.mark(shown.kind) {
                cx.set_style(Rect::new(view.x, y, 1, 1), body.fg(hue));
            }
            if let Some(word) = tints.word(shown.kind) {
                for span in &shown.spans {
                    let start = text_columns(&shown.text[..span.start]);
                    let end = text_columns(&shown.text[..span.end.min(shown.text.len())]);
                    let width = u16::try_from(end.saturating_sub(start)).unwrap_or(0);
                    let at = u16::try_from(start).unwrap_or(u16::MAX);
                    if width > 0 && at < view.width {
                        let reach = width.min(view.width - at);
                        cx.set_style(
                            Rect::new(view.x.saturating_add(at), y, reach, 1),
                            text.bg(word),
                        );
                    }
                }
            }
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
        heading: (&str, Style),
        lines: &[Line],
        scroll: CodeViewState,
        tints: &Tints,
        cx: &mut RenderCx<'_>,
    ) {
        let (title, title_style) = heading;
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
            paint_ascii(view_area, lines, top, rows, tints, cx);
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
        let hits = Hits { lines, tints };
        CodeView::new(lines)
            .view(&scroll)
            .marks(&marks)
            .highlight(&hits)
            .state(state)
            .render(view_area, cx);
        paint_rows(view_area, lines, top, rows, scroll.left(), tints, cx);
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

fn paint_ascii(
    view_area: Rect,
    lines: &[Line],
    top: usize,
    rows: usize,
    tints: &Tints,
    cx: &mut RenderCx<'_>,
) {
    let plain = Style::new().fg(cx.token(Token::TextPrimary));
    let numbers = plain.with_attrs(Attrs::new().set_dim(true));
    let highlight = Style::new().fg(HIGHLIGHT);
    let places = digit_places(lines.len().max(1));
    let gutter = places.saturating_add(2);
    let bar = u16::from(view_area.width > gutter && lines.len() > rows);
    let text_width = view_area.width.saturating_sub(gutter).saturating_sub(bar);
    let text_x = view_area.x.saturating_add(gutter.min(view_area.width));
    let limit = usize::from(text_width);
    let row_width = view_area.width.saturating_sub(bar);
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
        if line.kind == RowKind::Filler {
            if let Some(color) = tints.row(RowKind::Filler) {
                cx.fill(
                    Rect::new(view_area.x, y, row_width, 1),
                    plain.bg(color),
                    ' ',
                );
            }
            continue;
        }
        let row = tints.row(line.kind);
        let body = row.map_or(plain, |color| plain.bg(color));
        if row.is_some() {
            cx.fill(Rect::new(view_area.x, y, row_width, 1), body, ' ');
        }
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
            cx.put_str(x, y, shown, body);
            let width = u16::try_from(shown.len()).unwrap_or(0);
            match row {
                Some(color) => cx.set_style(Rect::new(x, y, width, 1), numbers.bg(color)),
                None => dims.push(cx, numbers, x, y, width),
            }
        }
        if let Some((glyph, hue)) = tints.mark(line.kind) {
            cx.fill(Rect::new(view_area.x, y, 1, 1), body.fg(hue), glyph);
        }
        if repeated.is_some() || limit == 0 || line.text.is_empty() {
            continue;
        }
        let shown = if line.text.len() > limit {
            &line.text[..limit]
        } else {
            line.text.as_str()
        };
        cx.put_str(text_x, y, shown, body);
        let word = tints
            .word(line.kind)
            .map_or(highlight, |color| plain.bg(color));
        paint_spans(cx, &line.spans, shown.len(), (text_x, y), word);
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

fn paint_spans(
    cx: &mut RenderCx<'_>,
    spans: &[Range<usize>],
    shown: usize,
    origin: (u16, u16),
    style: Style,
) {
    for span in spans {
        let end = span.end.min(shown);
        if span.start >= shown || end <= span.start {
            continue;
        }
        let x = origin
            .0
            .saturating_add(u16::try_from(span.start).unwrap_or(0));
        let width = u16::try_from(end - span.start).unwrap_or(0);
        cx.set_style(Rect::new(x, origin.1, width, 1), style);
    }
}

fn text_columns(text: &str) -> usize {
    let mut column = 0usize;
    for ch in text.chars() {
        if ch == '\t' {
            column += 4 - column % 4;
        } else {
            column += UnicodeWidthChar::width(ch).unwrap_or(0);
        }
    }
    column
}

fn paint_rows(
    view_area: Rect,
    lines: &[Line],
    top: usize,
    rows: usize,
    left: u16,
    tints: &Tints,
    cx: &mut RenderCx<'_>,
) {
    let plain = Style::new().fg(cx.token(Token::TextPrimary));
    let numbers = plain.with_attrs(Attrs::new().set_dim(true));
    let gutter = digit_places(lines.len().max(1))
        .saturating_add(2)
        .min(view_area.width);
    let bar = u16::from(view_area.width > gutter && lines.len() > rows);
    let row_width = view_area.width.saturating_sub(bar);
    for (offset, line) in lines.iter().skip(top).take(rows).enumerate() {
        let Some(row) = tints.row(line.kind) else {
            continue;
        };
        let y = view_area
            .y
            .saturating_add(u16::try_from(offset).unwrap_or(0));
        if line.kind == RowKind::Filler {
            cx.fill(Rect::new(view_area.x, y, row_width, 1), plain.bg(row), ' ');
            continue;
        }
        let gutter_width = gutter.min(row_width);
        if gutter_width > 1 {
            cx.set_style(
                Rect::new(view_area.x.saturating_add(1), y, gutter_width - 1, 1),
                numbers.bg(row),
            );
        }
        if gutter_width > 0 {
            let (glyph, style) = match tints.mark(line.kind) {
                Some((glyph, hue)) => (glyph, plain.fg(hue).bg(row)),
                None => (' ', plain.bg(row)),
            };
            cx.fill(Rect::new(view_area.x, y, 1, 1), style, glyph);
        }
        let used = text_columns(&line.text).saturating_sub(usize::from(left));
        let start = gutter.saturating_add(u16::try_from(used).unwrap_or(u16::MAX));
        if start < row_width {
            cx.fill(
                Rect::new(view_area.x.saturating_add(start), y, row_width - start, 1),
                plain.bg(row),
                ' ',
            );
        }
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
        if line.mark.is_some() || !line.spans.is_empty() || line.kind != RowKind::Plain {
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

    const SIZES: [(u16, u16); 2] = [(200, 40), (200, 60)];

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

    fn tints() -> Tints {
        DiffPalette::fallback().tints(Color::Rgb(30, 30, 46))
    }

    fn bg_at(grid: &lunatui_core::Grid, x: usize, y: usize) -> Color {
        grid.get(u16::try_from(x).unwrap(), u16::try_from(y).unwrap())
            .unwrap()
            .style()
            .bg
    }

    fn fg_cell(grid: &lunatui_core::Grid, x: usize, y: usize) -> Color {
        grid.get(u16::try_from(x).unwrap(), u16::try_from(y).unwrap())
            .unwrap()
            .style()
            .fg
    }

    fn columns_of(text: &str, row: usize, needle: &str) -> Vec<usize> {
        let line: Vec<char> = text.lines().nth(row).unwrap().chars().collect();
        let want: Vec<char> = needle.chars().collect();
        (0..line.len().saturating_sub(want.len()) + 1)
            .filter(|at| line[*at..*at + want.len()] == want[..])
            .collect()
    }

    fn row_color(kind: RowKind) -> Color {
        tints().row(kind).unwrap()
    }

    fn render_text(
        screen: &mut DiffScreen,
        columns: u16,
        rows: u16,
    ) -> (lunatui_core::Grid, String) {
        let (grid, clusters) = render_once(screen, columns, rows);
        let text = grid.display_text(&clusters);
        (grid, text)
    }

    fn ours_only() -> DiffScreen {
        DiffScreen::new(
            "alpha\nhello world\nomega\n",
            "alpha\nhello WORLD\nomega\n",
            "alpha\nhello world\nomega\n",
        )
    }

    #[test]
    fn the_four_pane_view_tints_whole_rows_and_marks_the_gutter() {
        let mut screen = ours_only();
        let (grid, text) = render_text(&mut screen, 200, 40);
        let row = cmp_row(&text, "WORLD").unwrap();
        let ours = columns_of(&text, row, "hello WORLD");
        let old = columns_of(&text, row, "hello world");
        let (base, theirs) = (old[0], old[1]);
        let (ours, result) = (ours[0], ours[1]);
        let edge = |x: usize| x + "hello WORLD".len() + 3;
        assert_eq!(bg_at(&grid, ours, row), row_color(RowKind::Added));
        assert_eq!(bg_at(&grid, edge(ours), row), row_color(RowKind::Added));
        assert_eq!(bg_at(&grid, base, row), row_color(RowKind::Removed));
        assert_eq!(bg_at(&grid, edge(base), row), row_color(RowKind::Removed));
        assert_eq!(bg_at(&grid, result, row), row_color(RowKind::Added));
        assert_ne!(bg_at(&grid, theirs, row), row_color(RowKind::Removed));
        assert_ne!(bg_at(&grid, theirs, row), row_color(RowKind::Added));
        let line: Vec<char> = text.lines().nth(row).unwrap().chars().collect();
        assert_eq!(line[ours - 3], '+');
        assert_eq!(line[base - 3], '-');
        assert_eq!(line[result - 3], '+');
        assert_ne!(line[theirs - 3], '+');
        assert_ne!(line[theirs - 3], '-');
        let above = row - 1;
        assert_ne!(bg_at(&grid, ours, above), row_color(RowKind::Added));
    }

    #[test]
    fn the_changed_characters_are_stronger_than_their_row() {
        let mut screen = ours_only();
        let (grid, text) = render_text(&mut screen, 200, 40);
        let row = cmp_row(&text, "WORLD").unwrap();
        let ours = columns_of(&text, row, "hello WORLD")[0];
        let word = tints().word(RowKind::Added).unwrap();
        assert_eq!(bg_at(&grid, ours + 6, row), word);
        assert_eq!(bg_at(&grid, ours + 10, row), word);
        assert_eq!(bg_at(&grid, ours, row), row_color(RowKind::Added));
        assert_ne!(word, row_color(RowKind::Added));
    }

    #[test]
    fn the_two_pane_view_shows_ours_only_and_theirs_only_lines_against_filler() {
        let mut screen = DiffScreen::new(
            "alpha\ngone\nomega\nkeep\n",
            "alpha\ngone\nomega\nnew line\nkeep\n",
            "alpha\nomega\nkeep\n",
        );
        let (grid, text) = render_text(&mut screen, 120, 30);
        assert!(matches!(screen.effective(), Layout::Two));
        let gone = cmp_row(&text, "gone").unwrap();
        let theirs_side = text.lines().nth(gone).unwrap().chars().count() / 2;
        assert_eq!(bg_at(&grid, 4, gone), row_color(RowKind::Added));
        let line: Vec<char> = text.lines().nth(gone).unwrap().chars().collect();
        assert_eq!(line[0], '+');
        let filler = bg_at(&grid, theirs_side + 4, gone);
        assert_eq!(filler, row_color(RowKind::Filler));
        assert_eq!(line[theirs_side + 1], ' ');
        assert!(
            !text
                .lines()
                .nth(gone)
                .unwrap()
                .chars()
                .skip(theirs_side + 1)
                .any(|c| c.is_ascii_digit()),
            "{text}"
        );
        let new_line = cmp_row(&text, "new line").unwrap();
        assert_eq!(bg_at(&grid, 4, new_line), row_color(RowKind::Added));
    }

    #[test]
    fn a_line_that_changed_on_both_sides_is_marked_and_tinted_as_changed() {
        let mut screen = DiffScreen::new("a\nb\n", "a\nmine\n", "a\ntheirs\n");
        let (grid, text) = render_text(&mut screen, 120, 30);
        let row = text.lines().position(|line| line.starts_with('~')).unwrap();
        let line: Vec<char> = text.lines().nth(row).unwrap().chars().collect();
        assert_eq!(line[0], '~');
        assert_eq!(bg_at(&grid, 1, row), row_color(RowKind::Changed));
        let theirs = columns_of(&text, row, "theirs")[0];
        assert_eq!(line[theirs - 3], '~');
        assert_eq!(bg_at(&grid, theirs - 2, row), row_color(RowKind::Changed));
        assert_eq!(
            bg_at(&grid, theirs, row),
            tints().word(RowKind::Changed).unwrap()
        );
    }

    #[test]
    fn a_theirs_only_line_in_two_pane_is_red_with_a_minus() {
        let mut screen = DiffScreen::new("a\nz\n", "a\nz\n", "a\nextra\nz\n");
        let (grid, text) = render_text(&mut screen, 120, 30);
        let row = cmp_row(&text, "extra").unwrap();
        let theirs = columns_of(&text, row, "extra")[0];
        let line: Vec<char> = text.lines().nth(row).unwrap().chars().collect();
        assert_eq!(line[theirs - 3], '-');
        assert_eq!(bg_at(&grid, theirs, row), row_color(RowKind::Removed));
        assert_eq!(bg_at(&grid, 4, row), row_color(RowKind::Filler));
    }

    #[test]
    fn the_stacked_view_tints_removed_and_added_rows() {
        let mut screen = ours_only();
        let (grid, text) = render_text(&mut screen, 80, 24);
        assert!(text.contains("stacked"));
        let minus = text
            .lines()
            .position(|l| l.starts_with("- hello world"))
            .unwrap();
        let plus = text
            .lines()
            .position(|l| l.starts_with("+ hello WORLD"))
            .unwrap();
        assert_eq!(bg_at(&grid, 30, minus), row_color(RowKind::Removed));
        assert_eq!(bg_at(&grid, 30, plus), row_color(RowKind::Added));
        assert_eq!(bg_at(&grid, 9, plus), tints().word(RowKind::Added).unwrap(),);
        assert_ne!(fg_cell(&grid, 0, plus), fg_cell(&grid, 30, plus));
        let context = text.lines().position(|l| l.starts_with("  alpha")).unwrap();
        assert_ne!(bg_at(&grid, 30, context), row_color(RowKind::Added));
    }

    #[test]
    fn non_ascii_rows_keep_the_tint_to_the_edge_and_the_changed_characters() {
        let mut screen = DiffScreen::new("é a\nb\n", "é a\nbé\n", "é a\nb\n");
        let (grid, text) = render_text(&mut screen, 200, 40);
        let row = cmp_row(&text, "bé").unwrap();
        let ours = columns_of(&text, row, "bé")[0];
        assert_eq!(bg_at(&grid, ours, row), row_color(RowKind::Added));
        assert_eq!(bg_at(&grid, ours + 12, row), row_color(RowKind::Added));
        assert_eq!(bg_at(&grid, ours - 2, row), row_color(RowKind::Added));
        assert_eq!(
            bg_at(&grid, ours + 1, row),
            tints().word(RowKind::Added).unwrap()
        );
    }

    #[test]
    fn resolving_a_hunk_tints_its_result_rows() {
        let mut screen = DiffScreen::new("a\nb\n", "a\nmine\n", "a\ntheirs\n");
        let (_, before) = render_text(&mut screen, 200, 40);
        assert!(before.contains("unresolved"));
        screen.run(MINE);
        let (grid, text) = render_text(&mut screen, 200, 40);
        let row = text
            .lines()
            .position(|line| line.matches("mine").count() >= 2)
            .unwrap();
        let result = *columns_of(&text, row, "mine").last().unwrap();
        assert_eq!(bg_at(&grid, result, row), row_color(RowKind::Added));
    }

    #[test]
    fn a_theme_palette_recolors_the_rows() {
        let themed = DiffPalette::from_colors("green = \"#00ff00\"\nred = \"#ff0000\"\n");
        let mut screen = ours_only().with_palette(themed);
        let (grid, text) = render_text(&mut screen, 200, 40);
        let row = cmp_row(&text, "WORLD").unwrap();
        let ours = columns_of(&text, row, "hello WORLD")[0];
        let want = themed
            .tints(Color::Rgb(30, 30, 46))
            .row(RowKind::Added)
            .unwrap();
        assert_eq!(bg_at(&grid, ours, row), want);
        assert_ne!(want, row_color(RowKind::Added));
    }

    #[test]
    fn filler_rows_keep_the_panes_the_same_height() {
        let screen = DiffScreen::new("a\nb\nc\n", "a\nb\nX\nY\nZ\n", "a\nb\nc\n");
        let counts = [
            screen.aligned.base.len(),
            screen.aligned.ours.len(),
            screen.aligned.theirs.len(),
            screen.aligned.result.len(),
            screen.aligned.duo_ours.len(),
            screen.aligned.duo_theirs.len(),
        ];
        assert!(counts.iter().all(|count| *count == counts[0]), "{counts:?}");
        let fillers = screen
            .aligned
            .base
            .iter()
            .filter(|line| line.kind == RowKind::Filler)
            .count();
        assert_eq!(fillers, 2);
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
                (200, 40) => include_str!("../snapshots/diff-200x40.txt"),
                (200, 60) => include_str!("../snapshots/diff-200x60.txt"),
                _ => unreachable!(),
            };
            assert_eq!(snap, expected, "{columns}x{rows}");
        }
    }

    #[test]
    fn four_panes_are_chosen_only_when_every_text_pane_is_useful() {
        let narrowest = |columns: u16| {
            let mut screen = changed();
            screen.pinned = Some(Layout::Four);
            let _ = render_once(&mut screen, columns, 40);
            screen.pane_widths()[1..].iter().copied().min().unwrap()
        };
        assert!(narrowest(WIDE) >= MIN_TEXT_PANE);
        assert!(narrowest(WIDE - 20) < MIN_TEXT_PANE);
        for (columns, expected) in [
            (120_u16, Layout::Two),
            (160, Layout::Two),
            (WIDE - 1, Layout::Two),
            (WIDE, Layout::Four),
            (240, Layout::Four),
        ] {
            let mut screen = changed();
            let _ = render_once(&mut screen, columns, 40);
            assert_eq!(screen.effective(), expected, "{columns}");
        }
    }

    #[test]
    fn the_layout_key_still_reaches_four_panes_below_the_automatic_width() {
        let mut screen = changed();
        let _ = render_once(&mut screen, 170, 40);
        assert_eq!(screen.effective(), Layout::Two);
        press(&mut screen, 'v');
        press(&mut screen, 'v');
        assert_eq!(screen.effective(), Layout::Four);
        let (grid, clusters) = render_once(&mut screen, 170, 40);
        assert!(grid.display_text(&clusters).contains("hunks"));
    }

    #[test]
    fn snapshots_show_narrower_layouts() {
        for (columns, rows) in [(80_u16, 24_u16), (100, 30), (120, 36), (160, 40)] {
            let mut screen = changed();
            let (grid, clusters) = render_once(&mut screen, columns, rows);
            let text = grid.display_text(&clusters);
            assert!(text.lines().next().unwrap().contains("Diff"), "{columns}");
            match (columns, rows) {
                (80, 24) => {
                    assert!(text.contains("stacked"), "{columns}");
                    assert!(text.contains("hello WORLD"), "{columns}");
                }
                (100, 30) | (120, 36) | (160, 40) => {
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
                (160, 40) => include_str!("../snapshots/diff-160x40.txt"),
                _ => unreachable!(),
            };
            assert_eq!(snap, expected, "{columns}x{rows}");
        }
    }

    #[test]
    fn the_measured_foot_float_uses_two_panes() {
        let columns = 121;
        let rows = 33;
        assert!(matches!(width_class(columns, rows), WidthClass::Medium));
        let mut screen = changed();
        let (grid, clusters) = render_once(&mut screen, columns, rows);
        assert!(matches!(screen.effective(), Layout::Two));
        let text = grid.display_text(&clusters);
        let header = text.lines().nth(1).unwrap_or("");
        assert!(
            header.contains("ours") && header.contains("theirs"),
            "{text}"
        );
        assert!(text.contains("result"), "{text}");
        assert!(!header.contains("hunks"), "{header}");
        assert!(!text.contains("stacked"), "{text}");
        assert!(text.contains("hello WORLD"), "{text}");
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
    fn a_choice_keeps_the_selected_stacked_hunk_visible() {
        let mut base = String::new();
        let mut ours = String::new();
        let mut theirs = String::new();
        for index in 0..30 {
            let old = format!("OLD{index:02}\n");
            let new = format!("NEW{index:02}\n");
            base.push_str(&old);
            ours.push_str(&old);
            theirs.push_str(&new);
        }
        for index in 0..30 {
            let line = format!("CTX{index:02}\n");
            base.push_str(&line);
            ours.push_str(&line);
            theirs.push_str(&line);
        }
        base.push_str("SECOND_OLD\n");
        ours.push_str("SECOND_OLD\n");
        theirs.push_str("SECOND_NEW\n");
        let mut screen = DiffScreen::new(&base, &ours, &theirs);
        let _ = frame(&mut screen);
        press(&mut screen, 'n');
        let jumped = frame(&mut screen);
        assert_eq!(screen.selected_hunk(), 1);
        let top = screen.scroll_top();
        assert!(jumped.contains("SECOND_OLD"), "{jumped}");
        assert!(jumped.contains("SECOND_NEW"), "{jumped}");
        press(&mut screen, 'o');
        let chosen = frame(&mut screen);
        assert_eq!(screen.selected_hunk(), 1);
        assert_eq!(screen.scroll_top(), top);
        let body = stacked_body(&chosen);
        assert!(
            body.iter().any(|line| line.contains("SECOND_OLD")),
            "{chosen}"
        );
        assert!(
            body.iter().any(|line| line.contains("= SECOND_OLD")),
            "{chosen}"
        );
    }

    #[test]
    fn stacked_choices_show_the_result_without_resizing() {
        let base = "alpha\nbase-line\nomega\n";
        let ours = "alpha\nours-line\nomega\n";
        let theirs = "alpha\ntheirs-line\nomega\n";
        let mut screen =
            DiffScreen::with_editor(base, ours, theirs, |_| Ok(String::from("CUSTOM_RESULT\n")));
        let _ = frame(&mut screen);
        press(&mut screen, 'o');
        let mine = frame(&mut screen);
        assert!(mine.contains("= ours-line"), "{mine}");
        assert_eq!(screen.result_text().unwrap(), ours);
        press(&mut screen, 't');
        let taken = frame(&mut screen);
        assert!(taken.contains("= theirs-line"), "{taken}");
        assert_eq!(screen.result_text().unwrap(), theirs);
        press(&mut screen, 'b');
        let both = frame(&mut screen);
        assert!(both.contains("= ours-line"), "{both}");
        assert!(both.contains("= theirs-line"), "{both}");
        assert_eq!(
            screen.result_text().unwrap(),
            "alpha\nours-line\ntheirs-line\nomega\n"
        );
        press(&mut screen, 'e');
        let edited = frame(&mut screen);
        assert!(edited.contains("= CUSTOM_RESULT"), "{edited}");
        assert_eq!(
            screen.result_text().unwrap(),
            "alpha\nCUSTOM_RESULT\nomega\n"
        );
    }

    #[test]
    fn width_picks_the_layout_and_keeps_the_hunk() {
        let mut screen = two_hunks();
        let _ = render_once(&mut screen, 200, 40);
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
        let (grid, clusters) = render_once(&mut screen, 240, 40);
        let wide = grid.display_text(&clusters);
        assert!(wide.contains("hunks"), "{wide}");
        screen.update(&Event::Key(KeyEvent::press(KeyCode::Char('v'))));
        let (grid, clusters) = render_once(&mut screen, 240, 40);
        let two = grid.display_text(&clusters);
        assert!(two.contains("ours") && two.contains("result"), "{two}");
        assert!(!two.lines().nth(1).unwrap_or("").contains("hunks"));
        screen.update(&Event::Key(KeyEvent::press(KeyCode::Char('v'))));
        let (grid, clusters) = render_once(&mut screen, 240, 40);
        assert!(grid.display_text(&clusters).contains("stacked"));
        screen.update(&Event::Key(KeyEvent::press(KeyCode::Char('v'))));
        let (grid, clusters) = render_once(&mut screen, 210, 40);
        assert!(grid.display_text(&clusters).contains("hunks"));
        screen.update(&Event::Key(KeyEvent::press(KeyCode::Char('v'))));
        let (grid, clusters) = render_once(&mut screen, 205, 40);
        assert!(grid.display_text(&clusters).contains("result"));
        let (grid, clusters) = render_once(&mut screen, 80, 24);
        assert!(grid.display_text(&clusters).contains("stacked"));
    }

    #[test]
    fn an_unknown_base_shows_two_panes_and_a_note() {
        let mut screen = DiffScreen::without_base("alpha\nhello\n", "alpha\nHELLO\n");
        let (grid, clusters) = render_once(&mut screen, 200, 40);
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
        let (once, once_clusters) = render_once(&mut marked, 200, 40);
        let (again, again_clusters) = render_once(&mut marked, 200, 40);
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
        let (scrolled, grid, clusters) = timed_frame(&mut changed, 200, 40);
        assert!(grid.display_text(&clusters).contains('B'));
        changed.update(&Event::Key(KeyEvent::press(KeyCode::Tab)));
        let (focused, _, _) = timed_frame(&mut changed, 200, 40);
        changed.update(&Event::Key(KeyEvent::press(KeyCode::Char('n'))));
        let (jumped, grid, clusters) = timed_frame(&mut changed, 200, 40);
        assert!(grid.display_text(&clusters).contains('C'));
        assert!(changed.scroll_top() > 0);
        let worst_cold = scrolled.max(focused).max(jumped);
        assert!(
            worst_cold < Duration::from_millis(8),
            "200x40 cold changed worst {worst_cold:?} scroll {scrolled:?} focus {focused:?} hunk {jumped:?}"
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
            let _ = render_once(&mut screen, 200, 40);
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

    fn press(screen: &mut DiffScreen, key: char) {
        screen.update(&Event::Key(KeyEvent::press(KeyCode::Char(key))));
    }

    fn click(screen: &mut DiffScreen, column: u16, row: u16) -> ControlFlow {
        screen.update(&Event::Mouse(MouseEvent::new(
            MouseKind::Down(MouseButton::Left),
            column,
            row,
        )))
    }

    fn state(screen: &DiffScreen, flow: ControlFlow) -> (bool, usize, usize, u8, bool, String) {
        let layout = match screen.effective() {
            Layout::Four => 4,
            Layout::Two => 2,
            Layout::Stacked => 1,
        };
        (
            flow == ControlFlow::Break,
            screen.hunk,
            screen.focus,
            layout,
            screen.plan.is_some(),
            format!("{:?}", screen.choices),
        )
    }

    #[test]
    fn nothing_a_click_can_ask_for_is_missing_from_the_keys() {
        for (columns, rows, step) in [(80_u16, 24_u16, 1_u16), (120, 36, 1), (170, 40, 2)] {
            let mut keys = std::collections::BTreeSet::new();
            let mut start = two_hunks();
            let _ = render_once(&mut start, columns, rows);
            keys.insert(state(&start, ControlFlow::Continue));
            for command in diff_keymap().commands() {
                for chord in command.chords() {
                    let mut screen = two_hunks();
                    let _ = render_once(&mut screen, columns, rows);
                    let event =
                        Event::Key(KeyEvent::press(chord.code()).modifiers(chord.modifiers()));
                    let flow = screen.update(&event);
                    keys.insert(state(&screen, flow));
                }
            }
            let mut moved = std::collections::BTreeSet::new();
            for row in 0..rows {
                for column in (0..columns).step_by(usize::from(step)) {
                    let mut screen = two_hunks();
                    let _ = render_once(&mut screen, columns, rows);
                    let flow = click(&mut screen, column, row);
                    moved.insert(state(&screen, flow));
                }
            }
            assert!(moved.len() >= 3, "{columns}: {moved:?}");
            let only_mouse: Vec<_> = moved.difference(&keys).collect();
            assert!(
                only_mouse.is_empty(),
                "{columns}: mouse-only {only_mouse:?}"
            );
        }
    }

    #[test]
    fn every_hint_has_a_key_the_keymap_or_the_screen_answers() {
        let keymap = diff_keymap();
        for (label, code) in HINTS {
            let global = matches!(code, KeyCode::Char('?' | ':'));
            let bound = keymap.dispatch(&crate::commands::press(*code)).is_some();
            assert!(global || bound, "{label}");
            let shown = format!("{}", KeyChord::new(*code, Modifiers::new()));
            assert!(label.starts_with(&format!("[{shown}]")), "{label}");
        }
    }

    #[test]
    fn clicking_a_hint_does_what_its_key_does_at_every_width() {
        for (columns, rows) in [(80_u16, 24_u16), (120, 36), (200, 40)] {
            let mut seen = 0;
            for (index, (label, code)) in HINTS.iter().enumerate() {
                if matches!(code, KeyCode::Char('?' | ':')) {
                    continue;
                }
                let mut clicked = two_hunks();
                let _ = render_once(&mut clicked, columns, rows);
                let hint = regions(Rect::new(0, 0, columns, rows)).2;
                let Some((rect, _)) = hint_cells(hint).into_iter().find(|(_, at)| *at == index)
                else {
                    continue;
                };
                seen += 1;
                let flow = click(&mut clicked, rect.x + rect.width / 2, rect.y);
                let by_click = state(&clicked, flow);
                let mut typed = two_hunks();
                let _ = render_once(&mut typed, columns, rows);
                let flow = typed.update(&crate::commands::press(*code));
                assert_eq!(by_click, state(&typed, flow), "{label} at {columns}");
            }
            assert!(seen >= 4, "{columns}: {seen}");
        }
    }

    #[test]
    fn the_hint_bar_shows_write_and_a_click_on_it_opens_the_plan() {
        let mut screen = changed();
        let (grid, clusters) = render_once(&mut screen, 80, 24);
        let text = grid.display_text(&clusters);
        let bar = text.lines().last().unwrap();
        assert!(bar.contains("[W] Write"), "{bar}");
        let column = u16::try_from(bar.find("[W]").unwrap()).unwrap();
        assert!(screen.plan.is_none());
        click(&mut screen, column + 2, 23);
        assert!(screen.plan.is_some());
    }

    #[test]
    fn the_pointer_works_in_the_stacked_and_two_pane_layouts() {
        for (columns, rows) in [(80_u16, 24_u16), (120, 36)] {
            let mut screen = long_file();
            let _ = render_once(&mut screen, columns, rows);
            assert_ne!(screen.effective(), Layout::Four);
            assert_eq!(screen.scroll_top(), 0);
            screen.update(&Event::Mouse(MouseEvent::new(
                MouseKind::Scroll(Scroll::Down),
                10,
                5,
            )));
            assert!(screen.scroll_top() > 0, "{columns}");
            let chip = screen
                .chips
                .iter()
                .find(|chip| chip.command == MINE)
                .unwrap();
            let (x, y) = (chip.x, chip.y);
            assert!(screen.choices.iter().all(Option::is_none));
            click(&mut screen, x + 1, y);
            assert!(screen.choices.iter().any(Option::is_some), "{columns}");
        }
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

    fn four_hunks() -> DiffScreen {
        let base = "a\n1\nb\n2\nc\n3\nd\n4\ne\n";
        let ours = "a\nO1\nb\nO2\nc\nO3\nd\nO4\ne\n";
        let theirs = "a\nT1\nb\n2\nc\nT3\nd\nT4\ne\n";
        DiffScreen::new(base, ours, theirs)
    }

    #[test]
    fn capital_n_and_p_visit_only_the_unresolved_conflicts() {
        let mut screen = four_hunks();
        assert_eq!(screen.hunk, 0);
        press(&mut screen, 'N');
        assert_eq!(screen.hunk, 2);
        press(&mut screen, 'N');
        assert_eq!(screen.hunk, 3);
        press(&mut screen, 'N');
        assert_eq!(screen.hunk, 3);
        assert_eq!(
            screen.notice(),
            Some("no unresolved conflict after this hunk")
        );
        press(&mut screen, 'P');
        assert_eq!(screen.hunk, 2);
        assert_eq!(screen.notice(), None);
        press(&mut screen, 'P');
        assert_eq!(screen.hunk, 0);
        press(&mut screen, 'P');
        assert_eq!(screen.hunk, 0);
        assert_eq!(
            screen.notice(),
            Some("no unresolved conflict before this hunk")
        );
    }

    #[test]
    fn a_resolved_conflict_is_skipped_by_the_conflict_keys() {
        let mut screen = four_hunks();
        press(&mut screen, 'N');
        press(&mut screen, 'o');
        press(&mut screen, 'p');
        press(&mut screen, 'P');
        assert_eq!(screen.hunk, 0);
        press(&mut screen, 'N');
        assert_eq!(screen.hunk, 3);
    }

    #[test]
    fn an_unresolved_conflict_blocks_write_and_says_why() {
        let (base, ours, theirs) = conflict_files();
        let mut screen = DiffScreen::new(base, ours, theirs);
        press(&mut screen, 'w');
        assert!(!screen.plan_open());
        assert_eq!(screen.notice(), Some("unresolved conflict blocks write"));
        let (grid, clusters) = render_once(&mut screen, 200, 40);
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
        let (grid, clusters) = render_once(&mut screen, 200, 40);
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
        let (grid, clusters) = render_once(&mut screen, 200, 40);
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
        let (grid, clusters) = render_once(&mut two, 200, 40);
        assert!(grid.display_text(&clusters).contains("base unknown"));
        let mut missing = DiffScreen::from_file_diff(&file_diff(
            DiffContent::Absent,
            DiffContent::Absent,
            content("new file\n"),
        ))
        .unwrap();
        let (grid, clusters) = render_once(&mut missing, 200, 40);
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

    #[test]
    fn every_command_in_the_diff_keymap_is_handled_and_an_unknown_one_is_not() {
        let mut screen = two_hunks();
        let _ = render_once(&mut screen, 200, 40);
        for command in diff_keymap().commands() {
            if command.id() == "diff.edit" || command.id() == "diff.write" {
                continue;
            }
            assert!(screen.run(command.id()).is_some(), "{}", command.id());
        }
        assert!(screen.run("diff.nope").is_none());
        assert_eq!(screen.run("diff.quit"), Some(ControlFlow::Break));
    }
}
