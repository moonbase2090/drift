use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Ident(String),
    Colons,
    Punct(char),
}

struct Token {
    tok: Tok,
    line: usize,
}

struct Import {
    path: Vec<String>,
    alias: Option<String>,
    glob: bool,
    line: usize,
}

fn tokenize(code: &str) -> Vec<Token> {
    let chars: Vec<char> = code.chars().collect();
    let mut out = Vec::new();
    let mut line = 1;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\n' {
            line += 1;
            i += 1;
        } else if c.is_whitespace() {
            i += 1;
        } else if c.is_alphanumeric() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            out.push(Token {
                tok: Tok::Ident(chars[start..i].iter().collect()),
                line,
            });
        } else if c == ':' && chars.get(i + 1) == Some(&':') {
            out.push(Token {
                tok: Tok::Colons,
                line,
            });
            i += 2;
        } else {
            out.push(Token {
                tok: Tok::Punct(c),
                line,
            });
            i += 1;
        }
    }
    out
}

struct UseParser<'a> {
    toks: &'a [Token],
    pos: usize,
    out: Vec<Import>,
}

impl UseParser<'_> {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos).map(|t| &t.tok)
    }

    fn line(&self) -> usize {
        self.toks.get(self.pos).map_or(0, |t| t.line)
    }

    fn eat_colons(&mut self) -> bool {
        let found = self.peek() == Some(&Tok::Colons);
        if found {
            self.pos += 1;
        }
        found
    }

    fn group(&mut self, path: &[String]) {
        self.pos += 1;
        loop {
            match self.peek() {
                Some(Tok::Punct('}')) => {
                    self.pos += 1;
                    return;
                }
                Some(Tok::Punct(',')) => self.pos += 1,
                Some(_) => {
                    let before = self.pos;
                    self.tree(path);
                    if self.pos == before {
                        return;
                    }
                }
                None => return,
            }
        }
    }

    fn tree(&mut self, prefix: &[String]) {
        let mut path = prefix.to_vec();
        let line = self.line();
        self.eat_colons();
        loop {
            match self.peek() {
                Some(Tok::Ident(name)) if name != "as" => {
                    path.push(name.clone());
                    self.pos += 1;
                    if !self.eat_colons() {
                        break;
                    }
                }
                Some(Tok::Punct('{')) => {
                    self.group(&path);
                    return;
                }
                Some(Tok::Punct('*')) => {
                    self.pos += 1;
                    self.out.push(Import {
                        path,
                        alias: None,
                        glob: true,
                        line,
                    });
                    return;
                }
                _ => return,
            }
        }
        let mut alias = None;
        if self.peek() == Some(&Tok::Ident("as".to_string())) {
            self.pos += 1;
            if let Some(Tok::Ident(name)) = self.peek() {
                alias = Some(name.clone());
                self.pos += 1;
            }
        }
        if path.last().is_some_and(|s| s == "self") {
            path.pop();
        }
        self.out.push(Import {
            path,
            alias,
            glob: false,
            line,
        });
    }
}

fn banned_path(path: &[String]) -> Option<String> {
    let n = path.len();
    if n >= 2 {
        let (module, item) = (path[n - 2].as_str(), path[n - 1].as_str());
        if module == "fs" && matches!(item, "write" | "rename" | "copy") {
            return Some(format!("fs::{item}"));
        }
        if module == "File" && item == "create" {
            return Some("File::create".to_string());
        }
    }
    if path.iter().any(|s| s == "OpenOptions") {
        return Some("OpenOptions".to_string());
    }
    None
}

fn expand(path: &[String], aliases: &HashMap<String, Vec<String>>) -> Vec<String> {
    let mut current = path.to_vec();
    for _ in 0..4 {
        let Some(first) = current.first() else { break };
        let Some(target) = aliases.get(first) else {
            break;
        };
        if target.first() == Some(first) && target.len() == 1 {
            break;
        }
        let mut next = target.clone();
        next.extend_from_slice(&current[1..]);
        if next == current {
            break;
        }
        current = next;
    }
    current
}

pub fn banned_paths(code: &str) -> Vec<(usize, String)> {
    let toks = tokenize(code);
    let mut imports = Vec::new();
    let mut use_ranges = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        let is_use = toks[i].tok == Tok::Ident("use".to_string())
            && !matches!(
                i.checked_sub(1).map(|p| &toks[p].tok),
                Some(Tok::Punct('.')) | Some(Tok::Colons)
            );
        if is_use {
            let mut parser = UseParser {
                toks: &toks,
                pos: i + 1,
                out: Vec::new(),
            };
            parser.tree(&[]);
            let mut end = parser.pos;
            while end < toks.len() && toks[end].tok != Tok::Punct(';') {
                end += 1;
            }
            use_ranges.push((i, end));
            imports.extend(parser.out);
            i = end + 1;
        } else {
            i += 1;
        }
    }

    let mut aliases: HashMap<String, Vec<String>> = HashMap::new();
    for import in imports.iter().filter(|im| !im.glob) {
        let name = import.alias.clone().or_else(|| import.path.last().cloned());
        if let Some(name) = name.filter(|n| n != "_") {
            aliases.insert(name, import.path.clone());
        }
    }

    let mut found = Vec::new();
    for import in &imports {
        let resolved = expand(&import.path, &aliases);
        if import.glob {
            if resolved.last().is_some_and(|s| s == "fs") {
                found.push((import.line, "fs::*".to_string()));
            }
        } else if let Some(what) = banned_path(&resolved) {
            found.push((import.line, what));
        }
    }

    let mut i = 0;
    while i < toks.len() {
        if let Some(&(_, end)) = use_ranges.iter().find(|(start, _)| *start == i) {
            i = end + 1;
            continue;
        }
        let Tok::Ident(first) = &toks[i].tok else {
            i += 1;
            continue;
        };
        let line = toks[i].line;
        let after_dot = i > 0 && toks[i - 1].tok == Tok::Punct('.');
        let mut path = vec![first.clone()];
        i += 1;
        while i + 1 < toks.len() && toks[i].tok == Tok::Colons {
            let Tok::Ident(next) = &toks[i + 1].tok else {
                break;
            };
            path.push(next.clone());
            i += 2;
        }
        if !after_dot {
            if let Some(what) = banned_path(&expand(&path, &aliases)) {
                found.push((line, what));
            }
        }
    }
    found
}

pub fn identifiers(code: &str) -> Vec<(usize, String)> {
    tokenize(code)
        .into_iter()
        .filter_map(|t| match t.tok {
            Tok::Ident(name) => Some((t.line, name)),
            _ => None,
        })
        .collect()
}
