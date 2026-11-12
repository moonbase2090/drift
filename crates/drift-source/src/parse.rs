use std::collections::BTreeMap;

use flate2::read::GzDecoder;
use std::io::Read;

use crate::error::Error;
use crate::installed::{Upgrade, PACKAGE_NAME};

pub fn field(text: &str, key: &str) -> Option<String> {
    section(text, key).and_then(|lines| lines.into_iter().next())
}

pub fn file_list(text: &str) -> Option<Vec<String>> {
    section(text, "FILES").map(|lines| lines.into_iter().filter(|line| !line.is_empty()).collect())
}

pub fn mtree_hashes(bytes: &[u8]) -> Result<BTreeMap<String, String>, Error> {
    let mut decoder = GzDecoder::new(bytes);
    let mut text = String::new();
    decoder
        .read_to_string(&mut text)
        .map_err(|_| Error::BadMtree)?;
    mtree_text(&text)
}

pub fn upgrades(text: &str) -> Vec<Upgrade> {
    let needle = format!("upgraded {PACKAGE_NAME} (");
    let mut found = Vec::new();
    for line in text.lines() {
        let Some(start) = line.find(&needle) else {
            continue;
        };
        let rest = &line[start + needle.len()..];
        let Some((from, after)) = rest.split_once(" -> ") else {
            continue;
        };
        let Some(to) = after.split(')').next() else {
            continue;
        };
        let from = from.trim();
        let to = to.trim();
        if from.is_empty() || to.is_empty() || from.contains(' ') || to.contains(' ') {
            continue;
        }
        found.push(Upgrade {
            from: from.to_string(),
            to: to.to_string(),
        });
    }
    found
}

fn section(text: &str, key: &str) -> Option<Vec<String>> {
    let header = format!("%{key}%");
    let lines: Vec<&str> = text.lines().collect();
    let mut index = 0;
    while index < lines.len() {
        if lines[index] == header {
            index += 1;
            let mut body = Vec::new();
            while index < lines.len() && !lines[index].is_empty() && !is_header(lines[index]) {
                body.push(lines[index].to_string());
                index += 1;
            }
            return Some(body);
        }
        index += 1;
    }
    None
}

fn is_header(line: &str) -> bool {
    let bytes = line.as_bytes();
    bytes.len() >= 3
        && bytes[0] == b'%'
        && bytes[bytes.len() - 1] == b'%'
        && !line[1..line.len() - 1].contains('%')
}

fn mtree_text(text: &str) -> Result<BTreeMap<String, String>, Error> {
    let mut defaults: BTreeMap<String, String> = BTreeMap::new();
    let mut files = BTreeMap::new();
    for line in text.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let Some(first) = parts.next() else {
            continue;
        };
        if first == "/set" {
            for (key, value) in pairs(parts) {
                defaults.insert(key, value);
            }
            continue;
        }
        let path = unescape(first).map_err(|_| Error::BadMtree)?;
        let path = path.strip_prefix("./").unwrap_or(&path).to_string();
        let mut fields = defaults.clone();
        for (key, value) in pairs(parts) {
            fields.insert(key, value);
        }
        if fields.get("type").map(String::as_str) != Some("file") {
            continue;
        }
        let Some(digest) = fields.get("sha256digest") else {
            continue;
        };
        files.insert(path, digest.to_ascii_lowercase());
    }
    Ok(files)
}

fn pairs<'a>(parts: impl Iterator<Item = &'a str>) -> Vec<(String, String)> {
    parts
        .filter_map(|part| {
            let (key, value) = part.split_once('=')?;
            Some((key.to_string(), value.to_string()))
        })
        .collect()
}

