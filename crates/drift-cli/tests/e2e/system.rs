use std::io::Write;
use std::path::{Path, PathBuf};

use drift_fixtures::{MaterializedRoot, RootBuilder};
use flate2::write::GzEncoder;
use flate2::Compression;
use sha2::{Digest, Sha256};

use crate::harness::{Launch, Session};

pub const VERSION: &str = "4.5.0-3";

pub struct Template {
    pub rel: &'static str,
    pub theirs: &'static str,
    pub ours: Option<&'static str>,
    pub base: Option<&'static str>,
}

pub const UNCHANGED: Template = Template {
    rel: "hypr/input.lua",
    theirs: "kb_layout = us\n",
    ours: Some("kb_layout = us\n"),
    base: Some("kb_layout = us\n"),
};

pub const UPSTREAM: Template = Template {
    rel: "hypr/looknfeel.lua",
    theirs: "gaps = 8\nrounding = 2\n",
    ours: Some("gaps = 5\nrounding = 2\n"),
    base: Some("gaps = 5\nrounding = 2\n"),
};

pub const OVERRIDE: Template = Template {
    rel: "hypr/monitors.lua",
    theirs: "scale = 1\n",
    ours: Some("scale = 2\n"),
    base: Some("scale = 1\n"),
};

pub const CONFLICT: Template = Template {
    rel: "hypr/bindings.lua",
    theirs: "bind = SUPER, A, nine\nbind = SUPER, B, two\nbind = SUPER, C, three\n",
    ours: Some("bind = SUPER, A, seven\nbind = SUPER, B, two\nbind = SUPER, C, three\n"),
    base: Some("bind = SUPER, A, one\nbind = SUPER, B, two\nbind = SUPER, C, three\n"),
};

pub const CLEAN_MERGE: Template = Template {
    rel: "hypr/autostart.lua",
    theirs: "exec a\nexec b\nexec c\nexec d\nexec e\nexec f\nexec g\nexec H\n",
    ours: Some("exec A\nexec b\nexec c\nexec d\nexec e\nexec f\nexec g\nexec h\n"),
    base: Some("exec a\nexec b\nexec c\nexec d\nexec e\nexec f\nexec g\nexec h\n"),
};

pub const MISSING: Template = Template {
    rel: "waybar/config.jsonc",
    theirs: "{ \"height\": 30 }\n",
    ours: None,
    base: Some("{ \"height\": 30 }\n"),
};

pub const NO_BASE: Template = Template {
    rel: "hypr/windows.lua",
    theirs: "rule = float\nrule = tile\n",
    ours: Some("rule = float\nrule = stack\n"),
    base: None,
};

pub const USER_ONLY: (&str, &str) = ("hypr/mine.lua", "-- only mine\n");

pub const WORLD: [&Template; 7] = [
    &UNCHANGED,
    &UPSTREAM,
    &OVERRIDE,
    &CONFLICT,
    &CLEAN_MERGE,
    &MISSING,
    &NO_BASE,
];

fn sha(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub struct SystemBuilder {
    templates: Vec<&'static Template>,
    user_only: Vec<(&'static str, &'static str)>,
}

impl SystemBuilder {
    pub fn template(mut self, template: &'static Template) -> Self {
        self.templates.push(template);
        self
    }

    pub fn user_only(mut self, rel: &'static str, text: &'static str) -> Self {
        self.user_only.push((rel, text));
        self
    }

    pub fn build(self) -> System {
        let mut builder = RootBuilder::new();
        for keep in [
            "home/.local/share/.keep",
            "home/.local/state/.keep",
            "runtime/.keep",
            "bin/.keep",
            "run/.keep",
            "var/lib/drift/.keep",
        ] {
            builder.file(keep, b"").unwrap();
        }
        builder
            .file_with_mode(
                "bin/omarchy-notification-send",
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$DRIFT_ROOT/notifications.log\"\n",
                0o755,
            )
            .unwrap();
        for name in ["bin/omarchy", "bin/omarchy-shell", "bin/hyprctl"] {
            builder
                .file_with_mode(name, b"#!/bin/sh\nexit 0\n", 0o755)
                .unwrap();
        }
        let mut listed = String::from("%FILES%\nusr/share/omarchy/config/\n");
        let mut mtree = String::from("#mtree\n/set type=file uid=0 gid=0 mode=644\n");
        for template in &self.templates {
            let rel = template.rel;
            listed.push_str(&format!("usr/share/omarchy/config/{rel}\n"));
            mtree.push_str(&format!(
                "./usr/share/omarchy/config/{rel} time=1.0 size=1 sha256digest={}\n",
                sha(template.theirs)
            ));
            builder
                .file(format!("usr/share/omarchy/config/{rel}"), template.theirs)
                .unwrap();
            if let Some(ours) = template.ours {
                builder.file(format!("home/.config/{rel}"), ours).unwrap();
            }
            if let Some(base) = template.base {
                builder
                    .file(format!("home/.local/share/drift/bases/config/{rel}"), base)
                    .unwrap();
            }
        }
        for (rel, text) in &self.user_only {
            builder.file(format!("home/.config/{rel}"), text).unwrap();
        }
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(mtree.as_bytes()).unwrap();
        let dir = format!("var/lib/pacman/local/omarchy-settings-{VERSION}");
        builder
            .file(
                format!("{dir}/desc"),
                format!("%NAME%\nomarchy-settings\n\n%VERSION%\n{VERSION}\n"),
            )
            .unwrap()
            .file(format!("{dir}/files"), listed)
            .unwrap()
            .file(format!("{dir}/mtree"), encoder.finish().unwrap())
            .unwrap();
        System {
            root: builder.materialize().unwrap(),
        }
    }
}

pub struct System {
    root: MaterializedRoot,
}

impl System {
    pub fn builder() -> SystemBuilder {
        SystemBuilder {
            templates: Vec::new(),
            user_only: Vec::new(),
        }
    }

    pub fn world() -> System {
        let mut builder = System::builder();
        for template in WORLD {
            builder = builder.template(template);
        }
        builder.user_only(USER_ONLY.0, USER_ONLY.1).build()
    }

    pub fn root(&self) -> &Path {
        self.root.path()
    }

    pub fn home(&self) -> PathBuf {
        self.root().join("home")
    }

    pub fn env(&self) -> Vec<(String, String)> {
        let home = self.home();
        let text = |path: PathBuf| path.display().to_string();
        vec![
            ("PATH".into(), text(self.root().join("bin"))),
            ("HOME".into(), text(home.clone())),
            ("XDG_CONFIG_HOME".into(), text(home.join(".config"))),
            ("XDG_DATA_HOME".into(), text(home.join(".local/share"))),
            ("XDG_STATE_HOME".into(), text(home.join(".local/state"))),
            ("XDG_RUNTIME_DIR".into(), text(self.root().join("runtime"))),
            (
                "OMARCHY_PATH".into(),
                text(self.root().join("usr/share/omarchy")),
            ),
            ("DRIFT_ROOT".into(), text(self.root().to_path_buf())),
            ("TERM".into(), "xterm-256color".into()),
            ("EDITOR".into(), text(self.root().join("bin/editor"))),
        ]
    }

    pub fn launch(&self, args: &[&str], size: (u16, u16)) -> Session {
        Session::start(Launch {
            program: env!("CARGO_BIN_EXE_drift"),
            args: args.iter().map(|arg| arg.to_string()).collect(),
            env: self.env(),
            size,
        })
    }

    pub fn open(&self) -> Session {
        self.launch(&[], (120, 36))
    }
}
