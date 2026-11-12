use similar::TextDiff;

use crate::error::Error;

pub const MENU_PATH: &str = "extensions/omarchy-menu.jsonc";
pub const HOOK_PATH: &str = "hooks/post-update.d/drift";
pub const MENU_ID: &str = "update.drift";
pub const HOOK_MODE: u32 = 0o755;

const HOOK_SCRIPT: &str = "#!/bin/bash\ndrift notify\n";

fn row() -> String {
    format!(
        "\"{MENU_ID}\": {{\"icon\": \"\\uf0ec\", \"label\": \"Drift\", \"description\": \"Review configuration drift after an update\", \"action\": \"omarchy-launch-or-focus-tui --app-id=TUI.float drift\"}}"
    )
}

fn fail(message: impl Into<String>) -> Error {
    Error::Integration {
        message: message.into(),
    }
}

pub fn hook_content(current: Option<&str>) -> Result<Option<String>, Error> {
    match current {
        None => Ok(Some(HOOK_SCRIPT.to_string())),
        Some(text) if text == HOOK_SCRIPT => Ok(None),
        Some(_) => Err(fail(format!(
            "{HOOK_PATH} already exists with other content"
        ))),
    }
}

pub fn add_menu_row(current: Option<&str>) -> Result<Option<String>, Error> {
    let Some(text) = current.filter(|text| !text.trim().is_empty()) else {
        return Ok(Some(format!("{{\n  {}\n}}\n", row())));
    };
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let scan = scan(text)?;
    if scan.keys.iter().any(|key| key == MENU_ID) {
        return Ok(None);
    }
    let mut out = String::with_capacity(text.len() + 256);
    let line_start = text[..scan.close]
        .rfind('\n')
        .map(|at| at + 1)
        .filter(|start| text[*start..scan.close].trim().is_empty());
    let insert_at = line_start.unwrap_or(scan.close);
    let comma_at = match scan.last {
        Last::Value(at) => Some(at + 1),
        Last::Open | Last::Comma => None,
    };
    let mut cursor = 0;
    if let Some(at) = comma_at {
        out.push_str(&text[..at]);
        out.push(',');
        cursor = at;
    }
    out.push_str(&text[cursor..insert_at]);
    match line_start {
        Some(_) => {
            out.push_str("  ");
            out.push_str(&row());
            out.push_str(newline);
        }
        None => {
            out.push_str(newline);
            out.push_str("  ");
            out.push_str(&row());
            out.push_str(newline);
        }
    }
    out.push_str(&text[insert_at..]);
    Ok(Some(out))
}

pub fn unified_diff(path: &str, before: Option<&str>, after: &str) -> String {
    let old = before.unwrap_or("");
    let diff = TextDiff::from_lines(old, after);
    let from = if before.is_some() {
        format!("a/{path}")
    } else {
        "/dev/null".to_string()
    };
    let to = format!("b/{path}");
    let mut text = diff
        .unified_diff()
        .context_radius(3)
        .header(&from, &to)
        .to_string();
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text
}

enum Last {
    Open,
    Comma,
    Value(usize),
}

struct Scan {
    keys: Vec<String>,
    close: usize,
    last: Last,
}

const NOT_AN_OBJECT: &str = "the menu file is not a JSON object";
const UNBALANCED: &str = "unbalanced braces in the menu file";

struct Scanner<'a> {
    text: &'a str,
    bytes: &'a [u8],
    index: usize,
    depth: usize,
    started: bool,
    expect_key: bool,
    keys: Vec<String>,
    last: Last,
    close: Option<usize>,
}

fn scan(text: &str) -> Result<Scan, Error> {
    let mut scanner = Scanner {
        text,
        bytes: text.as_bytes(),
        index: 0,
        depth: 0,
        started: false,
        expect_key: false,
        keys: Vec::new(),
        last: Last::Open,
        close: None,
    };
    while scanner.index < scanner.bytes.len() {
        scanner.step()?;
    }
    match scanner.close {
        Some(close) if scanner.depth == 0 => Ok(Scan {
            keys: scanner.keys,
            close,
            last: scanner.last,
        }),
        _ => Err(fail(UNBALANCED)),
    }
}

fn hex4(chars: &mut std::str::Chars<'_>) -> Result<u32, Error> {
    let digits: String = chars.by_ref().take(4).collect();
    if digits.len() != 4 {
        return Err(bad_escape());
    }
    u32::from_str_radix(&digits, 16).map_err(|_| bad_escape())
}

fn bad_escape() -> Error {
    fail("bad escape in a key of the menu file")
}

