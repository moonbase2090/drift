#!/bin/bash
set -euo pipefail

root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
repo_name=omarchy-drift
published_fpr=F6365E856F3A896BABDAA4BD378705A414BF786C
work=
key_home=

cleanup() {
  if [[ -n $key_home ]]; then
    gpgconf --homedir "$key_home" --kill all || true
    rm -rf -- "$key_home"
  fi
  if [[ -n $work ]]; then rm -rf -- "$work"; fi
}
trap cleanup EXIT

die() { printf 'error: %s\n' "$*" >&2; exit 1; }

import_key() {
  [[ -n ${DRIFT_PACMAN_SIGNING_KEY_ID:-} ]] || die 'Actions secret DRIFT_PACMAN_SIGNING_KEY_ID is not set.'
  [[ $DRIFT_PACMAN_SIGNING_KEY_ID == "$published_fpr" ]] || die 'Signing key ID does not match the published fingerprint.'
  [[ -n ${DRIFT_PACMAN_SIGNING_KEY:-}${DRIFT_KEY_FILE:-} ]] || die 'Actions secret DRIFT_PACMAN_SIGNING_KEY is not set. Refusing to publish an unsigned repository.'
  key_home=$(mktemp -d)
  export GNUPGHOME=$key_home
  local fingerprint
  gpg --batch --import "$root/keys/drift-pacman-signing.pub.asc" >/dev/null 2>&1 || die 'Cannot import the published public key.'
  fingerprint=$(gpg --batch --with-colons --list-keys | awk -F: '$1 == "fpr" {print $10; exit}')
  [[ $fingerprint == "$published_fpr" ]] || die 'Public key does not match the published fingerprint.'
  if [[ -n ${DRIFT_KEY_FILE:-} ]]; then
    gpg --batch --import "$DRIFT_KEY_FILE" >/dev/null 2>&1 || die 'Cannot import the signing key.'
  else
    printf '%s\n' "$DRIFT_PACMAN_SIGNING_KEY" | gpg --batch --import >/dev/null 2>&1 || die 'Cannot import the signing key.'
  fi
  fingerprint=$(gpg --batch --with-colons --list-secret-keys | awk -F: '$1 == "fpr" {print $10; exit}')
  [[ $fingerprint == "$published_fpr" ]] || die 'Secret key does not match the published fingerprint.'
  printf '%s:6:\n' "$published_fpr" | gpg --batch --import-ownertrust >/dev/null 2>&1
}

prepare() {
  local version=${DRIFT_VERSION:-} cargo_version archive hash
  version=${version#v}
  [[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die 'Set DRIFT_VERSION to the version tag, vX.Y.Z.'
  cargo_version=$(awk '/^\[workspace.package\]/{inside=1;next} inside && /^\[/{exit} inside && /^version *=/{gsub(/[" ]/, "", $0); sub(/^version=/, ""); print; exit}' "$root/Cargo.toml")
  [[ $version == "$cargo_version" ]] || die 'Version tag must match workspace.package.version in Cargo.toml.'
  work=$(mktemp -d)
  archive=$repo_name-$version.tar.gz
  git -C "$root" -c safe.directory="$root" archive --format=tar.gz --prefix="drift-$version/" -o "$work/$archive" HEAD
  hash=$(sha256sum "$work/$archive" | cut -d' ' -f1)
  sed -e "s/^pkgver=.*/pkgver=$version/" \
      -e 's/^pkgrel=.*/pkgrel=1/' \
      -e "s/^sha256sums=.*/sha256sums=('$hash')/" \
      -e 's|^source=.*|source=("$pkgname-$pkgver.tar.gz::$url/releases/download/v$pkgver/$pkgname-$pkgver.tar.gz")|' \
      "$root/packaging/PKGBUILD" > "$work/PKGBUILD"
  cp "$root/packaging/omarchy-drift.install" "$work/"
}

build() {
  if [[ $(id -u) == 0 ]]; then
    if ! id builder >/dev/null 2>&1; then useradd --create-home builder; fi
    chown -R builder:builder "$work"
    runuser -u builder -- bash -c 'cd "$1"; makepkg --noconfirm --nosign; makepkg --printsrcinfo > .SRCINFO' bash "$work"
  else
    (cd "$work"; makepkg --noconfirm --nosign; makepkg --printsrcinfo > .SRCINFO)
  fi
}

stage() {
  local mode=$1 dest=$2 pkg base
  mkdir -p "$dest"
  dest=$(cd "$dest" && pwd)
  [[ -z $(find "$dest" -mindepth 1 -maxdepth 1 -print -quit) ]] || die 'Output directory must be empty.'
  if [[ $mode == signed ]]; then import_key; fi
  prepare
  build
  local packages=("$work"/"$repo_name"-*.pkg.tar.zst)
  [[ ${#packages[@]} == 1 && -f ${packages[0]} ]] || die 'Expected exactly one package archive.'
  pkg=${packages[0]}
  cp "$pkg" "$work/PKGBUILD" "$work/omarchy-drift.install" "$work/$repo_name-"*.tar.gz "$dest/"
  cp "$work/.SRCINFO" "$dest/SRCINFO"
  cp "$root/keys/drift-pacman-signing.pub.asc" "$dest/"
  if [[ $mode == signed ]]; then
    gpg --batch --yes --pinentry-mode loopback --local-user "$published_fpr" --detach-sign "$dest/$(basename "$pkg")"
    repo-add --sign --key "$published_fpr" --include-sigs "$dest/$repo_name.db.tar.gz" "$dest/$(basename "$pkg")"
  else
    repo-add "$dest/$repo_name.db.tar.gz" "$dest/$(basename "$pkg")"
  fi
  for base in db files; do
    rm "$dest/$repo_name.$base"
    cp "$dest/$repo_name.$base.tar.gz" "$dest/$repo_name.$base"
    if [[ $mode == signed ]]; then
      rm -f "$dest/$repo_name.$base.sig"
      cp "$dest/$repo_name.$base.tar.gz.sig" "$dest/$repo_name.$base.sig"
    fi
  done
  printf '%s\n' "$published_fpr" > "$dest/FINGERPRINT"
  (cd "$dest"; sha256sum -- * > SHA256SUMS)
  if [[ $mode == signed ]]; then
    gpg --batch --yes --pinentry-mode loopback --local-user "$published_fpr" --detach-sign "$dest/SHA256SUMS"
  fi
  chmod -R a+rX "$dest"
  printf '%s repository written to %s\n' "$mode" "$dest"
}

case ${1:-} in
  check-key) [[ $# == 1 ]] || die 'check-key takes no arguments.'; import_key; printf 'Package signing key verified.\n' ;;
  stage) [[ $# == 2 ]] || die 'stage requires an output directory.'; stage signed "$2" ;;
  unsigned) [[ $# == 2 ]] || die 'unsigned requires an output directory.'; stage unsigned "$2" ;;
  *) die 'usage: release-repo.sh check-key | stage DEST | unsigned DEST' ;;
esac
