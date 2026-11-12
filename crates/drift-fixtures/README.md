# drift-fixtures

Test-only helper for Drift. It builds temporary home and package trees, and it hashes a directory by content, mode, and mtime. Production crates do not depend on it. A test crate may:

```toml
[dev-dependencies]
drift-fixtures.workspace = true
```

`ours/` is the live config and is written to `$HOME/.config/`. `base/` is the reviewed base and is written to `$HOME/.local/share/drift/bases/config/`. `theirs/` is the installed package templates and is written to `$OMARCHY_PATH/config/`.

Materialize copies bytes, permission bits, and symlink targets. It does not copy mtimes. It creates missing parents one directory at a time and refuses to follow a symlink, so a link cannot redirect a write outside the temporary tree. `hash_tree` notices a one-byte edit, a mode change, an mtime change, or a rename, including a name that is not UTF-8.

## Case directory

```
fixtures/<group>/<case>/
  expected.json
  layout.json      optional
  base/...
  ours/...
  theirs/...
```

`expected.json` lists each relative path once:

```json
{
  "files": [
    {
      "path": "hypr/looknfeel.lua",
      "status": "upstream changed",
      "badge": "merges cleanly",
      "base": "same",
      "ours": "same",
      "theirs": "changed"
    }
  ]
}
```

`status` is one of `unchanged`, `upstream changed`, `local override`, `conflict`, `user-only`, or `missing`. `badge` is `merges cleanly`, `base unknown`, `deleted by you`, or omitted. `base`, `ours`, and `theirs` are `absent`, `same`, or `changed`.

`layout.json` names anything git cannot store:

```json
{
  "modes": { "ours/hypr/secret.lua": "0600" },
  "symlinks": { "ours/hypr/alias.lua": "secret.lua" },
  "absent": ["ours/hypr/removed.lua"]
}
```

Modes are octal permission bits. A symlink may instead be a real symlink in the case tree. An absent path is not created. Paths start with `base/`, `ours/`, or `theirs/`.

## Builder

```rust
let trees = drift_fixtures::FixtureBuilder::new()
    .put(
        drift_fixtures::Root::Config,
        "hypr/looknfeel.lua",
        drift_fixtures::Node::file(b"local\n"),
    )?
    .put(
        drift_fixtures::Root::Templates,
        "hypr/looknfeel.lua",
        drift_fixtures::Node::file(b"upstream\n"),
    )?
    .put(
        drift_fixtures::Root::Bases,
        "hypr/looknfeel.lua",
        drift_fixtures::Node::absent(),
    )?
    .materialize()?;

let home = trees.home();
let omarchy_path = trees.omarchy_path();
```

`materialize_case` does the same from a case directory. `case_dirs` lists every case under `fixtures/`. The trees are removed when the value is dropped.

## Tree hash

`hash_tree` walks a directory without following symlinks. Each path contributes its full relative path as raw filename bytes, its kind, permission bits, mtime (seconds and nanoseconds), and payload. The payload is the file bytes, the raw symlink target, or empty for a directory. The digest is SHA-256 over those records in path order.

## Golden cases

`fixtures/presence/` is the full matrix of base, ours, and theirs each being absent, the same bytes, or distinct bytes. The file is `hypr/note.txt`. Same bytes are `same\n`. Changed bytes are `base\n`, `ours\n`, or `theirs\n`.

`fixtures/status/` has one tree for each classification: unchanged, upstream changed, a clean merge, a local override, a local override with no base, an overlapping conflict, a user-only file beside a template, a template the live tree deleted, and a missing live file with no base.

`fixtures/entries/modes-and-links/` covers a `0600` file, a symlink, and an absent path.
