use std::ops::Range;

use similar::{capture_diff_slices, Algorithm, DiffOp};

use crate::error::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HunkKind {
    OursOnly,
    TheirsOnly,
    BothSame,
    Conflict,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    pub kind: HunkKind,
    pub base: String,
    pub ours: String,
    pub theirs: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    Stable(String),
    Hunk(Hunk),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    Mine,
    Theirs,
    Both,
    Custom(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Merge {
    segments: Vec<Segment>,
    terminator: &'static str,
}

#[derive(Debug, Clone)]
struct Region {
    base: Range<usize>,
    side: Range<usize>,
}

fn lines(text: &str) -> Vec<&str> {
    text.split_inclusive('\n').collect()
}

fn regions(base: &[&str], side: &[&str]) -> Vec<Region> {
    let mut found: Vec<Region> = Vec::new();
    let mut old = 0;
    let mut new = 0;
    for op in capture_diff_slices(Algorithm::Myers, base, side) {
        let (old_len, new_len) = match op {
            DiffOp::Equal { len, .. } => {
                old += len;
                new += len;
                continue;
            }
            DiffOp::Delete { old_len, .. } => (old_len, 0),
            DiffOp::Insert { new_len, .. } => (0, new_len),
            DiffOp::Replace {
                old_len, new_len, ..
            } => (old_len, new_len),
        };
        let base_range = old..old + old_len;
        let side_range = new..new + new_len;
        old += old_len;
        new += new_len;
        match found.last_mut() {
            Some(last)
                if last.base.end == base_range.start && last.side.end == side_range.start =>
            {
                last.base.end = base_range.end;
                last.side.end = side_range.end;
            }
            _ => found.push(Region {
                base: base_range,
                side: side_range,
            }),
        }
    }
    found
}

fn collides(a: &Range<usize>, b: &Range<usize>) -> bool {
    match (a.is_empty(), b.is_empty()) {
        (true, true) => a.start == b.start,
        (true, false) => b.start <= a.start && a.start < b.end,
        (false, true) => a.start <= b.start && b.start < a.end,
        (false, false) => a.start < b.end && b.start < a.end,
    }
}

struct Group {
    base: Range<usize>,
    ours: Vec<Region>,
    theirs: Vec<Region>,
}

fn group(ours: Vec<Region>, theirs: Vec<Region>) -> Vec<Group> {
    let mut tagged: Vec<(bool, Region)> = ours
        .into_iter()
        .map(|r| (true, r))
        .chain(theirs.into_iter().map(|r| (false, r)))
        .collect();
    tagged.sort_by_key(|(_, r)| (r.base.start, r.base.end));
    let mut groups: Vec<Group> = Vec::new();
    for (is_ours, region) in tagged {
        let joins = groups.last().is_some_and(|g| g.joins(&region.base));
        if joins {
            let last = groups.last_mut().unwrap();
            last.base.end = last.base.end.max(region.base.end);
            last.push(is_ours, region);
        } else {
            let mut fresh = Group {
                base: region.base.clone(),
                ours: Vec::new(),
                theirs: Vec::new(),
            };
            fresh.push(is_ours, region);
            groups.push(fresh);
        }
    }
    groups
}

impl Group {
    fn push(&mut self, is_ours: bool, region: Region) {
        if is_ours {
            self.ours.push(region);
        } else {
            self.theirs.push(region);
        }
    }

    fn joins(&self, base: &Range<usize>) -> bool {
        self.ours
            .iter()
            .chain(self.theirs.iter())
            .any(|r| collides(&r.base, base))
    }
}

fn side_text(base: &[&str], side: &[&str], span: &Range<usize>, regions: &[Region]) -> String {
    let mut out = String::new();
    let mut cursor = span.start;
    for region in regions {
        out.extend(base[cursor..region.base.start].iter().copied());
        out.extend(side[region.side.clone()].iter().copied());
        cursor = region.base.end;
    }
    out.extend(base[cursor..span.end].iter().copied());
    out
}

pub fn merge3(base: &str, ours: &str, theirs: &str) -> Merge {
    let b = lines(base);
    let o = lines(ours);
    let t = lines(theirs);
    let groups = group(regions(&b, &o), regions(&b, &t));
    let mut segments = Vec::new();
    let mut cursor = 0;
    for g in groups {
        if g.base.start > cursor {
            segments.push(Segment::Stable(b[cursor..g.base.start].concat()));
        }
        let base_text = b[g.base.clone()].concat();
        let ours_text = side_text(&b, &o, &g.base, &g.ours);
        let theirs_text = side_text(&b, &t, &g.base, &g.theirs);
        let kind = if g.theirs.is_empty() {
            HunkKind::OursOnly
        } else if g.ours.is_empty() {
            HunkKind::TheirsOnly
        } else if ours_text == theirs_text {
            HunkKind::BothSame
        } else {
            HunkKind::Conflict
        };
        segments.push(Segment::Hunk(Hunk {
            kind,
            base: base_text,
            ours: ours_text,
            theirs: theirs_text,
        }));
        cursor = g.base.end;
    }
    if cursor < b.len() {
        segments.push(Segment::Stable(b[cursor..].concat()));
    }
    Merge {
        segments,
        terminator: terminator_for(ours),
    }
}

fn terminator_for(text: &str) -> &'static str {
    if text.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

fn both(hunk: &Hunk, terminator: &str) -> String {
    let mut out = hunk.ours.clone();
    if !out.is_empty() && !hunk.theirs.is_empty() && !out.ends_with('\n') {
        out.push_str(terminator);
    }
    out.push_str(&hunk.theirs);
    out
}

impl Merge {
    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    pub fn hunks(&self) -> impl Iterator<Item = &Hunk> {
        self.segments.iter().filter_map(|s| match s {
            Segment::Hunk(h) => Some(h),
            Segment::Stable(_) => None,
        })
    }

    pub fn hunk_count(&self) -> usize {
        self.hunks().count()
    }

    pub fn conflict_count(&self) -> usize {
        self.hunks()
            .filter(|h| h.kind == HunkKind::Conflict)
            .count()
    }

    pub fn is_clean(&self) -> bool {
        self.conflict_count() == 0
    }

    pub fn render(&self, choices: &[Option<Choice>]) -> Result<String, Error> {
        let expected = self.hunk_count();
        if choices.len() != expected {
            return Err(Error::ChoiceCount {
                expected,
                got: choices.len(),
            });
        }
        let mut out = String::new();
        let mut index = 0;
        for segment in &self.segments {
            match segment {
                Segment::Stable(text) => out.push_str(text),
                Segment::Hunk(hunk) => {
                    match (&choices[index], hunk.kind) {
                        (Some(Choice::Mine), _) => out.push_str(&hunk.ours),
                        (Some(Choice::Theirs), _) => out.push_str(&hunk.theirs),
                        (Some(Choice::Both), _) => out.push_str(&both(hunk, self.terminator)),
                        (Some(Choice::Custom(text)), _) => out.push_str(text),
                        (None, HunkKind::TheirsOnly) => out.push_str(&hunk.theirs),
                        (None, HunkKind::OursOnly | HunkKind::BothSame) => out.push_str(&hunk.ours),
                        (None, HunkKind::Conflict) => return Err(Error::UnresolvedHunk(index)),
                    }
                    index += 1;
                }
            }
        }
        Ok(out)
    }

    pub fn auto(&self) -> Result<String, Error> {
        self.render(&vec![None; self.hunk_count()])
    }

    pub fn hunk_text(&self, index: usize, choice: Option<&Choice>) -> Option<String> {
        let hunk = self.hunks().nth(index)?;
        Some(match choice {
            Some(Choice::Mine) => hunk.ours.clone(),
            Some(Choice::Theirs) => hunk.theirs.clone(),
            Some(Choice::Both) => both(hunk, self.terminator),
            Some(Choice::Custom(text)) => text.clone(),
            None => match hunk.kind {
                HunkKind::TheirsOnly => hunk.theirs.clone(),
                HunkKind::OursOnly | HunkKind::BothSame => hunk.ours.clone(),
                HunkKind::Conflict => return None,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(m: &Merge) -> Vec<HunkKind> {
        m.hunks().map(|h| h.kind).collect()
    }

    fn auto(base: &str, ours: &str, theirs: &str) -> String {
        merge3(base, ours, theirs).auto().unwrap()
    }

    #[test]
    fn disjoint_edits_merge_cleanly_and_keep_both() {
        let m = merge3("a\nb\nc\nd\ne\n", "a\nB\nc\nd\ne\n", "a\nb\nc\nD\ne\n");
        assert_eq!(kinds(&m), [HunkKind::OursOnly, HunkKind::TheirsOnly]);
        assert!(m.is_clean());
        assert_eq!(m.auto().unwrap(), "a\nB\nc\nD\ne\n");
    }

    #[test]
    fn overlapping_edits_are_a_conflict_that_blocks_auto_render() {
        let m = merge3("a\nb\nc\n", "a\nours\nc\n", "a\ntheirs\nc\n");
        assert_eq!(kinds(&m), [HunkKind::Conflict]);
        assert!(!m.is_clean());
        assert_eq!(m.conflict_count(), 1);
        assert_eq!(m.auto(), Err(Error::UnresolvedHunk(0)));
        let hunk = m.hunks().next().unwrap();
        assert_eq!(hunk.base, "b\n");
        assert_eq!(hunk.ours, "ours\n");
        assert_eq!(hunk.theirs, "theirs\n");
    }

    #[test]
    fn adjacent_but_not_overlapping_edits_stay_separate_and_clean() {
        let m = merge3("a\nb\nc\nd\n", "a\nB\nc\nd\n", "a\nb\nC\nd\n");
        assert_eq!(kinds(&m), [HunkKind::OursOnly, HunkKind::TheirsOnly]);
        assert_eq!(m.auto().unwrap(), "a\nB\nC\nd\n");
    }

    #[test]
    fn inserts_at_the_same_point_conflict_unless_identical() {
        let m = merge3("a\nb\n", "a\nx\nb\n", "a\ny\nb\n");
        assert_eq!(kinds(&m), [HunkKind::Conflict]);
        let same = merge3("a\nb\n", "a\nx\nb\n", "a\nx\nb\n");
        assert_eq!(kinds(&same), [HunkKind::BothSame]);
        assert_eq!(same.auto().unwrap(), "a\nx\nb\n");
    }

    #[test]
    fn an_insert_inside_a_changed_range_conflicts() {
        let m = merge3("a\nb\nc\nd\n", "a\nB\nC\nd\n", "a\nb\nx\nc\nd\n");
        assert_eq!(kinds(&m), [HunkKind::Conflict]);
    }

    #[test]
    fn an_insert_at_the_start_of_a_changed_line_conflicts() {
        let m = merge3("a\nb\n", "a\nx\nb\n", "a\nB\n");
        assert_eq!(kinds(&m), [HunkKind::Conflict]);
        let swapped = merge3("a\nb\n", "a\nB\n", "a\nx\nb\n");
        assert_eq!(kinds(&swapped), [HunkKind::Conflict]);
    }

    #[test]
    fn an_insert_just_after_a_changed_line_is_adjacent_and_clean() {
        let m = merge3("a\nb\nc\n", "a\nB\nc\n", "a\nb\nx\nc\n");
        assert_eq!(kinds(&m), [HunkKind::OursOnly, HunkKind::TheirsOnly]);
        assert_eq!(m.auto().unwrap(), "a\nB\nx\nc\n");
    }

    #[test]
    fn identical_edits_are_both_same_and_apply_once() {
        let m = merge3("a\nb\nc\n", "a\nZ\nc\n", "a\nZ\nc\n");
        assert_eq!(kinds(&m), [HunkKind::BothSame]);
        assert_eq!(m.auto().unwrap(), "a\nZ\nc\n");
    }

    #[test]
    fn both_deleting_the_same_line_is_both_same() {
        let m = merge3("a\nb\nc\n", "a\nc\n", "a\nc\n");
        assert_eq!(kinds(&m), [HunkKind::BothSame]);
        assert_eq!(m.auto().unwrap(), "a\nc\n");
    }

    #[test]
    fn edits_at_the_first_line_and_the_last_line_merge() {
        let out = auto("a\nb\nc\n", "A\nb\nc\n", "a\nb\nC\n");
        assert_eq!(out, "A\nb\nC\n");
    }

    #[test]
    fn insert_before_the_first_line_and_after_the_last_line() {
        let m = merge3("a\nb\n", "top\na\nb\n", "a\nb\nbottom\n");
        assert_eq!(kinds(&m), [HunkKind::OursOnly, HunkKind::TheirsOnly]);
        assert_eq!(m.auto().unwrap(), "top\na\nb\nbottom\n");
    }

    #[test]
    fn crlf_lines_merge_without_normalising_endings() {
        let out = auto("a\r\nb\r\nc\r\n", "a\r\nB\r\nc\r\n", "a\r\nb\r\nC\r\n");
        assert_eq!(out, "a\r\nB\r\nC\r\n");
    }

    #[test]
    fn a_line_ending_change_is_a_real_edit() {
        let m = merge3("a\nb\n", "a\r\nb\n", "a\nb\nc\n");
        assert_eq!(kinds(&m), [HunkKind::OursOnly, HunkKind::TheirsOnly]);
        assert_eq!(m.auto().unwrap(), "a\r\nb\nc\n");
        let clash = merge3("a\nb\n", "a\r\nb\n", "A\nb\n");
        assert_eq!(kinds(&clash), [HunkKind::Conflict]);
    }

    #[test]
    fn missing_trailing_newline_is_preserved_per_side() {
        assert_eq!(auto("a\nb", "a\nb", "a\nb\nc"), "a\nb\nc");
        assert_eq!(auto("a\nb\n", "a\nb", "a\nb\n"), "a\nb");
        assert_eq!(auto("a\nb", "A\nb", "a\nb"), "A\nb");
    }

    #[test]
    fn both_adding_the_final_newline_is_both_same() {
        let m = merge3("a\nb", "a\nb\n", "a\nb\n");
        assert_eq!(kinds(&m), [HunkKind::BothSame]);
        assert_eq!(m.auto().unwrap(), "a\nb\n");
    }

    #[test]
    fn different_changes_to_the_unterminated_last_line_conflict() {
        let m = merge3("a\nb", "a\nb\n", "a\nbx");
        assert_eq!(kinds(&m), [HunkKind::Conflict]);
    }

    #[test]
    fn empty_files_in_every_position() {
        assert_eq!(auto("", "", ""), "");
        assert_eq!(auto("", "x\n", ""), "x\n");
        assert_eq!(auto("", "", "y\n"), "y\n");
        assert_eq!(auto("", "x\n", "x\n"), "x\n");
        assert_eq!(kinds(&merge3("", "x\n", "y\n")), [HunkKind::Conflict]);
        assert_eq!(auto("a\n", "", "a\n"), "");
        assert_eq!(auto("a\n", "a\n", ""), "");
        assert_eq!(auto("a\n", "", ""), "");
        assert_eq!(kinds(&merge3("a\n", "", "b\n")), [HunkKind::Conflict]);
        assert!(merge3("", "", "").segments().is_empty());
    }

    #[test]
    fn whitespace_only_changes_count_as_changes() {
        let m = merge3("a\nb\n", "a \nb\n", "a\nb \n");
        assert_eq!(kinds(&m), [HunkKind::OursOnly, HunkKind::TheirsOnly]);
        assert_eq!(m.auto().unwrap(), "a \nb \n");
        let clash = merge3("a\n", "a \n", "a\t\n");
        assert_eq!(kinds(&clash), [HunkKind::Conflict]);
        let indent = merge3("a\n", "  a\n", "a\n");
        assert_eq!(indent.auto().unwrap(), "  a\n");
    }

    #[test]
    fn ours_equal_to_base_yields_theirs_and_theirs_equal_to_base_yields_ours() {
        let texts = [
            "",
            "a\n",
            "a",
            "a\nb\nc\n",
            "a\r\nb\r\n",
            "x\n\ny\n",
            "a\nb\nc\nd\ne\nf\n",
            "f\ne\nd\n",
        ];
        for base in texts {
            for other in texts {
                assert_eq!(
                    auto(base, base, other),
                    other,
                    "O==B base={base:?} t={other:?}"
                );
                assert_eq!(
                    auto(base, other, base),
                    other,
                    "T==B base={base:?} o={other:?}"
                );
            }
        }
    }

    #[test]
    fn identical_sides_merge_to_that_side() {
        let texts = ["", "a\n", "a\nb", "a\r\nb\r\n", "q\nr\ns\n"];
        for base in texts {
            for side in texts {
                assert_eq!(auto(base, side, side), side, "base={base:?} side={side:?}");
            }
        }
    }

    #[test]
    fn stable_and_hunk_segments_reassemble_to_ours_theirs_and_base() {
        let base = "1\n2\n3\n4\n5\n6\n";
        let ours = "1\nO2\n3\n4\n5\n6\n";
        let theirs = "1\n2\n3\n4\nT5\n6\n";
        let m = merge3(base, ours, theirs);
        let mut b = String::new();
        let mut o = String::new();
        let mut t = String::new();
        for segment in m.segments() {
            match segment {
                Segment::Stable(text) => {
                    b.push_str(text);
                    o.push_str(text);
                    t.push_str(text);
                }
                Segment::Hunk(h) => {
                    b.push_str(&h.base);
                    o.push_str(&h.ours);
                    t.push_str(&h.theirs);
                }
            }
        }
        assert_eq!((b.as_str(), o.as_str(), t.as_str()), (base, ours, theirs));
    }

    #[test]
    fn choices_render_mine_theirs_both_and_custom() {
        let m = merge3("a\nb\nc\n", "a\nours\nc\n", "a\ntheirs\nc\n");
        let pick = |c| m.render(&[Some(c)]).unwrap();
        assert_eq!(pick(Choice::Mine), "a\nours\nc\n");
        assert_eq!(pick(Choice::Theirs), "a\ntheirs\nc\n");
        assert_eq!(pick(Choice::Both), "a\nours\ntheirs\nc\n");
        assert_eq!(
            pick(Choice::Custom("mine+yours\n".into())),
            "a\nmine+yours\nc\n"
        );
        assert_eq!(pick(Choice::Custom(String::new())), "a\nc\n");
    }

    #[test]
    fn both_inserts_a_line_break_when_mine_has_none() {
        let m = merge3("a\nb", "a\nX", "a\nY");
        assert_eq!(m.render(&[Some(Choice::Both)]).unwrap(), "a\nX\nY");
        let crlf = merge3("a\r\nb", "a\r\nX", "a\r\nY");
        assert_eq!(crlf.render(&[Some(Choice::Both)]).unwrap(), "a\r\nX\r\nY");
    }

    #[test]
    fn both_adds_no_separator_when_either_side_is_empty() {
        let theirs_deleted = merge3("base", "ours", "");
        assert_eq!(
            theirs_deleted.render(&[Some(Choice::Both)]).unwrap(),
            "ours"
        );
        let ours_deleted = merge3("base", "", "theirs");
        assert_eq!(
            ours_deleted.render(&[Some(Choice::Both)]).unwrap(),
            "theirs"
        );
        let crlf = merge3("a\r\nbase", "a\r\nours", "a\r\n");
        assert_eq!(crlf.render(&[Some(Choice::Both)]).unwrap(), "a\r\nours");
        let terminated = merge3("base\n", "ours\n", "");
        assert_eq!(terminated.render(&[Some(Choice::Both)]).unwrap(), "ours\n");
    }

    #[test]
    fn explicit_choices_override_the_automatic_side_of_clean_hunks() {
        let m = merge3("a\nb\nc\n", "a\nB\nc\n", "a\nb\nc\nd\n");
        assert_eq!(kinds(&m), [HunkKind::OursOnly, HunkKind::TheirsOnly]);
        let out = m
            .render(&[Some(Choice::Theirs), Some(Choice::Mine)])
            .unwrap();
        assert_eq!(out, "a\nb\nc\n");
    }

    #[test]
    fn render_rejects_a_wrong_number_of_choices_and_names_the_first_open_conflict() {
        let m = merge3("a\nb\nc\n", "A\nb\nC\n", "x\nb\ny\n");
        assert_eq!(kinds(&m), [HunkKind::Conflict, HunkKind::Conflict]);
        assert_eq!(
            m.render(&[None]),
            Err(Error::ChoiceCount {
                expected: 2,
                got: 1
            })
        );
        assert_eq!(
            m.render(&[Some(Choice::Mine), Some(Choice::Mine), None]),
            Err(Error::ChoiceCount {
                expected: 2,
                got: 3
            })
        );
        assert_eq!(
            m.render(&[Some(Choice::Mine), None]),
            Err(Error::UnresolvedHunk(1))
        );
        assert_eq!(
            m.render(&[None, Some(Choice::Mine)]),
            Err(Error::UnresolvedHunk(0))
        );
    }

    #[test]
    fn a_larger_realistic_merge_mixes_every_kind_in_order() {
        let base = "l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\nl9\n";
        let ours = "l1\nO2\nl3\nl4\nl5\nSAME\nl7\nOC\nl9\n";
        let theirs = "l1\nl2\nl3\nT4\nl5\nSAME\nl7\nTC\nl9\n";
        let m = merge3(base, ours, theirs);
        assert_eq!(
            kinds(&m),
            [
                HunkKind::OursOnly,
                HunkKind::TheirsOnly,
                HunkKind::BothSame,
                HunkKind::Conflict
            ]
        );
        let out = m.render(&[None, None, None, Some(Choice::Both)]).unwrap();
        assert_eq!(out, "l1\nO2\nl3\nT4\nl5\nSAME\nl7\nOC\nTC\nl9\n");
    }

    fn all_texts() -> Vec<String> {
        let mut out = vec![String::new()];
        let mut frontier = vec![String::new()];
        for _ in 0..3 {
            let mut next = Vec::new();
            for prefix in &frontier {
                for line in ["a\n", "b\n", "c"] {
                    if prefix.ends_with('c') {
                        continue;
                    }
                    next.push(format!("{prefix}{line}"));
                }
            }
            out.extend(next.iter().cloned());
            frontier = next;
        }
        out
    }

    #[test]
    fn every_small_triple_reassembles_and_honours_both_identities() {
        let texts = all_texts();
        assert!(texts.len() > 20);
        for base in &texts {
            for ours in &texts {
                for theirs in &texts {
                    let m = merge3(base, ours, theirs);
                    let (mut b, mut o, mut t) = (String::new(), String::new(), String::new());
                    for segment in m.segments() {
                        match segment {
                            Segment::Stable(x) => {
                                b.push_str(x);
                                o.push_str(x);
                                t.push_str(x);
                            }
                            Segment::Hunk(h) => {
                                b.push_str(&h.base);
                                o.push_str(&h.ours);
                                t.push_str(&h.theirs);
                            }
                        }
                    }
                    let ctx = format!("{base:?} {ours:?} {theirs:?}");
                    assert_eq!((&b, &o, &t), (base, ours, theirs), "{ctx}");
                    if ours == base {
                        assert_eq!(m.auto().as_deref(), Ok(theirs.as_str()), "{ctx}");
                    }
                    if theirs == base {
                        assert_eq!(m.auto().as_deref(), Ok(ours.as_str()), "{ctx}");
                    }
                    if ours == theirs {
                        assert_eq!(m.auto().as_deref(), Ok(ours.as_str()), "{ctx}");
                    }
                    assert!(m
                        .hunks()
                        .all(|h| h.kind != HunkKind::BothSame || h.ours == h.theirs));
                    assert!(m
                        .hunks()
                        .all(|h| h.kind != HunkKind::Conflict || h.ours != h.theirs));
                }
            }
        }
    }
}
