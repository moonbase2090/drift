#!/bin/bash
set -euo pipefail

root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
[[ $# == 1 ]] || { echo 'usage: test-release.sh OUTPUT' >&2; exit 2; }
mkdir -p "$1"
dest=$(cd "$1" && pwd)
[[ -z $(find "$dest" -mindepth 1 -maxdepth 1 -print -quit) ]] || { echo 'Output must be empty.' >&2; exit 2; }
work=$(mktemp -d)
key_home=$work/key
mkdir -m 700 "$key_home"
cleanup() {
  gpgconf --homedir "$key_home" --kill all || true
  rm -rf -- "$work"
}
trap cleanup EXIT
export GNUPGHOME=$key_home
gpg --batch --passphrase '' --quick-generate-key 'Release proof <release@example.com>' ed25519 sign 1d >/dev/null 2>&1
fingerprint=$(gpg --batch --with-colons --list-secret-keys | awk -F: '$1 == "fpr" {print $10; exit}')
gpg --batch --armor --export-secret-keys "$fingerprint" > "$work/key.asc"
git clone --quiet --shared "$root" "$work/source"
gpg --batch --armor --export "$fingerprint" > "$work/source/keys/drift-pacman-signing.pub.asc"
sed -i "s/F6365E856F3A896BABDAA4BD378705A414BF786C/$fingerprint/g" "$work/source/packaging/release-repo.sh"
git -C "$work/source" -c user.name='Release proof' -c user.email=release@example.com add keys packaging/release-repo.sh
git -C "$work/source" -c user.name='Release proof' -c user.email=release@example.com commit --quiet -m 'Use a throwaway key in the private release proof.'
if env -u DRIFT_PACMAN_SIGNING_KEY -u DRIFT_KEY_FILE DRIFT_PACMAN_SIGNING_KEY_ID="$fingerprint" "$root/packaging/release-repo.sh" check-key > "$work/rejected.log" 2>&1; then
  echo 'Production validation accepted a throwaway key ID.' >&2
  exit 1
fi
if env -u DRIFT_PACMAN_SIGNING_KEY -u DRIFT_KEY_FILE DRIFT_PACMAN_SIGNING_KEY_ID=F6365E856F3A896BABDAA4BD378705A414BF786C "$root/packaging/release-repo.sh" check-key > "$work/missing.log" 2>&1; then
  echo 'Production validation accepted a missing secret.' >&2
  exit 1
fi
grep -q 'DRIFT_PACMAN_SIGNING_KEY is not set' "$work/missing.log"
if env -u DRIFT_PACMAN_SIGNING_KEY DRIFT_KEY_FILE="$work/key.asc" DRIFT_PACMAN_SIGNING_KEY_ID=F6365E856F3A896BABDAA4BD378705A414BF786C "$root/packaging/release-repo.sh" check-key > "$work/wrong.log" 2>&1; then
  echo 'Production validation accepted an unrelated secret key.' >&2
  exit 1
fi
grep -q 'Secret key does not match' "$work/wrong.log"
DRIFT_VERSION=$(awk '/^\[workspace.package\]/{inside=1;next} inside && /^version *=/{gsub(/[" ]/, "", $0); sub(/^version=/, ""); print; exit}' "$root/Cargo.toml")
export DRIFT_VERSION DRIFT_KEY_FILE=$work/key.asc DRIFT_PACMAN_SIGNING_KEY_ID=$fingerprint
if DRIFT_VERSION=99.99.99 "$work/source/packaging/release-repo.sh" stage "$work/mismatch" > "$work/version.log" 2>&1; then
  echo 'A version tag that disagrees with Cargo.toml was accepted.' >&2
  exit 1
fi
grep -q 'Version tag must match' "$work/version.log"
"$work/source/packaging/release-repo.sh" stage "$dest"
[[ -f $dest/SRCINFO ]]
[[ -z $(find "$dest" -type l -print -quit) ]]
for artifact in "$dest/"*.pkg.tar.zst "$dest/omarchy-drift.db" "$dest/omarchy-drift.files" "$dest/SHA256SUMS"; do
  gpg --batch --verify "$artifact.sig" "$artifact"
done
(cd "$dest"; sha256sum --check SHA256SUMS)
pkg=("$dest/"*.pkg.tar.zst)
mkdir "$work/extracted"
bsdtar -xf "${pkg[0]}" -C "$work/extracted"
"$work/extracted/usr/bin/drift" --version | grep -Fx "drift $DRIFT_VERSION"
for file in BarWidget.qml DriftStatus.js manifest.json; do
  cmp "$work/source/plugin/$file" "$work/extracted/usr/share/omarchy-drift/plugin/$file"
done
cp "${pkg[0]}" "$work/corrupt.pkg"
printf '\0' >> "$work/corrupt.pkg"
if gpg --batch --verify "${pkg[0]}.sig" "$work/corrupt.pkg" >/dev/null 2>&1; then
  echo 'A corrupted package passed signature verification.' >&2
  exit 1
fi
printf 'Signed release proof passed. Public artifacts: %s\n' "$dest"
