use std::borrow::Cow;
use std::collections::{BTreeMap, VecDeque};
use std::ops::Range;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Role {
    Key,
    Value,
}

struct StringToken {
    span: Range<usize>,
    decoded: String,
    role: Role,
}

fn is_json(path: &str) -> bool {
    path.ends_with(".json") || path.ends_with(".jsonc")
}

pub fn comparison_text<'a>(path: &str, text: &'a str) -> Cow<'a, str> {
    if !is_json(path) {
        return Cow::Borrowed(text);
    }
    normalize(text).map_or(Cow::Borrowed(text), Cow::Owned)
}

fn trivia(bytes: &[u8], mut at: usize) -> Option<usize> {
    loop {
        while bytes.get(at).is_some_and(u8::is_ascii_whitespace) {
            at += 1;
        }
        if bytes.get(at..at + 2) == Some(b"//") {
            at += 2;
            while at < bytes.len() && bytes[at] != b'\n' {
                at += 1;
            }
        } else if bytes.get(at..at + 2) == Some(b"/*") {
            at += 2;
            while at + 1 < bytes.len() && &bytes[at..at + 2] != b"*/" {
                at += 1;
            }
            if at + 1 >= bytes.len() {
                return None;
            }
            at += 2;
        } else {
            return Some(at);
        }
    }
}

fn strings(text: &str) -> Option<Vec<StringToken>> {
    let bytes = text.as_bytes();
    let mut found = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        at = trivia(bytes, at)?;
        if bytes.get(at) != Some(&b'"') {
            at += 1;
            continue;
        }
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
        let decoded = serde_json::from_str(&text[start..at]).ok()?;
        let role = if bytes.get(trivia(bytes, at)?) == Some(&b':') {
            Role::Key
        } else {
            Role::Value
        };
        found.push(StringToken {
            span: start..at,
            decoded,
            role,
        });
    }
    Some(found)
}

fn normalize(text: &str) -> Option<String> {
    let mut out = String::with_capacity(text.len());
    let mut copied = 0;
    for token in strings(text)? {
        out.push_str(&text[copied..token.span.start]);
        out.push_str(&serde_json::to_string(&token.decoded).ok()?);
        copied = token.span.end;
    }
    out.push_str(&text[copied..]);
    Some(out)
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum Part {
    Field(String),
    Index(usize),
}

#[derive(Default)]
struct Paths(BTreeMap<(usize, Part), usize>);

impl Paths {
    fn child(&mut self, parent: usize, part: Part) -> usize {
        let next = self.0.len() + 1;
        *self.0.entry((parent, part)).or_insert(next)
    }
}

enum Frame {
    Object { path: usize, key: Option<usize> },
    Array { path: usize, index: usize },
}

fn value_path(stack: &[Frame], paths: &mut Paths) -> Option<usize> {
    match stack.last() {
        None => Some(0),
        Some(Frame::Object { key, .. }) => *key,
        Some(Frame::Array { path, index }) => Some(paths.child(*path, Part::Index(*index))),
    }
}

fn located_strings(text: &str, paths: &mut Paths) -> Option<Vec<(usize, StringToken)>> {
    let mut tokens = strings(text)?.into_iter().peekable();
    let mut stack = Vec::new();
    let mut located = Vec::new();
    let bytes = text.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        at = trivia(bytes, at)?;
        if tokens.peek().is_some_and(|token| token.span.start == at) {
            let token = tokens.next()?;
            let path = if token.role == Role::Key {
                let Frame::Object { path, key } = stack.last_mut()? else {
                    return None;
                };
                let field = paths.child(*path, Part::Field(token.decoded.clone()));
                *key = Some(field);
                field
            } else {
                value_path(&stack, paths)?
            };
            at = token.span.end;
            located.push((path, token));
            continue;
        }
        match bytes.get(at) {
            Some(b'{') => stack.push(Frame::Object {
                path: value_path(&stack, paths)?,
                key: None,
            }),
            Some(b'[') => stack.push(Frame::Array {
                path: value_path(&stack, paths)?,
                index: 0,
            }),
            Some(b'}') => {
                if !matches!(stack.pop(), Some(Frame::Object { .. })) {
                    return None;
                }
            }
            Some(b']') => {
                if !matches!(stack.pop(), Some(Frame::Array { .. })) {
                    return None;
                }
            }
            Some(b',') => match stack.last_mut()? {
                Frame::Object { key, .. } => *key = None,
                Frame::Array { index, .. } => *index += 1,
            },
            _ => {}
        }
        at += 1;
    }
    stack.is_empty().then_some(located)
}

