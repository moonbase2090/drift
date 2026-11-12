use std::ops::Range;

use toml::de::{DeTable, DeValue};
use toml::Spanned;

use crate::error::{Error, Location};
use crate::path::RelPath;
use crate::registry::{Origin, Registry, Root};

pub const USER_CONFIG_PATH: &str = ".config/drift/config.toml";

type Value<'i> = Spanned<DeValue<'i>>;

fn fail(text: &str, span: Range<usize>, message: impl Into<String>) -> Error {
    Error::Config {
        message: message.into(),
        location: Some(Location::at(text, span.start)),
    }
}

fn string_field(
    text: &str,
    table: &DeTable<'_>,
    entry_span: &Range<usize>,
    key: &str,
) -> Result<(String, Range<usize>), Error> {
    match table.iter().find(|(k, _)| k.get_ref().as_ref() == key) {
        Some((_, value)) => match value.get_ref() {
            DeValue::String(s) => Ok((s.to_string(), value.span())),
            _ => Err(fail(
                text,
                value.span(),
                format!("`{key}` must be a string"),
            )),
        },
        None => Err(fail(text, entry_span.clone(), format!("missing `{key}`"))),
    }
}

fn path_value(text: &str, value: String, span: Range<usize>) -> Result<RelPath, Error> {
    RelPath::new(&value).map_err(|e| fail(text, span, e.to_string()))
}

fn volatile_field(text: &str, table: &DeTable<'_>) -> Result<Vec<RelPath>, Error> {
    let Some((_, value)) = table
        .iter()
        .find(|(k, _)| k.get_ref().as_ref() == "volatile")
    else {
        return Ok(Vec::new());
    };
    let message = "`volatile` must be an array of strings";
    let DeValue::Array(items) = value.get_ref() else {
        return Err(fail(text, value.span(), message));
    };
    items
        .iter()
        .map(|item: &Value<'_>| match item.get_ref() {
            DeValue::String(s) => path_value(text, s.to_string(), item.span()),
            _ => Err(fail(text, item.span(), message)),
        })
        .collect()
}

fn root_from_entry(text: &str, entry: &Value<'_>) -> Result<Root, Error> {
    let DeValue::Table(table) = entry.get_ref() else {
        return Err(fail(text, entry.span(), "each `root` must be a table"));
    };
    for (key, _) in table.iter() {
        if !matches!(
            key.get_ref().as_ref(),
            "id" | "template" | "live" | "volatile"
        ) {
            return Err(fail(
                text,
                key.span(),
                format!("unknown key `{}`", key.get_ref()),
            ));
        }
    }
    let span = entry.span();
    let (id, id_span) = string_field(text, table, &span, "id")?;
    let (template, template_span) = string_field(text, table, &span, "template")?;
    let (live, live_span) = string_field(text, table, &span, "live")?;
    let template = path_value(text, template, template_span)?;
    let live = path_value(text, live, live_span)?;
    let volatile = volatile_field(text, table)?;
    Root::new(&id, template, live, volatile, Origin::User)
        .map_err(|e| fail(text, id_span, e.to_string()))
}

pub fn parse_user_roots(text: &str) -> Result<Vec<Root>, Error> {
    let document = DeTable::parse(text).map_err(|e| Error::Config {
        message: e.message().to_string(),
        location: e.span().map(|span| Location::at(text, span.start)),
    })?;
    let table = document.get_ref();
    let mut roots = Vec::new();
    for (key, value) in table.iter() {
        if key.get_ref().as_ref() != "root" {
            return Err(fail(
                text,
                key.span(),
                format!("unknown key `{}`", key.get_ref()),
            ));
        }
        let DeValue::Array(entries) = value.get_ref() else {
            return Err(fail(
                text,
                value.span(),
                "`root` must be an array of tables ([[root]])",
            ));
        };
        for entry in entries.iter() {
            roots.push(root_from_entry(text, entry)?);
        }
    }
    Ok(roots)
}