fn decode_unicode(chars: &mut std::str::Chars<'_>) -> Result<char, Error> {
    let unit = hex4(chars)?;
    let code = if (0xd800..0xdc00).contains(&unit) {
        if chars.next() != Some('\\') || chars.next() != Some('u') {
            return Err(bad_escape());
        }
        let low = hex4(chars)?;
        if !(0xdc00..0xe000).contains(&low) {
            return Err(bad_escape());
        }
        0x10000 + ((unit - 0xd800) << 10) + (low - 0xdc00)
    } else {
        unit
    };
    char::from_u32(code).ok_or_else(bad_escape)
}

fn decode_key(raw: &str) -> Result<String, Error> {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        let decoded = match chars.next() {
            Some(c @ ('"' | '\\' | '/')) => c,
            Some('b') => '\u{8}',
            Some('f') => '\u{c}',
            Some('n') => '\n',
            Some('r') => '\r',
            Some('t') => '\t',
            Some('u') => decode_unicode(&mut chars)?,
            _ => return Err(bad_escape()),
        };
        out.push(decoded);
    }
    Ok(out)
}

impl Scanner<'_> {
    fn step(&mut self) -> Result<(), Error> {
        let byte = self.bytes[self.index];
        if matches!(byte, b' ' | b'\t' | b'\r' | b'\n' | b':') {
            self.index += 1;
            return Ok(());
        }
        if byte == b'/' && self.skip_comment()? {
            return Ok(());
        }
        if self.close.is_some() {
            return Err(fail("text after the closing brace of the menu file"));
        }
        if byte != b'{' && !self.started {
            return Err(fail(NOT_AN_OBJECT));
        }
        match byte {
            b'"' => self.string(),
            b'{' | b'[' => self.open(byte),
            b'}' | b']' => self.shut(byte),
            b',' => {
                self.comma();
                Ok(())
            }
            _ => {
                self.scalar();
                Ok(())
            }
        }
    }

    fn skip_comment(&mut self) -> Result<bool, Error> {
        match self.bytes.get(self.index + 1) {
            Some(b'/') => {
                while self.index < self.bytes.len() && self.bytes[self.index] != b'\n' {
                    self.index += 1;
                }
                Ok(true)
            }
            Some(b'*') => match self.text[self.index + 2..].find("*/") {
                Some(end) => {
                    self.index += end + 4;
                    Ok(true)
                }
                None => Err(fail("unterminated comment in the menu file")),
            },
            _ => Ok(false),
        }
    }

    fn string(&mut self) -> Result<(), Error> {
        let start = self.index + 1;
        self.index = start;
        loop {
            match self.bytes.get(self.index) {
                None => return Err(fail("unterminated string in the menu file")),
                Some(b'\\') => self.index += 2,
                Some(b'"') => break,
                Some(_) => self.index += 1,
            }
        }
        if self.depth == 1 && self.expect_key {
            self.keys.push(decode_key(&self.text[start..self.index])?);
            self.expect_key = false;
        }
        self.last = Last::Value(self.index);
        self.index += 1;
        Ok(())
    }

    fn open(&mut self, byte: u8) -> Result<(), Error> {
        if !self.started {
            if byte != b'{' {
                return Err(fail(NOT_AN_OBJECT));
            }
            self.started = true;
            self.expect_key = true;
        }
        self.last = Last::Open;
        self.depth += 1;
        self.index += 1;
        Ok(())
    }

    fn shut(&mut self, byte: u8) -> Result<(), Error> {
        if self.depth == 0 {
            return Err(fail(UNBALANCED));
        }
        self.depth -= 1;
        if self.depth > 0 {
            self.last = Last::Value(self.index);
        } else if byte == b'}' {
            self.close = Some(self.index);
        } else {
            return Err(fail(UNBALANCED));
        }
        self.index += 1;
        Ok(())
    }

    fn comma(&mut self) {
        if self.depth == 1 {
            self.expect_key = true;
        }
        self.last = Last::Comma;
        self.index += 1;
    }

    fn scalar(&mut self) {
        self.index += 1;
        while self.index < self.bytes.len()
            && !matches!(
                self.bytes[self.index],
                b' ' | b'\t' | b'\r' | b'\n' | b',' | b'}' | b']' | b'/' | b':'
            )
        {
            self.index += 1;
        }
        self.last = Last::Value(self.index - 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEMPLATE: &str =
        "{\n  // Extend the menu.\n  //\n  // \"about\": {\"label\": \"About\"},\n}\n";

    fn added(text: &str) -> String {
        add_menu_row(Some(text)).unwrap().unwrap()
    }

    fn row_line() -> String {
        format!("  {}\n", row())
    }

    #[test]
    fn the_stock_file_gains_the_row_before_the_closing_brace() {
        let out = added(TEMPLATE);
        assert_eq!(
            out,
            format!("{{\n  // Extend the menu.\n  //\n  // \"about\": {{\"label\": \"About\"}},\n{}}}\n", row_line())
        );
    }

    #[test]
    fn existing_entries_keep_their_text_and_order_and_get_a_comma() {
        let text =
            "{\n  \"a.b\": {\"label\": \"B\", \"x\": [1, 2, {\"y\": 3}]},\n  \"c\": 1 // note\n}\n";
        let out = added(text);
        assert_eq!(
            out,
            format!("{{\n  \"a.b\": {{\"label\": \"B\", \"x\": [1, 2, {{\"y\": 3}}]}},\n  \"c\": 1, // note\n{}}}\n", row_line())
        );
    }

    #[test]
    fn a_trailing_comma_is_not_doubled() {
        let out = added("{\n  \"a\": \"v\",\n}\n");
        assert_eq!(out, format!("{{\n  \"a\": \"v\",\n{}}}\n", row_line()));
    }

    #[test]
    fn scalars_and_nested_values_before_the_brace_get_the_comma_after_them() {
        for (value, expected) in [
            ("12", "12,"),
            ("true", "true,"),
            ("null", "null,"),
            ("[1]", "[1],"),
            ("{}", "{},"),
            ("\"s\"", "\"s\","),
        ] {
            let text = format!("{{\n  \"k\": {value}\n}}");
            let out = added(&text);
            assert!(
                out.starts_with(&format!("{{\n  \"k\": {expected}\n")),
                "{out}"
            );
            assert!(out.ends_with("}"), "{out}");
        }
    }

    #[test]
    fn a_row_in_the_file_makes_it_a_no_op() {
        let once = added(TEMPLATE);
        assert_eq!(add_menu_row(Some(&once)).unwrap(), None);
        let own = "{ \"update.drift\": {\"label\": \"Mine\"} }";
        assert_eq!(add_menu_row(Some(own)).unwrap(), None);
    }

    #[test]
    fn the_id_is_found_only_as_a_top_level_key() {
        for text in [
            "{\n  // \"update.drift\": {}\n}",
            "{ \"a\": {\"update.drift\": 1} }",
            "{ \"a\": \"update.drift\" }",
            "{ \"a\": [\"update.drift\"] }",
            "{ /* \"update.drift\": 1 */ }",
        ] {
            assert!(add_menu_row(Some(text)).unwrap().is_some(), "{text}");
        }
    }

    #[test]
    fn strings_comments_and_escapes_do_not_confuse_the_scan() {
        let text = "{\n  \"a\": \"}\\\"{,\", // } ,\n  /* } */ \"b\": \"x\"\n}";
        let out = added(text);
        assert!(out.contains("\"b\": \"x\",\n  \"update.drift\""), "{out}");
        assert!(out.starts_with("{\n  \"a\": \"}\\\"{,\", // } ,\n  /* } */ \"b\": \"x\","));
    }

    #[test]
    fn empty_and_one_line_objects_work() {
        assert_eq!(added("{}"), format!("{{\n  {}\n}}", row()));
        assert_eq!(added("{ }"), format!("{{ \n  {}\n}}", row()));
        assert_eq!(added("{\"a\": 1}"), format!("{{\"a\": 1,\n  {}\n}}", row()));
    }

    #[test]
    fn crlf_files_keep_crlf() {
        let out = added("{\r\n  \"a\": 1\r\n}\r\n");
        assert!(out.contains("\"a\": 1,\r\n  \"update.drift\""), "{out:?}");
        assert!(!out.replace("\r\n", "").contains('\n'), "{out:?}");
    }

    #[test]
    fn an_absent_or_blank_file_is_created() {
        let fresh = format!("{{\n  {}\n}}\n", row());
        assert_eq!(add_menu_row(None).unwrap().unwrap(), fresh);
        assert_eq!(add_menu_row(Some(" \n")).unwrap().unwrap(), fresh);
        assert_eq!(add_menu_row(Some(&fresh)).unwrap(), None);
    }

    #[test]
    fn the_result_is_valid_once_comments_are_stripped() {
        let out = added(TEMPLATE);
        let stripped: String = out
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        let value: serde_json::Value = serde_json::from_str(&stripped).unwrap();
        let entry = &value[MENU_ID];
        assert_eq!(entry["label"], "Drift");
        assert_eq!(entry["icon"], "\u{f0ec}");
        assert_eq!(
            entry["action"],
            "omarchy-launch-or-focus-tui --app-id=TUI.float drift"
        );
    }

    #[test]
    fn malformed_files_are_refused_not_edited() {
        for text in [
            "[1]",
            "\"x\"",
            "42",
            "{",
            "}",
            "{ \"a\": \"open }",
            "{ /* open }",
            "{} {}",
            "{} x",
            "{ \"a\": [1 }",
            "{ ] }",
            "a { }",
            ", {}",
        ] {
            let error = add_menu_row(Some(text)).unwrap_err();
            assert!(matches!(error, Error::Integration { .. }), "{text}");
        }
    }

    #[test]
    fn trailing_comments_after_the_object_are_fine() {
        let out = added("{}\n// end\n");
        assert!(out.ends_with("}\n// end\n"), "{out}");
    }

    #[test]
    fn the_hook_is_created_kept_or_refused() {
        assert_eq!(hook_content(None).unwrap().unwrap(), HOOK_SCRIPT);
        assert_eq!(hook_content(Some(HOOK_SCRIPT)).unwrap(), None);
        let error = hook_content(Some("#!/bin/sh\necho hi\n")).unwrap_err();
        assert!(error.to_string().contains("already exists"), "{error}");
        assert!(HOOK_SCRIPT.starts_with("#!/bin/bash\n"));
        assert!(HOOK_SCRIPT.contains("drift notify"));
        assert_eq!(HOOK_MODE, 0o755);
    }

    #[test]
    fn the_diff_is_a_unified_diff_with_headers() {
        let after = added(TEMPLATE);
        let diff = unified_diff(MENU_PATH, Some(TEMPLATE), &after);
        assert!(
            diff.starts_with(
                "--- a/extensions/omarchy-menu.jsonc\n+++ b/extensions/omarchy-menu.jsonc\n@@"
            ),
            "{diff}"
        );
        assert!(diff.contains(&format!("+{}", row_line())), "{diff}");
        assert!(!diff.contains("\n-"), "{diff}");
        let created = unified_diff(HOOK_PATH, None, HOOK_SCRIPT);
        assert!(
            created.starts_with("--- /dev/null\n+++ b/hooks/post-update.d/drift\n"),
            "{created}"
        );
        assert!(
            created.contains("+#!/bin/bash\n+drift notify\n"),
            "{created}"
        );
    }

    #[test]
    fn an_escaped_spelling_of_the_id_counts_as_present() {
        for key in [
            "update\\u002edrift",
            "update\\u002Edrift",
            "\\u0075pdate.drift",
            "update\\u002e\\u0064rift",
            "update.drif\\u0074",
        ] {
            let text = format!("{{\n  \"{key}\": {{\"label\": \"Mine\"}}\n}}\n");
            assert_eq!(add_menu_row(Some(&text)).unwrap(), None, "{key}");
        }
    }

    #[test]
    fn other_escapes_in_keys_are_decoded_and_bad_ones_refused() {
        let text = "{ \"a\\\"b\": 1, \"c\\/d\": 2, \"\\ud83d\\ude00\": 3 }";
        assert!(add_menu_row(Some(text)).unwrap().is_some());
        for bad in [
            "{ \"update\\x2edrift\": 1 }",
            "{ \"update\\u00zzdrift\": 1 }",
            "{ \"update\\u00\": 1 }",
            "{ \"update\\ud83d\": 1 }",
            "{ \"update\\ude00\": 1 }",
            "{ \"a\\ud83d\\u0041\": 1 }",
            "{ \"a\\ud83dx\": 1 }",
        ] {
            assert!(add_menu_row(Some(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_similar_looking_key_is_not_the_id() {
        for key in ["update\\u002fdrift", "update.drift\\u0020", "Update.drift"] {
            let text = format!("{{ \"{key}\": 1 }}");
            assert!(add_menu_row(Some(&text)).unwrap().is_some(), "{key}");
        }
    }

    #[test]
    fn keys_decode_to_their_text() {
        assert_eq!(decode_key("\\ud83d\\ude00").unwrap(), "\u{1f600}");
        assert_eq!(
            decode_key("\\u00e9\\n\\t\\\\\\\"").unwrap(),
            "\u{e9}\n\t\\\""
        );
        assert_eq!(decode_key("plain é").unwrap(), "plain é");
    }
}