pub(crate) fn preserve_local_strings<'a>(path: &str, ours: &str, theirs: &'a str) -> Cow<'a, str> {
    if !is_json(path) {
        return Cow::Borrowed(theirs);
    }
    let mut paths = Paths::default();
    let (Some(local), Some(upstream)) = (
        located_strings(ours, &mut paths),
        located_strings(theirs, &mut paths),
    ) else {
        return Cow::Borrowed(theirs);
    };
    let mut lexemes: BTreeMap<(Role, usize), VecDeque<(String, &str)>> = BTreeMap::new();
    for (path, token) in local {
        lexemes
            .entry((token.role, path))
            .or_default()
            .push_back((token.decoded, &ours[token.span.clone()]));
    }
    let mut out = String::new();
    let mut copied = 0;
    for (path, token) in upstream {
        let original = lexemes
            .get_mut(&(token.role, path))
            .and_then(VecDeque::pop_front)
            .filter(|(decoded, _)| *decoded == token.decoded)
            .map(|(_, raw)| raw);
        if let Some(original) = original.filter(|raw| *raw != &theirs[token.span.clone()]) {
            out.push_str(&theirs[copied..token.span.start]);
            out.push_str(original);
            copied = token.span.end;
        }
    }
    if copied == 0 {
        Cow::Borrowed(theirs)
    } else {
        out.push_str(&theirs[copied..]);
        Cow::Owned(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_lexemes_survive_number_edits_without_changing_upstream_values() {
        let local = r#"{"\u0061":"\uD83D\uDE80","slash":"a\/b","n":1}"#;
        let upstream = r#"{"a":"🚀","slash":"a/b","n":2}"#;
        let expected = r#"{"\u0061":"\uD83D\uDE80","slash":"a\/b","n":2}"#;
        let result = preserve_local_strings("a.json", local, upstream);
        assert_eq!(result, expected);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&result).unwrap(),
            serde_json::from_str::<serde_json::Value>(upstream).unwrap()
        );
        assert_eq!(preserve_local_strings("a.conf", local, upstream), upstream);
        let changed = r#"{"a":"different","slash":"new","n":2}"#;
        assert_eq!(
            preserve_local_strings("a.json", local, changed),
            r#"{"\u0061":"different","slash":"new","n":2}"#
        );
    }

    #[test]
    fn equal_values_only_reuse_lexemes_at_the_same_structural_path() {
        let local = r#"{"a":"\u2014","b":"—","nested":{"b":"\u2014"},"n":1}"#;
        let upstream = r#"{"n":2,"nested":{"b":"—"},"b":"—","a":"different"}"#;
        let expected = r#"{"n":2,"nested":{"b":"\u2014"},"b":"—","a":"different"}"#;
        assert_eq!(preserve_local_strings("a.json", local, upstream), expected);
    }

    #[test]
    fn lexeme_retention_keeps_roles_occurrences_comments_and_invalid_strings_distinct() {
        assert_eq!(
            preserve_local_strings("a.json", r#"{"\u0061":"a"}"#, r#"{"x":"a"}"#),
            r#"{"x":"a"}"#
        );
        assert_eq!(
            preserve_local_strings("a.json", r#"["\u0061","a"]"#, r#"["a","a","a"]"#),
            r#"["\u0061","a","a"]"#
        );
        assert_eq!(
            preserve_local_strings(
                "a.jsonc",
                "// \"\\u2014\"\n{\"a\":\"\\u2014\"}",
                "// \"—\"\n{\"a\":\"—\"}"
            ),
            "// \"—\"\n{\"a\":\"\\u2014\"}"
        );
        let invalid = r#"{"a":"\u2014","bad":"\q"}"#;
        let literal = r#"{"a":"—","bad":"\q"}"#;
        assert_eq!(preserve_local_strings("a.json", invalid, literal), literal);
    }

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
