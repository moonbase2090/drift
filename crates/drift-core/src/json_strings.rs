use std::borrow::Cow;

pub fn comparison_text<'a>(path: &str, text: &'a str) -> Cow<'a, str> {
    if !path.ends_with(".json") && !path.ends_with(".jsonc") {
        return Cow::Borrowed(text);
    }
    normalize(text).map_or(Cow::Borrowed(text), Cow::Owned)
}

fn normalize(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut copied = 0;
    let mut at = 0;
    while at < bytes.len() {
        match bytes[at] {
            b'/' if bytes.get(at + 1) == Some(&b'/') => {
                at += 2;
                while at < bytes.len() && bytes[at] != b'\n' {
                    at += 1;
                }
            }
            b'/' if bytes.get(at + 1) == Some(&b'*') => {
                at += 2;
                while at + 1 < bytes.len() && &bytes[at..at + 2] != b"*/" {
                    at += 1;
                }
                if at + 1 >= bytes.len() {
                    return None;
                }
                at += 2;
            }
            b'"' => {
                let start = at;
                at += 1;
                loop {
                    match bytes.get(at)? {
                        b'\\' => at += 2,
                        b'"' => {
                            at += 1;
                            break;
                        }
                        _ => at += 1,
                    }
                }
                let decoded: String = serde_json::from_str(&text[start..at]).ok()?;
                out.push_str(&text[copied..start]);
                out.push_str(&serde_json::to_string(&decoded).ok()?);
                copied = at;
            }
            _ => at += 1,
        }
    }
    out.push_str(&text[copied..]);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_escapes_share_a_key_without_changing_structure_or_comments() {
        for (escaped, literal) in [
            (r#"{"x":"\u2014"}"#, "{\"x\":\"—\"}"),
            (r#"{"\u0061":"\uD83D\uDE80"}"#, "{\"a\":\"🚀\"}"),
            (r#"{"x":"a\/b"}"#, r#"{"x":"a/b"}"#),
            (r#"{"x":"\u000a"}"#, r#"{"x":"\n"}"#),
        ] {
            assert_eq!(
                comparison_text("a.json", escaped),
                comparison_text("a.json", literal)
            );
        }
        let comments = "// \"\\u2014\"\n/* \"\\u2014\" */\n{\"x\":\"\\u2014\"}\n";
        assert_eq!(
            comparison_text("a.jsonc", comments),
            "// \"\\u2014\"\n/* \"\\u2014\" */\n{\"x\":\"—\"}\n"
        );
    }

    #[test]
    fn invalid_escapes_literal_backslashes_and_non_json_stay_distinct() {
        for text in [
            r#"{"x":"\uD800"}"#,
            r#"{"x":"\q"}"#,
            "{\"x\":\"unfinished",
            "/*",
        ] {
            assert_eq!(comparison_text("a.json", text), text);
        }
        assert_ne!(
            comparison_text("a.json", r#"{"x":"\\u2014"}"#),
            comparison_text("a.json", "{\"x\":\"—\"}")
        );
        assert_ne!(
            comparison_text("a.json", "{\"x\":1}"),
            comparison_text("a.json", "{\"x\":1.0}")
        );
        assert_ne!(
            comparison_text("a.json", "{\"x\":1}"),
            comparison_text("a.json", "{ \"x\":1}")
        );
        assert_eq!(
            comparison_text("a.conf", r#"{"x":"\u2014"}"#),
            r#"{"x":"\u2014"}"#
        );
    }
}
