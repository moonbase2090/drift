# Release the pacman package

Only the owner publishes version tags. Preparing or testing this workflow does not publish a release.

## Prepare a version

1. Set `[workspace.package].version` in `Cargo.toml` to the intended `X.Y.Z`, update `Cargo.lock`, and run the normal checks. Review the packaged plugin version when its compatibility changes.
2. Merge the release changes, then create and push the owner-approved tag `vX.Y.Z` at that commit. The workflow checks that the tag's version matches the workspace version.
3. The Release workflow builds in an Arch container using `packaging/PKGBUILD`. It archives the tagged source with `git archive`, sets `pkgver`, resets `pkgrel` to `1`, and pins that archive's SHA-256 in a generated PKGBUILD. It never combines plugin files from outside the archive with the tagged binary.
4. It signs the package, repository database and file database, and the checksums. It publishes the source archive, generated `PKGBUILD`, `.SRCINFO`, install script, public key, package, databases, signatures and `SHA256SUMS` on the version release. The same assets update the fixed `pacman-repo` release used by pacman.
5. Check the workflow result, download the assets, and run `sha256sum --check SHA256SUMS` and `gpg --verify SHA256SUMS.sig SHA256SUMS` after importing the published public key. Test installation using [the install instructions](install.md).

The generated PKGBUILD downloads its exact source archive from the version release. To update the recipe kept in the repository, copy the generated `PKGBUILD` and `.SRCINFO` from that release into `packaging/`, review the diff, and commit them. This avoids using the checksum of GitHub's independently generated tag archive. Keep `omarchy-drift.install` beside the recipe when building with `makepkg`.

Publication is serialized across tags. Retrying a workflow replaces assets with the same names; a later package version becomes the one advertised by the fixed repository database. Old package assets may remain available but are no longer indexed. Do not rerun an older tag after a newer release unless intentionally rolling the repository back.

## Signing configuration

The repository owner supplies Actions secrets `DRIFT_PACMAN_SIGNING_KEY` (an armored signing key with no passphrase) and `DRIFT_PACMAN_SIGNING_KEY_ID` (the full fingerprint). The script requires the fingerprint in `keys/drift-pacman-signing.pub.asc` to match its pinned value and requires the imported secret key to match it too. Missing or mismatched keys stop publication. The workflow removes its temporary key file on success or failure; the signing script uses a temporary GnuPG home and removes it on exit.

No signing material is committed except the public key. Update the pinned fingerprint, public key and installation instructions together when rotating the signing key.

## Local proof without publication

On Arch with the PKGBUILD's dependencies installed:

```bash
packaging/test-release.sh /tmp/drift-release-proof
```

This makes a private copy of the source, creates a one-day throwaway signing key, substitutes that key only in the private copy, builds with `makepkg`, signs with `repo-add`, verifies signatures and checksums, and checks the package's binary and plugin files. It leaves public artifacts in the output directory and deletes its temporary private key and build tree. Production key validation stays unchanged. It never calls GitHub or creates a tag.

For an end-to-end install, mount the output directory read-only as `/repo` in a disposable, clean Arch container. In that container:

```bash
pacman-key --init
pacman-key --populate archlinux
pacman-key --add /repo/drift-pacman-signing.pub.asc
pacman-key --lsign-key "$(cat /repo/FINGERPRINT)"
printf '\n[omarchy-drift]\nSigLevel = Required DatabaseRequired\nServer = file:///repo\n' >> /etc/pacman.conf
pacman -Syu --noconfirm omarchy-drift
drift --version
ls /usr/share/omarchy-drift/plugin/
```

Use the real public key and HTTPS server from the install guide when testing a published release. A local throwaway-key test does not prove that the production Actions secrets or GitHub uploads work.

## AUR later

AUR publication is outside this workflow. When an owner has a maintainer account and approves publication, the generated recipe and `.SRCINFO` can seed an `omarchy-drift` AUR repository. Do not push to AUR as part of a pacman release.
