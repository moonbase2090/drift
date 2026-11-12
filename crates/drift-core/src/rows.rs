use std::collections::{HashMap, VecDeque};
use std::hash::Hash;
use std::ops::Range;
use std::sync::Arc;

use similar::{capture_diff_slices, Algorithm, DiffOp};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::merge::{Hunk, HunkKind, Merge, Segment};

pub const TAB_WIDTH: usize = 4;
pub const MAX_SPAN_GRAPHEMES: usize = 2000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    Context,
    Removed,
    Added,
    Mine,
    Theirs,
}

impl Mark {
    pub fn glyph(self) -> char {
        match self {
            Mark::Context => ' ',
            Mark::Removed => '-',
            Mark::Added => '+',
            Mark::Mine => '<',
            Mark::Theirs => '>',
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEnding {
    Lf,
    CrLf,
    None,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LineNumbers {
    pub base: Option<usize>,
    pub ours: Option<usize>,
    pub theirs: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub mark: Mark,
    pub numbers: LineNumbers,
    pub text: String,
    pub width: usize,
    pub ending: LineEnding,
    pub spans: Vec<Span>,
    pub hunk: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffRows {
    rows: Vec<Row>,
    hunk_starts: Vec<usize>,
}

struct Cells {
    text: String,
    bounds: Vec<(usize, usize)>,
    graphemes: Vec<String>,
}

fn split_ending(line: &str) -> (&str, LineEnding) {
    if let Some(body) = line.strip_suffix("\r\n") {
        (body, LineEnding::CrLf)
    } else if let Some(body) = line.strip_suffix('\n') {
        (body, LineEnding::Lf)
    } else {
        (line, LineEnding::None)
    }
}

fn cells(line: &str) -> Cells {
    let mut text = String::new();
    let mut bounds = Vec::new();
    let mut graphemes = Vec::new();
    let mut column = 0;
    for grapheme in line.graphemes(true) {
        let start = column;
        if grapheme == "\t" {
            let next = (column / TAB_WIDTH + 1) * TAB_WIDTH;
            text.extend(std::iter::repeat_n(' ', next - column));
            column = next;
        } else if grapheme.chars().any(char::is_control) {
            for _ in grapheme.chars() {
                text.push('\u{fffd}');
                column += 1;
            }
        } else {
            text.push_str(grapheme);
            column += grapheme.width();
        }
        bounds.push((start, column));
        graphemes.push(grapheme.to_string());
    }
    Cells {
        text,
        bounds,
        graphemes,
    }
}

fn changed_ranges(old: &[String], new: &[String]) -> (Vec<Range<usize>>, Vec<Range<usize>>) {
    if old.len() > MAX_SPAN_GRAPHEMES || new.len() > MAX_SPAN_GRAPHEMES {
        return (Vec::new(), Vec::new());
    }
    let mut removed = Vec::new();
    let mut added = Vec::new();
    let (mut o, mut n) = (0, 0);
    for op in capture_diff_slices(Algorithm::Myers, old, new) {
        let (old_len, new_len) = match op {
            DiffOp::Equal { len, .. } => {
                o += len;
                n += len;
                continue;
            }
            DiffOp::Delete { old_len, .. } => (old_len, 0),
            DiffOp::Insert { new_len, .. } => (0, new_len),
            DiffOp::Replace {
                old_len, new_len, ..
            } => (old_len, new_len),
        };
        if old_len > 0 {
            removed.push(o..o + old_len);
        }
        if new_len > 0 {
            added.push(n..n + new_len);
        }
        o += old_len;
        n += new_len;
    }
    (removed, added)
}

fn spans(cells: &Cells, ranges: &[Range<usize>]) -> Vec<Span> {
    let mut out: Vec<Span> = Vec::new();
    for range in ranges {
        let start = cells.bounds[range.start].0;
        let end = cells.bounds[range.end - 1].1;
        if start == end {
            continue;
        }
        out.push(Span { start, end });
    }
    out
}

type LineRanges = Vec<Vec<Range<usize>>>;

struct Counters {
    base: usize,
    ours: usize,
    theirs: usize,
}

struct Line<'a> {
    body: &'a str,
    ending: LineEnding,
}

fn split(text: &str) -> Vec<Line<'_>> {
    text.split_inclusive('\n')
        .map(|line| {
            let (body, ending) = split_ending(line);
            Line { body, ending }
        })
        .collect()
}

fn row(
    mark: Mark,
    numbers: LineNumbers,
    line: &Line<'_>,
    spans_for: Option<&[Range<usize>]>,
    hunk: Option<usize>,
) -> Row {
    let c = cells(line.body);
    let spans = spans_for.map_or_else(Vec::new, |ranges| spans(&c, ranges));
    let width = c.bounds.last().map_or(0, |b| b.1);
    Row {
        mark,
        numbers,
        text: c.text,
        width,
        ending: line.ending,
        spans,
        hunk,
    }
}

fn pair_ranges(left: &[Line<'_>], right: &[Line<'_>]) -> (LineRanges, LineRanges) {
    let mut left_out = vec![Vec::new(); left.len()];
    let mut right_out = vec![Vec::new(); right.len()];
    for i in 0..left.len().min(right.len()) {
        let a = cells(left[i].body);
        let b = cells(right[i].body);
        let (removed, added) = changed_ranges(&a.graphemes, &b.graphemes);
        left_out[i] = removed;
        right_out[i] = added;
    }
    (left_out, right_out)
}

impl DiffRows {
    pub fn build(merge: &Merge) -> DiffRows {
        let mut rows = Vec::new();
        let mut hunk_starts = Vec::new();
        let mut count = Counters {
            base: 1,
            ours: 1,
            theirs: 1,
        };
        let mut index = 0;
        for segment in merge.segments() {
            match segment {
                Segment::Stable(text) => {
                    for line in split(text) {
                        let numbers = LineNumbers {
                            base: Some(count.base),
                            ours: Some(count.ours),
                            theirs: Some(count.theirs),
                        };
                        rows.push(row(Mark::Context, numbers, &line, None, None));
                        count.base += 1;
                        count.ours += 1;
                        count.theirs += 1;
                    }
                }
                Segment::Hunk(hunk) => {
                    hunk_starts.push(rows.len());
                    push_hunk(&mut rows, &mut count, hunk, index);
                    index += 1;
                }
            }
        }
        DiffRows { rows, hunk_starts }
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn window(&self, range: Range<usize>) -> &[Row] {
        let end = range.end.min(self.rows.len());
        let start = range.start.min(end);
        &self.rows[start..end]
    }

    pub fn hunk_row(&self, hunk: usize) -> Option<usize> {
        self.hunk_starts.get(hunk).copied()
    }

    pub fn hunk_count(&self) -> usize {
        self.hunk_starts.len()
    }
}

fn push_hunk(rows: &mut Vec<Row>, count: &mut Counters, hunk: &Hunk, index: usize) {
    let base = split(&hunk.base);
    let ours = split(&hunk.ours);
    let theirs = split(&hunk.theirs);
    let tag = Some(index);
    match hunk.kind {
        HunkKind::Conflict => {
            let (mine_spans, their_spans) = pair_ranges(&ours, &theirs);
            for (i, line) in ours.iter().enumerate() {
                let numbers = LineNumbers {
                    ours: Some(count.ours + i),
                    ..LineNumbers::default()
                };
                rows.push(row(Mark::Mine, numbers, line, Some(&mine_spans[i]), tag));
            }
            for (i, line) in theirs.iter().enumerate() {
                let numbers = LineNumbers {
                    theirs: Some(count.theirs + i),
                    ..LineNumbers::default()
                };
                rows.push(row(Mark::Theirs, numbers, line, Some(&their_spans[i]), tag));
            }
        }
        kind => {
            let (changed, side_number) = match kind {
                HunkKind::TheirsOnly => (&theirs, count.theirs),
                _ => (&ours, count.ours),
            };
            let (removed_spans, added_spans) = pair_ranges(&base, changed);
            for (i, line) in base.iter().enumerate() {
                let numbers = LineNumbers {
                    base: Some(count.base + i),
                    ..LineNumbers::default()
                };
                rows.push(row(
                    Mark::Removed,
                    numbers,
                    line,
                    Some(&removed_spans[i]),
                    tag,
                ));
            }
            for (i, line) in changed.iter().enumerate() {
                let mut numbers = LineNumbers::default();
                if kind == HunkKind::TheirsOnly {
                    numbers.theirs = Some(side_number + i);
                } else {
                    numbers.ours = Some(side_number + i);
                }
                rows.push(row(Mark::Added, numbers, line, Some(&added_spans[i]), tag));
            }
        }
    }
    count.base += base.len();
    count.ours += ours.len();
    count.theirs += theirs.len();
}

pub struct RowCache<K> {
    capacity: usize,
    entries: HashMap<K, Arc<DiffRows>>,
    order: VecDeque<K>,
    builds: usize,
}

impl<K: Hash + Eq + Clone> RowCache<K> {
    pub fn new(capacity: usize) -> RowCache<K> {
        RowCache {
            capacity: capacity.max(1),
            entries: HashMap::new(),
            order: VecDeque::new(),
            builds: 0,
        }
    }

    pub fn rows(&mut self, key: &K, merge: impl FnOnce() -> Merge) -> Arc<DiffRows> {
        if let Some(found) = self.entries.get(key) {
            return Arc::clone(found);
        }
        self.builds += 1;
        let built = Arc::new(DiffRows::build(&merge()));
        if self.entries.len() >= self.capacity {
            if let Some(oldest) = self.order.pop_front() {
                self.entries.remove(&oldest);
            }
        }
        self.entries.insert(key.clone(), Arc::clone(&built));
        self.order.push_back(key.clone());
        built
    }

    pub fn builds(&self) -> usize {
        self.builds
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::merge::merge3;
    use std::cell::Cell;

    fn rows(base: &str, ours: &str, theirs: &str) -> DiffRows {
        DiffRows::build(&merge3(base, ours, theirs))
    }

    #[test]
    fn tabs_expand_to_the_next_tab_stop() {
        let c = cells("a\tb\t\tc");
        assert_eq!(c.text, "a   b       c");
        assert_eq!(c.bounds[1], (1, 4));
        assert_eq!(c.bounds[3], (5, 8));
        let aligned = cells("abcd\te");
        assert_eq!(aligned.text, "abcd    e");
    }

    #[test]
    fn wide_characters_take_two_columns_and_combining_marks_none() {
        let wide = cells("\u{65e5}x");
        assert_eq!(wide.bounds, [(0, 2), (2, 3)]);
        let combined = cells("e\u{301}x");
        assert_eq!(combined.graphemes.len(), 2);
        assert_eq!(combined.bounds, [(0, 1), (1, 2)]);
        let family = cells("\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}x");
        assert_eq!(family.graphemes.len(), 2);
        assert_eq!(family.bounds[1].1 - family.bounds[1].0, 1);
    }

    #[test]
    fn spacing_marks_stay_in_the_same_grapheme() {
        let c = cells("\u{915}\u{903}x");
        assert_eq!(c.graphemes.len(), 2);
    }

    #[test]
    fn control_characters_take_one_visible_column_each() {
        let c = cells("a\u{1b}b");
        assert_eq!(c.text, "a\u{fffd}b");
        assert_eq!(c.bounds, [(0, 1), (1, 2), (2, 3)]);
    }

    #[test]
    fn spans_use_display_columns_after_tabs_and_wide_characters() {
        let d = rows("\t\u{65e5}a\n", "\t\u{65e5}b\n", "\t\u{65e5}a\n");
        let added = d.rows().iter().find(|r| r.mark == Mark::Added).unwrap();
        assert_eq!(added.text, "    \u{65e5}b");
        assert_eq!(added.spans, [Span { start: 6, end: 7 }]);
        let removed = d.rows().iter().find(|r| r.mark == Mark::Removed).unwrap();
        assert_eq!(removed.spans, [Span { start: 6, end: 7 }]);
    }

    #[test]
    fn a_changed_wide_character_spans_both_columns() {
        let d = rows("a\u{65e5}\n", "a\u{672c}\n", "a\u{65e5}\n");
        let added = d.rows().iter().find(|r| r.mark == Mark::Added).unwrap();
        assert_eq!(added.spans, [Span { start: 1, end: 3 }]);
    }

    #[test]
    fn adjacent_changed_graphemes_merge_into_one_span() {
        let d = rows("abcdef\n", "abXYef\n", "abcdef\n");
        let added = d.rows().iter().find(|r| r.mark == Mark::Added).unwrap();
        assert_eq!(added.spans, [Span { start: 2, end: 4 }]);
    }

    #[test]
    fn whitespace_only_changes_are_highlighted() {
        let d = rows("a b\n", "a  b\n", "a b\n");
        let added = d.rows().iter().find(|r| r.mark == Mark::Added).unwrap();
        assert_eq!(added.spans.len(), 1);
        assert_eq!(added.spans[0].end - added.spans[0].start, 1);
        assert!((1..=2).contains(&added.spans[0].start));
        let trailing = rows("a\n", "a  \n", "a\n");
        let added = trailing
            .rows()
            .iter()
            .find(|r| r.mark == Mark::Added)
            .unwrap();
        assert_eq!(added.spans, [Span { start: 1, end: 3 }]);
    }

    #[test]
    fn unpaired_lines_carry_no_spans() {
        let d = rows("a\n", "a\nnew\n", "a\n");
        let added = d.rows().iter().find(|r| r.mark == Mark::Added).unwrap();
        assert!(added.spans.is_empty());
    }

    #[test]
    fn very_long_lines_skip_span_work() {
        let long_a = "a".repeat(MAX_SPAN_GRAPHEMES + 1);
        let long_b = format!("{}b", "a".repeat(MAX_SPAN_GRAPHEMES));
        let d = rows(
            &format!("{long_a}\n"),
            &format!("{long_b}\n"),
            &format!("{long_a}\n"),
        );
        assert!(d.rows().iter().all(|r| r.spans.is_empty()));
    }

    #[test]
    fn line_numbers_advance_per_side_through_hunks() {
        let d = rows("1\n2\n3\n", "1\nA\nB\n3\n", "1\n2\n3\n");
        let seen: Vec<(Mark, LineNumbers)> = d.rows().iter().map(|r| (r.mark, r.numbers)).collect();
        let n = |b, o, t| LineNumbers {
            base: b,
            ours: o,
            theirs: t,
        };
        assert_eq!(
            seen,
            [
                (Mark::Context, n(Some(1), Some(1), Some(1))),
                (Mark::Removed, n(Some(2), None, None)),
                (Mark::Added, n(None, Some(2), None)),
                (Mark::Added, n(None, Some(3), None)),
                (Mark::Context, n(Some(3), Some(4), Some(3))),
            ]
        );
    }

    #[test]
    fn hunk_rows_are_indexed_and_tagged() {
        let d = rows("a\nb\nc\nd\ne\n", "a\nB\nc\nd\ne\n", "a\nb\nc\nD\ne\n");
        assert_eq!(d.hunk_count(), 2);
        let first = d.hunk_row(0).unwrap();
        let second = d.hunk_row(1).unwrap();
        assert!(first < second);
        assert_eq!(d.rows()[first].hunk, Some(0));
        assert_eq!(d.rows()[first].mark, Mark::Removed);
        assert_eq!(d.rows()[second].mark, Mark::Removed);
        assert_eq!(d.rows()[first - 1].mark, Mark::Context);
        assert_eq!(d.rows()[second].hunk, Some(1));
        assert_eq!(d.hunk_row(2), None);
        assert!(d
            .rows()
            .iter()
            .filter(|r| r.mark == Mark::Context)
            .all(|r| r.hunk.is_none()));
    }

    #[test]
    fn empty_input_has_no_rows() {
        let d = rows("", "", "");
        assert!(d.is_empty());
        assert_eq!(d.len(), 0);
        assert_eq!(d.hunk_count(), 0);
    }

    #[test]
    fn window_clamps_to_the_available_rows() {
        let d = rows("a\nb\nc\n", "a\nb\nc\n", "a\nb\nc\n");
        assert_eq!(d.window(1..2).len(), 1);
        assert_eq!(d.window(1..99).len(), 2);
        assert_eq!(d.window(50..99).len(), 0);
        assert_eq!(d.window(Range { start: 2, end: 1 }).len(), 0);
    }

    #[test]
    fn scrolling_hits_the_cache_without_recomputing() {
        let mut cache: RowCache<u64> = RowCache::new(4);
        let computed = Cell::new(0);
        let make = || {
            computed.set(computed.get() + 1);
            merge3("a\nb\nc\n", "a\nB\nc\n", "a\nb\nc\n")
        };
        let first = cache.rows(&1, make);
        for offset in 0..10 {
            let again = cache.rows(&1, || panic!("recomputed on scroll {offset}"));
            assert!(Arc::ptr_eq(&first, &again));
            let _ = again.window(offset..offset + 2);
        }
        assert_eq!(computed.get(), 1);
        assert_eq!(cache.builds(), 1);
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn a_new_key_builds_again_and_the_oldest_entry_is_evicted() {
        let mut cache: RowCache<u64> = RowCache::new(2);
        let build = |text: &'static str| move || merge3(text, text, text);
        cache.rows(&1, build("a\n"));
        cache.rows(&2, build("b\n"));
        cache.rows(&3, build("c\n"));
        assert_eq!(cache.builds(), 3);
        assert_eq!(cache.len(), 2);
        cache.rows(&3, || panic!("kept"));
        cache.rows(&2, || panic!("kept"));
        cache.rows(&1, build("a\n"));
        assert_eq!(cache.builds(), 4);
        assert!(!cache.is_empty());
    }

    #[test]
    fn a_zero_capacity_cache_still_holds_one_entry() {
        let mut cache: RowCache<u64> = RowCache::new(0);
        cache.rows(&1, || merge3("a\n", "a\n", "a\n"));
        cache.rows(&1, || panic!("kept"));
        assert_eq!(cache.len(), 1);
    }
}
