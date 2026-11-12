use std::path::{Path, PathBuf};

use crate::error::Error;
use crate::path::RelPath;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Builtin,
    User,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Root {
    id: String,
    template: RelPath,
    live: RelPath,
    volatile: Vec<RelPath>,
    origin: Origin,
}

impl Root {
    pub fn new(
        id: &str,
        template: RelPath,
        live: RelPath,
        volatile: Vec<RelPath>,
        origin: Origin,
    ) -> Result<Root, Error> {
        let valid = !id.is_empty()
            && id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
        if !valid {
            return Err(Error::InvalidRootId(id.to_string()));
        }
        Ok(Root {
            id: id.to_string(),
            template,
            live,
            volatile,
            origin,
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn template(&self) -> &RelPath {
        &self.template
    }

    pub fn live(&self) -> &RelPath {
        &self.live
    }

    pub fn volatile(&self) -> &[RelPath] {
        &self.volatile
    }

    pub fn origin(&self) -> Origin {
        self.origin
    }

    fn mapping(&self, rel: RelPath) -> Mapping {
        Mapping {
            root: self.id.clone(),
            template: self.template.join(&rel),
            live: self.live.join(&rel),
            volatile: self.volatile.contains(&rel),
            rel,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mapping {
    pub root: String,
    pub rel: RelPath,
    pub template: RelPath,
    pub live: RelPath,
    pub volatile: bool,
}

impl Mapping {
    pub fn live_path(&self, home: &Path) -> PathBuf {
        self.live.under(home)
    }

    pub fn template_path(&self, omarchy_path: &Path) -> PathBuf {
        self.template.under(omarchy_path)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registry {
    roots: Vec<Root>,
}

pub const BUILTIN_CONFIG_ROOT: &str = "config";
pub const VOLATILE_CHROMIUM_PREFERENCES: &str = "chromium/Default/Preferences";

impl Registry {
    pub fn v1() -> Registry {
        let root = Root::new(
            BUILTIN_CONFIG_ROOT,
            RelPath::new("config").expect("constant path is valid"),
            RelPath::new(".config").expect("constant path is valid"),
            vec![RelPath::new(VOLATILE_CHROMIUM_PREFERENCES).expect("constant path is valid")],
            Origin::Builtin,
        )
        .expect("constant root is valid");
        Registry { roots: vec![root] }
    }

    pub fn with_roots(self, extra: Vec<Root>) -> Result<Registry, Error> {
        let mut roots = self.roots;
        for root in extra {
            for existing in &roots {
                if existing.id == root.id {
                    return Err(Error::DuplicateRoot(root.id));
                }
                if existing.template.overlaps(&root.template) || existing.live.overlaps(&root.live)
                {
                    return Err(Error::OverlappingRoots {
                        first: existing.id.clone(),
                        second: root.id,
                    });
                }
            }
            roots.push(root);
        }
        Ok(Registry { roots })
    }

    pub fn roots(&self) -> &[Root] {
        &self.roots
    }

    pub fn map_template(&self, template: &RelPath) -> Option<Mapping> {
        self.roots.iter().find_map(|root| {
            template
                .strip_prefix(&root.template)
                .map(|rel| root.mapping(rel))
        })
    }

    pub fn locate_live(&self, live: &RelPath) -> Option<Mapping> {
        self.roots
            .iter()
            .find_map(|root| live.strip_prefix(&root.live).map(|rel| root.mapping(rel)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rel(s: &str) -> RelPath {
        RelPath::new(s).unwrap()
    }

    fn user_root(id: &str, template: &str, live: &str) -> Root {
        Root::new(id, rel(template), rel(live), Vec::new(), Origin::User).unwrap()
    }

    #[test]
    fn v1_maps_config_templates_to_dot_config() {
        let mapping = Registry::v1()
            .map_template(&rel("config/hypr/hyprland.lua"))
            .unwrap();
        assert_eq!(mapping.root, "config");
        assert_eq!(mapping.rel, rel("hypr/hyprland.lua"));
        assert_eq!(mapping.live, rel(".config/hypr/hyprland.lua"));
        assert_eq!(mapping.template, rel("config/hypr/hyprland.lua"));
        assert!(!mapping.volatile);
    }

    #[test]
    fn v1_has_exactly_one_builtin_root_and_no_secondary_roots() {
        let registry = Registry::v1();
        assert_eq!(registry.roots().len(), 1);
        assert_eq!(registry.roots()[0].origin(), Origin::Builtin);
        for deferred in [
            "applications/foo.desktop",
            "default/alacritty/Alacritty.desktop",
            "default/hypr/toggles/flags.lua",
        ] {
            assert_eq!(registry.map_template(&rel(deferred)), None, "{deferred}");
        }
    }

    #[test]
    fn templates_outside_the_config_prefix_do_not_map() {
        let registry = Registry::v1();
        assert_eq!(registry.map_template(&rel("configx/a.lua")), None);
        assert_eq!(registry.map_template(&rel("config")), None);
        assert_eq!(registry.map_template(&rel("bin/omarchy-update")), None);
    }

    #[test]
    fn chromium_preferences_is_volatile_and_siblings_are_not() {
        let registry = Registry::v1();
        let prefs = registry
            .map_template(&rel("config/chromium/Default/Preferences"))
            .unwrap();
        assert!(prefs.volatile);
        for other in [
            "config/chromium/Default/Preferences.bak",
            "config/chromium/Default/Bookmarks",
            "config/chromium/Preferences",
            "config/other/chromium/Default/Preferences",
            "config/chromium/Default/Preferences/child",
        ] {
            assert!(
                !registry.map_template(&rel(other)).unwrap().volatile,
                "{other}"
            );
        }
    }

    #[test]
    fn live_paths_map_back_to_the_same_mapping() {
        let registry = Registry::v1();
        let forward = registry
            .map_template(&rel("config/waybar/config.jsonc"))
            .unwrap();
        let back = registry.locate_live(&forward.live).unwrap();
        assert_eq!(forward, back);
        assert_eq!(registry.locate_live(&rel(".local/share/x")), None);
        assert_eq!(registry.locate_live(&rel(".configuration/x")), None);
    }

    #[test]
    fn mapping_resolves_concrete_paths_without_touching_the_filesystem() {
        let mapping = Registry::v1()
            .map_template(&rel("config/hypr/a.lua"))
            .unwrap();
        assert_eq!(
            mapping.live_path(Path::new("/home/user")),
            PathBuf::from("/home/user/.config/hypr/a.lua")
        );
        assert_eq!(
            mapping.template_path(Path::new("/usr/share/omarchy")),
            PathBuf::from("/usr/share/omarchy/config/hypr/a.lua")
        );
    }

    #[test]
    fn user_roots_are_added_and_map_independently() {
        let registry = Registry::v1()
            .with_roots(vec![user_root(
                "apps",
                "applications",
                ".local/share/applications",
            )])
            .unwrap();
        assert_eq!(registry.roots().len(), 2);
        let mapping = registry
            .map_template(&rel("applications/x.desktop"))
            .unwrap();
        assert_eq!(mapping.root, "apps");
        assert_eq!(mapping.live, rel(".local/share/applications/x.desktop"));
        assert_eq!(registry.roots()[1].origin(), Origin::User);
        assert!(registry.map_template(&rel("config/a")).is_some());
    }

    #[test]
    fn duplicate_ids_are_rejected() {
        let result = Registry::v1().with_roots(vec![user_root("config", "other", ".other")]);
        assert_eq!(result, Err(Error::DuplicateRoot("config".to_string())));
    }

    #[test]
    fn overlapping_template_or_live_prefixes_are_rejected() {
        let nested_template = Registry::v1().with_roots(vec![user_root("a", "config/sub", ".x")]);
        assert!(matches!(
            nested_template,
            Err(Error::OverlappingRoots { .. })
        ));
        let nested_live = Registry::v1().with_roots(vec![user_root("a", "tpl", ".config/sub")]);
        assert!(matches!(nested_live, Err(Error::OverlappingRoots { .. })));
    }

    #[test]
    fn two_user_roots_may_not_overlap_each_other() {
        let result = Registry::v1().with_roots(vec![
            user_root("a", "tpl", ".local/a"),
            user_root("b", "tpl/inner", ".local/b"),
        ]);
        assert!(matches!(result, Err(Error::OverlappingRoots { .. })));
    }

    #[test]
    fn root_ids_are_restricted_to_lowercase_slugs() {
        for bad in ["", "Config", "a b", "a/b", "é"] {
            let result = Root::new(bad, rel("t"), rel("l"), Vec::new(), Origin::User);
            assert_eq!(
                result,
                Err(Error::InvalidRootId(bad.to_string())),
                "{bad:?}"
            );
        }
        assert!(Root::new("my-root_2", rel("t"), rel("l"), Vec::new(), Origin::User).is_ok());
    }
}
