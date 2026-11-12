use crate::error::Error;
use crate::path::RelPath;
use crate::registry::{Origin, Registry, Root};

pub const USER_CONFIG_PATH: &str = ".config/drift/config.toml";

fn config_error(message: impl Into<String>) -> Error {
    Error::Config(message.into())
}

fn string_field(table: &toml::Table, key: &str, index: usize) -> Result<String, Error> {
    match table.get(key) {
        Some(toml::Value::String(value)) => Ok(value.clone()),
        Some(_) => Err(config_error(format!(
            "root #{index}: `{key}` must be a string"
        ))),
        None => Err(config_error(format!("root #{index}: missing `{key}`"))),
    }
}

fn volatile_field(table: &toml::Table, index: usize) -> Result<Vec<RelPath>, Error> {
    let Some(value) = table.get("volatile") else {
        return Ok(Vec::new());
    };
    let Some(list) = value.as_array() else {
        return Err(config_error(format!(
            "root #{index}: `volatile` must be an array of strings"
        )));
    };
    list.iter()
        .map(|item| match item.as_str() {
            Some(path) => RelPath::new(path),
            None => Err(config_error(format!(
                "root #{index}: `volatile` must be an array of strings"
            ))),
        })
        .collect()
}

pub fn parse_user_roots(text: &str) -> Result<Vec<Root>, Error> {
    let table: toml::Table = text
        .parse()
        .map_err(|e: toml::de::Error| config_error(e.message().to_string()))?;
    if let Some(key) = table.keys().find(|k| *k != "root") {
        return Err(config_error(format!("unknown key `{key}`")));
    }
    let Some(entries) = table.get("root") else {
        return Ok(Vec::new());
    };
    let Some(entries) = entries.as_array() else {
        return Err(config_error("`root` must be an array of tables ([[root]])"));
    };
    let mut roots = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        let index = index + 1;
        let Some(entry) = entry.as_table() else {
            return Err(config_error(format!("root #{index} must be a table")));
        };
        if let Some(key) = entry
            .keys()
            .find(|k| !matches!(k.as_str(), "id" | "template" | "live" | "volatile"))
        {
            return Err(config_error(format!("root #{index}: unknown key `{key}`")));
        }
        let id = string_field(entry, "id", index)?;
        let template = RelPath::new(&string_field(entry, "template", index)?)?;
        let live = RelPath::new(&string_field(entry, "live", index)?)?;
        let volatile = volatile_field(entry, index)?;
        roots.push(Root::new(&id, template, live, volatile, Origin::User)?);
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

    #[test]
    fn malformed_toml_is_a_config_error() {
        assert!(matches!(parse_user_roots("[[root"), Err(Error::Config(_))));
    }

    #[test]
    fn missing_and_mistyped_fields_are_rejected() {
        for bad in [
            "[[root]]\ntemplate = \"a\"\nlive = \"b\"\n",
            "[[root]]\nid = \"a\"\nlive = \"b\"\n",
            "[[root]]\nid = \"a\"\ntemplate = \"a\"\n",
            "[[root]]\nid = 3\ntemplate = \"a\"\nlive = \"b\"\n",
            "[[root]]\nid = \"a\"\ntemplate = \"a\"\nlive = \"b\"\nvolatile = \"x\"\n",
            "[[root]]\nid = \"a\"\ntemplate = \"a\"\nlive = \"b\"\nvolatile = [1]\n",
        ] {
            assert!(
                matches!(parse_user_roots(bad), Err(Error::Config(_))),
                "{bad}"
            );
        }
    }

    #[test]
    fn unknown_keys_are_rejected_at_both_levels() {
        assert!(matches!(
            parse_user_roots("theme = \"x\"\n"),
            Err(Error::Config(_))
        ));
        let bad = "[[root]]\nid = \"a\"\ntemplate = \"a\"\nlive = \"b\"\ncolor = 1\n";
        assert!(matches!(parse_user_roots(bad), Err(Error::Config(_))));
        assert!(matches!(
            parse_user_roots("root = 3\n"),
            Err(Error::Config(_))
        ));
    }

    #[test]
    fn traversal_in_a_configured_path_is_rejected() {
        let bad = "[[root]]\nid = \"a\"\ntemplate = \"../etc\"\nlive = \"b\"\n";
        assert!(matches!(
            parse_user_roots(bad),
            Err(Error::InvalidPath { .. })
        ));
        let bad = "[[root]]\nid = \"a\"\ntemplate = \"a\"\nlive = \"/abs\"\n";
        assert!(matches!(
            parse_user_roots(bad),
            Err(Error::InvalidPath { .. })
        ));
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