fn unescape(input: &str) -> Result<String, ()> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'\\' {
            out.push(bytes[index]);
            index += 1;
            continue;
        }
        index += 1;
        if index >= bytes.len() {
            return Err(());
        }
        match bytes[index] {
            b'\\' => out.push(b'\\'),
            b's' => out.push(b' '),
            b'n' => out.push(b'\n'),
            b't' => out.push(b'\t'),
            b'r' => out.push(b'\r'),
            b'0'..=b'7' => {
                let mut value = u32::from(bytes[index] - b'0');
                let mut count = 1;
                while count < 3
                    && index + 1 < bytes.len()
                    && (b'0'..=b'7').contains(&bytes[index + 1])
                {
                    index += 1;
                    value = value * 8 + u32::from(bytes[index] - b'0');
                    count += 1;
                }
                if value > 255 {
                    return Err(());
                }
                out.push(value as u8);
            }
            _ => return Err(()),
        }
        index += 1;
    }
    String::from_utf8(out).map_err(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    const DESC: &str = "\
%NAME%
omarchy-settings

%VERSION%
9.9.9-1

%DESC%
templates
";

    const FILES: &str = "\
%FILES%
usr/share/omarchy/config/hypr/a.lua
etc/

%BACKUP%
etc/foo
";

    #[test]
    fn desc_fields_ignore_other_sections() {
        assert_eq!(field(DESC, "NAME").as_deref(), Some("omarchy-settings"));
        assert_eq!(field(DESC, "VERSION").as_deref(), Some("9.9.9-1"));
        assert_eq!(field(DESC, "ARCH"), None);
    }

    #[test]
    fn file_list_stops_before_the_backup_section() {
        assert_eq!(
            file_list(FILES).unwrap(),
            vec![
                "usr/share/omarchy/config/hypr/a.lua".to_string(),
                "etc/".to_string(),
            ]
        );
    }

    #[test]
    fn mtree_keeps_file_digests_and_unescapes_names() {
        let text = "\
#mtree
/set type=file uid=0 gid=0 mode=644
./usr/share/omarchy/config/my\\sfile.txt time=1.0 size=1 sha256digest=AbC
./usr/share/omarchy/config/dir type=dir
./usr/share/omarchy/config/c.lua type=file sha256digest=def
";
        let map = mtree_text(text).unwrap();
        assert_eq!(
            map.get("usr/share/omarchy/config/my file.txt")
                .map(String::as_str),
            Some("abc")
        );
        assert_eq!(
            map.get("usr/share/omarchy/config/c.lua")
                .map(String::as_str),
            Some("def")
        );
        assert!(!map.keys().any(|path| path.ends_with("/dir")));
    }

    #[test]
    fn upgrades_keep_order_and_skip_other_lines() {
        let log = "\
[2026-01-01T00:00:00+0000] [ALPM] installed omarchy-settings (1.0.0-1)
[2026-01-02T00:00:00+0000] [ALPM] upgraded omarchy (1 -> 2)
[2026-01-03T00:00:00+0000] [ALPM] upgraded omarchy-settings (1.0.0-1 -> 1.1.0-1)
[2026-02-01T00:00:00+0000] [ALPM] upgraded omarchy-settings (1.1.0-1 -> 1.2.0-1)
[2026-03-01T00:00:00+0000] [ALPM] upgraded something (1.2.0-1 -> 2.0.0-1)
[2026-04-01T00:00:00+0000] [ALPM] upgraded omarchy-settings (not an upgrade)
[2026-05-01T00:00:00+0000] [ALPM] upgraded omarchy-settings (1.2.0-1 -> 2.0.0-1)
";
        assert_eq!(
            upgrades(log),
            vec![
                Upgrade {
                    from: "1.0.0-1".to_string(),
                    to: "1.1.0-1".to_string(),
                },
                Upgrade {
                    from: "1.1.0-1".to_string(),
                    to: "1.2.0-1".to_string(),
                },
                Upgrade {
                    from: "1.2.0-1".to_string(),
                    to: "2.0.0-1".to_string(),
                },
            ]
        );
    }

    #[test]
    fn a_corrupt_mtree_is_rejected() {
        assert!(matches!(mtree_hashes(b"not gzip"), Err(Error::BadMtree)));
    }
}
