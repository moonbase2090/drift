use std::path::PathBuf;

use drift_core::{merge3, DiffRows, LineEnding, Row};

fn render_row(row: &Row) -> String {
    let n = |v: Option<usize>| v.map_or("   ".to_string(), |v| format!("{v:>3}"));
    let ending = match row.ending {
        LineEnding::Lf => "",
        LineEnding::CrLf => " [crlf]",
        LineEnding::None => " [no eol]",
    };
    let mut out = format!(
        "{}|{}|{} {} {}{}\n",
        n(row.numbers.base),
        n(row.numbers.ours),
        n(row.numbers.theirs),
        row.mark.glyph(),
        row.text,
        ending
    );
    if !row.spans.is_empty() {
        let mut marks = vec![' '; row.width];
        for span in &row.spans {
            for cell in &mut marks[span.start..span.end] {
                *cell = '~';
            }
        }
        let line: String = marks.into_iter().collect();
        out.push_str(&format!("{}{}\n", " ".repeat(14), line.trim_end()));
    }
    out
}

fn render(base: &str, ours: &str, theirs: &str) -> String {
    let rows = DiffRows::build(&merge3(base, ours, theirs));
    rows.rows().iter().map(render_row).collect()
}

fn snapshot(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/snapshots")
        .join(format!("{name}.txt"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn check(name: &str, base: &str, ours: &str, theirs: &str) {
    let got = render(base, ours, theirs);
    let want = snapshot(name);
    assert_eq!(got, want, "snapshot {name}\n--- got ---\n{got}");
}

#[test]
fn context_only() {
    check("context-only", "a\nb\nc\n", "a\nb\nc\n", "a\nb\nc\n");
}

#[test]
fn ours_only() {
    check(
        "ours-only",
        "a\nkeep = 1\nc\n",
        "a\nkeep = 10\nc\nd\n",
        "a\nkeep = 1\nc\n",
    );
}

#[test]
fn theirs_only() {
    check(
        "theirs-only",
        "one\ntwo\nthree\nfour\n",
        "one\ntwo\nthree\nfour\n",
        "one\ntwo\nthree\nfour\nfive\n",
    );
}

#[test]
fn both_same() {
    check(
        "both-same",
        "x\nsize = 8\ny\n",
        "x\nsize = 12\ny\n",
        "x\nsize = 12\ny\n",
    );
}

#[test]
fn conflict() {
    check(
        "conflict",
        "top\ngap_in = 5\ngap_out = 10\nend\n",
        "top\ngap_in = 5\ngap_out = 12\nend\n",
        "top\ngap_in = 5\ngap_out = 14\nend\n",
    );
}

#[test]
fn mixed_kinds_keep_numbers_per_side() {
    check(
        "mixed",
        "1\n2\n3\n4\n5\n6\n7\n",
        "1\nO2\n3\n4\n5\nSAME\n7\nextra\n",
        "1\n2\n3\nT4\n5\nSAME\n7\n",
    );
}

#[test]
fn tabs_align_to_tab_stops() {
    check(
        "tabs",
        "\tindent\n\t\tdeep\nx\ttab\n",
        "\tindent\n\t\tdeeper\nx\ttab\n",
        "\tindent\n\t\tdeep\nx\ttab\n",
    );
}

#[test]
fn graphemes_and_wide_characters_align() {
    check(
        "unicode",
        "caf\u{e9} \u{65e5}\u{672c} ok\n",
        "cafe\u{301} \u{65e5}\u{672c}\u{8a9e} ok\n",
        "caf\u{e9} \u{65e5}\u{672c} ok\n",
    );
}

#[test]
fn line_endings_and_missing_final_newline_are_visible() {
    check(
        "endings",
        "a\r\nb\r\nlast",
        "a\r\nB\r\nlast",
        "a\r\nb\r\nlast\n",
    );
}

#[test]
fn control_characters_are_replaced_not_passed_through() {
    let rows = DiffRows::build(&merge3("a\n", "a\u{1b}[31m\n", "a\n"));
    let text: Vec<&str> = rows.rows().iter().map(|r| r.text.as_str()).collect();
    assert!(text.iter().all(|t| !t.contains('\u{1b}')), "{text:?}");
    assert!(text.iter().any(|t| t.contains('\u{fffd}')));
}
