use crate::path::RelPath;
use crate::registry::{Mapping, Registry};
use crate::status::{content_treatment, Treatment};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verification {
    Verified,
    Mismatch,
    Unchecked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Template {
    mapping: Mapping,
    verification: Verification,
}

impl Template {
    pub fn mapping(&self) -> &Mapping {
        &self.mapping
    }

    pub fn path(&self) -> &RelPath {
        &self.mapping.template
    }

    pub fn live_path(&self) -> &RelPath {
        &self.mapping.live
    }

    pub fn verification(&self) -> Verification {
        self.verification
    }

    pub fn has_warning(&self) -> bool {
        self.verification == Verification::Mismatch
    }

    pub fn is_volatile(&self) -> bool {
        self.mapping.volatile
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveState {
    Regular { len: u64, utf8: bool },
    Symlink { target: String },
    Unreadable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveTreatment {
    Content(Treatment),
    ReadOnlySymlink,
    Unreadable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveFile {
    mapping: Mapping,
    state: LiveState,
}

impl LiveFile {
    pub fn mapping(&self) -> &Mapping {
        &self.mapping
    }

    pub fn path(&self) -> &RelPath {
        &self.mapping.live
    }

    pub fn state(&self) -> &LiveState {
        &self.state
    }

    pub fn is_volatile(&self) -> bool {
        self.mapping.volatile
    }

    pub fn link_target(&self) -> Option<&str> {
        match &self.state {
            LiveState::Symlink { target } => Some(target),
            _ => None,
        }
    }

    pub fn treatment(&self) -> LiveTreatment {
        match &self.state {
            LiveState::Regular { len, utf8 } => {
                LiveTreatment::Content(content_treatment(*len, *utf8))
            }
            LiveState::Symlink { .. } => LiveTreatment::ReadOnlySymlink,
            LiveState::Unreadable => LiveTreatment::Unreadable,
        }
    }

    pub fn is_writable(&self) -> bool {
        self.treatment() != LiveTreatment::ReadOnlySymlink
    }
}

impl Registry {
    pub fn template(&self, path: &RelPath, verification: Verification) -> Option<Template> {
        self.map_template(path).map(|mapping| Template {
            mapping,
            verification,
        })
    }

    pub fn live_file(&self, path: &RelPath, state: LiveState) -> Option<LiveFile> {
        self.locate_live(path)
            .map(|mapping| LiveFile { mapping, state })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::status::{HashOnlyReason, MAX_DIFF_BYTES};

    fn rel(s: &str) -> RelPath {
        RelPath::new(s).unwrap()
    }

    fn regular(len: u64, utf8: bool) -> LiveState {
        LiveState::Regular { len, utf8 }
    }

    #[test]
    fn a_template_carries_its_mapping_and_verification() {
        let template = Registry::v1()
            .template(&rel("config/hypr/hyprland.lua"), Verification::Verified)
            .unwrap();
        assert_eq!(template.path(), &rel("config/hypr/hyprland.lua"));
        assert_eq!(template.live_path(), &rel(".config/hypr/hyprland.lua"));
        assert_eq!(template.mapping().rel, rel("hypr/hyprland.lua"));
        assert_eq!(template.verification(), Verification::Verified);
        assert!(!template.has_warning());
        assert!(!template.is_volatile());
    }

    #[test]
    fn only_a_verification_mismatch_is_a_warning() {
        let registry = Registry::v1();
        let path = rel("config/a");
        let warn = |v| registry.template(&path, v).unwrap().has_warning();
        assert!(warn(Verification::Mismatch));
        assert!(!warn(Verification::Verified));
        assert!(!warn(Verification::Unchecked));
    }

    #[test]
    fn templates_outside_every_root_are_not_templates() {
        let registry = Registry::v1();
        assert_eq!(
            registry.template(&rel("bin/omarchy-update"), Verification::Verified),
            None
        );
        assert_eq!(
            registry.template(&rel("configx/a"), Verification::Verified),
            None
        );
    }

    #[test]
    fn volatile_applies_to_templates_and_live_files() {
        let registry = Registry::v1();
        let template = registry
            .template(
                &rel("config/chromium/Default/Preferences"),
                Verification::Verified,
            )
            .unwrap();
        assert!(template.is_volatile());
        let live = registry
            .live_file(
                &rel(".config/chromium/Default/Preferences"),
                regular(10, true),
            )
            .unwrap();
        assert!(live.is_volatile());
        let other = registry
            .live_file(
                &rel(".config/chromium/Default/Bookmarks"),
                regular(10, true),
            )
            .unwrap();
        assert!(!other.is_volatile());
    }

    #[test]
    fn a_live_file_pairs_with_its_template() {
        let registry = Registry::v1();
        let template = registry
            .template(&rel("config/waybar/config"), Verification::Verified)
            .unwrap();
        let live = registry
            .live_file(template.live_path(), regular(1, true))
            .unwrap();
        assert_eq!(live.mapping(), template.mapping());
        assert_eq!(live.path(), &rel(".config/waybar/config"));
    }

    #[test]
    fn paths_outside_the_live_roots_are_not_live_files() {
        let registry = Registry::v1();
        assert_eq!(
            registry.live_file(&rel(".local/share/x"), regular(1, true)),
            None
        );
        assert_eq!(registry.live_file(&rel(".config"), regular(1, true)), None);
    }

    #[test]
    fn regular_files_follow_the_content_limits() {
        let registry = Registry::v1();
        let treat = |state| {
            registry
                .live_file(&rel(".config/a"), state)
                .unwrap()
                .treatment()
        };
        assert_eq!(
            treat(regular(5, true)),
            LiveTreatment::Content(Treatment::Diffable)
        );
        assert_eq!(
            treat(regular(MAX_DIFF_BYTES + 1, true)),
            LiveTreatment::Content(Treatment::HashOnly(HashOnlyReason::Oversize))
        );
        assert_eq!(
            treat(regular(5, false)),
            LiveTreatment::Content(Treatment::HashOnly(HashOnlyReason::NonUtf8))
        );
    }

    #[test]
    fn symlinks_are_read_only_and_expose_their_destination() {
        let registry = Registry::v1();
        let link = registry
            .live_file(
                &rel(".config/hypr/a.lua"),
                LiveState::Symlink {
                    target: "../elsewhere/a.lua".to_string(),
                },
            )
            .unwrap();
        assert_eq!(link.treatment(), LiveTreatment::ReadOnlySymlink);
        assert_eq!(link.link_target(), Some("../elsewhere/a.lua"));
        assert!(!link.is_writable());
        let file = registry
            .live_file(&rel(".config/hypr/a.lua"), regular(1, true))
            .unwrap();
        assert_eq!(file.link_target(), None);
        assert!(file.is_writable());
    }

    #[test]
    fn unreadable_files_have_their_own_treatment() {
        let live = Registry::v1()
            .live_file(&rel(".config/a"), LiveState::Unreadable)
            .unwrap();
        assert_eq!(live.treatment(), LiveTreatment::Unreadable);
        assert!(live.is_writable());
    }

    #[test]
    fn user_roots_produce_templates_and_live_files_too() {
        let registry = crate::registry_from_user_config(
            "[[root]]\nid = \"apps\"\ntemplate = \"applications\"\nlive = \".local/share/applications\"\n",
        )
        .unwrap();
        let template = registry
            .template(&rel("applications/x.desktop"), Verification::Unchecked)
            .unwrap();
        assert_eq!(
            template.live_path(),
            &rel(".local/share/applications/x.desktop")
        );
        let live = registry
            .live_file(template.live_path(), regular(1, true))
            .unwrap();
        assert_eq!(live.mapping().root, "apps");
    }
}
