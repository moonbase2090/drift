pub struct Scan {
    pub code: String,
    pub comment_lines: Vec<usize>,
}

pub fn scan(source: &str) -> Scan {
    let chars: Vec<char> = source.chars().collect();
    let mut code = String::with_capacity(source.len());
    let mut comment_lines = Vec::new();
    let mut line = 1usize;
    let mut i = 0usize;

    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        match (c, next) {
            ('/', Some('/')) => {
                comment_lines.push(line);
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            ('/', Some('*')) => {
                comment_lines.push(line);
                let mut depth = 1;
                i += 2;
                while i < chars.len() && depth > 0 {
                    if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
                        depth += 1;
                        i += 2;
                    } else if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                        depth -= 1;
                        i += 2;
                    } else {
                        if chars[i] == '\n' {
                            line += 1;
                            code.push('\n');
                        }
                        i += 1;
                    }
                }
            }
            ('"', _) => {
                i = skip_string(&chars, i + 1, &mut line, &mut code);
                code.push_str("\"\"");
            }
            ('r', _) | ('b', _) if is_raw_start(&chars, i) => {
                let (hashes, open) = raw_open(&chars, i);
                i = skip_raw(&chars, open, hashes, &mut line, &mut code);
                code.push_str("\"\"");
            }
            ('b', Some('"')) => {
                i = skip_string(&chars, i + 2, &mut line, &mut code);
                code.push_str("\"\"");
            }
            ('\'', _) => {
                if let Some(end) = char_literal_end(&chars, i) {
                    i = end;
                    code.push_str("''");
                } else {
                    code.push(c);
                    i += 1;
                }
            }
            ('\n', _) => {
                line += 1;
                code.push('\n');
                i += 1;
            }
            _ => {
                code.push(c);
                i += 1;
            }
        }
    }

    Scan {
        code,
        comment_lines,
    }
}

fn skip_string(chars: &[char], mut i: usize, line: &mut usize, code: &mut String) -> usize {
    while i < chars.len() {
        match chars[i] {
            '\\' => {
                if chars.get(i + 1) == Some(&'\n') {
                    *line += 1;
                    code.push('\n');
                }
                i += 2;
            }
            '"' => return i + 1,
            '\n' => {
                *line += 1;
                code.push('\n');
                i += 1;
            }
            _ => i += 1,
        }
    }
    i
}

fn is_raw_start(chars: &[char], i: usize) -> bool {
    if i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '_') {
        return false;
    }
    let mut j = i;
    if chars[j] == 'b' {
        j += 1;
    }
    if chars.get(j) != Some(&'r') {
        return false;
    }
    j += 1;
    while chars.get(j) == Some(&'#') {
        j += 1;
    }
    chars.get(j) == Some(&'"')
}

fn raw_open(chars: &[char], i: usize) -> (usize, usize) {
    let mut j = i;
    if chars[j] == 'b' {
        j += 1;
    }
    j += 1;
    let mut hashes = 0;
    while chars[j] == '#' {
        hashes += 1;
        j += 1;
    }
    (hashes, j + 1)
}

fn skip_raw(
    chars: &[char],
    mut i: usize,
    hashes: usize,
    line: &mut usize,
    code: &mut String,
) -> usize {
    while i < chars.len() {
        if chars[i] == '"' && (0..hashes).all(|k| chars.get(i + 1 + k) == Some(&'#')) {
            return i + 1 + hashes;
        }
        if chars[i] == '\n' {
            *line += 1;
            code.push('\n');
        }
        i += 1;
    }
    i
}

fn char_literal_end(chars: &[char], i: usize) -> Option<usize> {
    match (chars.get(i + 1), chars.get(i + 2)) {
        (Some('\\'), _) => {
            let mut j = i + 2;
            while j < chars.len() && chars[j] != '\'' && j < i + 12 {
                j += 1;
            }
            (chars.get(j) == Some(&'\'')).then_some(j + 1)
        }
        (Some(c), Some('\'')) if *c != '\'' => Some(i + 3),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_line_comments() {
        let s = scan("fn a() {}\n// note\nfn b() {}\n");
        assert_eq!(s.comment_lines, vec![2]);
    }

    #[test]
    fn finds_doc_and_block_comments() {
        let s = scan("/// doc\n//! inner\n/* block\nmore */\nfn a() {}\n");
        assert_eq!(s.comment_lines, vec![1, 2, 3]);
    }

    #[test]
    fn finds_trailing_comment() {
        let s = scan("let x = 1; // trailing\n");
        assert_eq!(s.comment_lines, vec![1]);
    }

    #[test]
    fn nested_block_comments_end_correctly() {
        let s = scan("/* a /* b */ c */\nfn a() {}\n");
        assert_eq!(s.comment_lines, vec![1]);
        assert!(s.code.contains("fn a()"));
    }

    #[test]
    fn slashes_in_strings_are_not_comments() {
        let s = scan("let u = \"https://example.com\";\n");
        assert!(s.comment_lines.is_empty());
    }

    #[test]
    fn escaped_quote_stays_in_string() {
        let s = scan("let u = \"a \\\" // b\";\n");
        assert!(s.comment_lines.is_empty());
    }

    #[test]
    fn raw_strings_are_skipped() {
        let s = scan("let u = r#\"// not \"a\" comment\"#;\nlet v = 1;\n");
        assert!(s.comment_lines.is_empty());
        assert!(s.code.contains("let v"));
    }

    #[test]
    fn lifetimes_and_char_literals_are_handled() {
        let s = scan("fn a<'a>(x: &'a str) -> char { '/' }\nfn b() {}\n");
        assert!(s.comment_lines.is_empty());
        assert!(s.code.contains("fn b()"));
    }

    #[test]
    fn string_contents_are_blanked_from_code() {
        let s = scan("let p = \"fs::write\";\n");
        assert!(!s.code.contains("fs::write"));
    }

    #[test]
    fn line_numbers_survive_multiline_strings() {
        let s = scan("let a = \"x\ny\";\n// c\n");
        assert_eq!(s.comment_lines, vec![3]);
    }
}