pub fn registry_from_user_config(text: &str) -> Result<Registry, Error> {
    Registry::v1().with_roots(parse_user_roots(text)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rel(s: &str) -> RelPath {
        RelPath::new(s).unwrap()
    }

    const VALID: &str = r#"
[[root]]
id = "apps"
template = "applications"
live = ".local/share/applications"
volatile = ["cache.db"]

[[root]]
id = "flags"
template = "default/hypr/toggles"
live = ".local/state/omarchy/toggles/hypr"
"#;

    #[test]
    fn empty_config_adds_no_roots() {
        assert_eq!(parse_user_roots("").unwrap(), Vec::new());
        assert_eq!(registry_from_user_config("").unwrap(), Registry::v1());
    }

    #[test]
    fn valid_roots_are_parsed_in_order_with_user_origin() {
        let roots = parse_user_roots(VALID).unwrap();
        assert_eq!(roots.len(), 2);
        assert_eq!(roots[0].id(), "apps");
        assert_eq!(roots[0].template(), &rel("applications"));
        assert_eq!(roots[0].live(), &rel(".local/share/applications"));
        assert_eq!(roots[0].volatile(), &[rel("cache.db")]);
        assert_eq!(roots[1].id(), "flags");
        assert!(roots[1].volatile().is_empty());
        assert!(roots.iter().all(|r| r.origin() == Origin::User));
    }

    #[test]
    fn registry_from_config_maps_user_roots_and_keeps_the_builtin_one() {
        let registry = registry_from_user_config(VALID).unwrap();
        assert_eq!(registry.roots().len(), 3);
        let mapping = registry
            .map_template(&rel("applications/cache.db"))
            .unwrap();
        assert!(mapping.volatile);
        assert_eq!(mapping.live, rel(".local/share/applications/cache.db"));
        assert!(registry.map_template(&rel("config/hypr/a.lua")).is_some());
    }

    fn located(result: Result<Vec<Root>, Error>) -> (usize, usize, String) {
        match result {
            Err(Error::Config {
                message,
                location: Some(at),
            }) => (at.line, at.column, message),
            other => panic!("expected a located config error, got {other:?}"),
        }
    }

    #[test]
    fn malformed_toml_names_the_line_and_column() {
        let text = "[[root]]\nid = \"a\"\ntemplate = \"t\"\nlive = \"l\"\nvolatile = [\"x\"\n";
        let (line, _, message) = located(parse_user_roots(text));
        assert_eq!(line, 5, "{message}");
        let (line, column, _) = located(parse_user_roots("id = \n"));
        assert_eq!((line, column), (1, 6));
    }

    #[test]
    fn syntax_error_in_a_later_root_points_at_that_root() {
        let text = format!("{VALID}\n[[root]]\nid = \"c\"\ntemplate = \n");
        let (line, _, _) = located(parse_user_roots(&text));
        assert_eq!(line, VALID.lines().count() + 4);
    }

    #[test]
    fn mistyped_field_points_at_the_value() {
        let text = "[[root]]\nid = 3\ntemplate = \"a\"\nlive = \"b\"\n";
        let (line, column, message) = located(parse_user_roots(text));
        assert_eq!((line, column), (2, 6));
        assert!(message.contains("`id` must be a string"), "{message}");
        let text = "[[root]]\nid = \"a\"\ntemplate = \"a\"\nlive = \"b\"\nvolatile = [\"ok\", 1]\n";
        let (line, column, _) = located(parse_user_roots(text));
        assert_eq!((line, column), (5, 19));
    }

    #[test]
    fn missing_field_points_at_the_root_that_lacks_it() {
        let text = "[[root]]\nid = \"a\"\ntemplate = \"t\"\nlive = \"l\"\n\n[[root]]\nid = \"b\"\nlive = \"l2\"\n";
        let (line, _, message) = located(parse_user_roots(text));
        assert!((6..=7).contains(&line), "line {line}");
        assert!(message.contains("missing `template`"), "{message}");
    }

    #[test]
    fn unknown_keys_point_at_the_key() {
        let (line, column, message) = located(parse_user_roots("\n\ntheme = \"x\"\n"));
        assert_eq!((line, column), (3, 1));
        assert!(message.contains("unknown key `theme`"), "{message}");
        let text = "[[root]]\nid = \"a\"\ntemplate = \"a\"\nlive = \"b\"\ncolor = 1\n";
        let (line, column, _) = located(parse_user_roots(text));
        assert_eq!((line, column), (5, 1));
        let (line, _, _) = located(parse_user_roots("\nroot = 3\n"));
        assert_eq!(line, 2);
    }

    #[test]
    fn unsafe_paths_and_bad_ids_point_at_their_values() {
        let text = "[[root]]\nid = \"a\"\ntemplate = \"../etc\"\nlive = \"b\"\n";
        let (line, column, message) = located(parse_user_roots(text));
        assert_eq!((line, column), (3, 12));
        assert!(message.contains("'..'"), "{message}");
        let text = "[[root]]\nid = \"a\"\ntemplate = \"a\"\nlive = \"/abs\"\n";
        let (line, _, _) = located(parse_user_roots(text));
        assert_eq!(line, 4);
        let text = "[[root]]\nid = \"Bad Id\"\ntemplate = \"a\"\nlive = \"b\"\n";
        let (line, _, _) = located(parse_user_roots(text));
        assert_eq!(line, 2);
    }

    #[test]
    fn error_text_includes_the_position() {
        let error = parse_user_roots("\n\ntheme = 1\n").unwrap_err();
        assert!(error
            .to_string()
            .starts_with("invalid configuration at line 3, column 1:"));
    }

    #[test]
    fn a_user_root_that_collides_with_the_builtin_root_is_rejected() {
        let overlap = "[[root]]\nid = \"mine\"\ntemplate = \"config/extra\"\nlive = \".extra\"\n";
        assert!(matches!(
            registry_from_user_config(overlap),
            Err(Error::OverlappingRoots { .. })
        ));
        let duplicate = "[[root]]\nid = \"config\"\ntemplate = \"x\"\nlive = \".x\"\n";
        assert_eq!(
            registry_from_user_config(duplicate),
            Err(Error::DuplicateRoot("config".to_string()))
        );
    }

    #[test]
    fn the_config_location_is_under_dot_config() {
        assert_eq!(USER_CONFIG_PATH, ".config/drift/config.toml");
    }
}
