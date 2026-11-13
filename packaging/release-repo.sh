#!/bin/bash
set -euo pipefail

repo_name=omarchy-drift
secret_name=DRIFT_PACMAN_SIGNING_KEY
key_id_name=DRIFT_PACMAN_SIGNING_KEY_ID
published_fpr=F6365E856F3A896BABDAA4BD378705A414BF786C

script_dir=$(CDPATH= cd -- "$(dirname "$0")" && pwd)
root=$(CDPATH= cd -- "$script_dir/.." && pwd)

die() {
  printf '%s\n' "error: $*" >&2
  exit 1
}

usage() {
  die "usage: release-repo.sh check-key | unsigned DEST | stage DEST"
}

load_key() {
  if [ -n "${DRIFT_KEY_FILE:-}" ]; then
    if [ ! -f "$DRIFT_KEY_FILE" ]; then
      die "package signing key file is missing."
    fi
    DRIFT_PACMAN_SIGNING_KEY=$(cat "$DRIFT_KEY_FILE")
  fi
  if [ -z "${DRIFT_PACMAN_SIGNING_KEY:-}" ]; then
    die "Actions secret ${secret_name} is not set. Refusing to publish an unsigned repository."
  fi
  if [ -z "${DRIFT_PACMAN_SIGNING_KEY_ID:-}" ]; then
    die "Actions secret ${key_id_name} is not set. Refusing to publish an unsigned repository."
  fi
  if [ "$DRIFT_PACMAN_SIGNING_KEY_ID" != "$published_fpr" ]; then
    die "Actions secret ${key_id_name} does not match the published signing key."
  fi
}

verify_published_key() {
  local pub gnupg fpr
  pub=$root/keys/drift-pacman-signing.pub.asc
  if [ ! -f "$pub" ]; then
    die "published signing key is missing."
  fi
  gnupg=$(mktemp -d)
  chmod 700 "$gnupg"
  if ! GNUPGHOME=$gnupg gpg --batch --import "$pub" >/dev/null 2>&1; then
    rm -rf "$gnupg"
    die "published signing key could not be imported."
  fi
  fpr=$(GNUPGHOME=$gnupg gpg --batch --with-colons --list-keys | awk -F: '$1 == "fpr" { print $10; exit }') || {
    rm -rf "$gnupg"
    exit 1
  }
  rm -rf "$gnupg"
  if [ "$fpr" != "$published_fpr" ]; then
    die "published signing key does not match the expected fingerprint."
  fi
}

pkgbuild_path() {
  if [ -f "$script_dir/PKGBUILD" ]; then
    printf '%s\n' "$script_dir/PKGBUILD"
    return
  fi
  if [ -f "$root/packaging/PKGBUILD" ]; then
    printf '%s\n' "$root/packaging/PKGBUILD"
    return
  fi
  die "packaging/PKGBUILD is not in this tree, so there is nothing to build."
}

ensure_builder() {
  if [ "$(id -u)" -ne 0 ]; then
    return
  fi
  if ! id builder >/dev/null 2>&1; then
    useradd --create-home --shell /bin/bash builder
  fi
  if [ ! -d /home/builder ]; then
    mkdir -p /home/builder
  fi
  chown builder:builder /home/builder
}

run_makepkg() {
  local build=$1/packaging
  ensure_builder
  if [ "$(id -u)" -eq 0 ]; then
    chown -R builder:builder "$1"
    runuser -u builder -- bash -c 'cd "$1" && makepkg --noconfirm --force --nosign' bash "$build"
  else
    (cd "$build" && makepkg --noconfirm --force --nosign)
  fi
}

built_package() {
  local work=$1/packaging
  local packages=()
  local pkg
  shopt -s nullglob
  packages=("$work"/${repo_name}-*.pkg.tar.zst)
  shopt -u nullglob
  if [ "${#packages[@]}" -ne 1 ]; then
    die "expected one built package, found ${#packages[@]}."
  fi
  pkg=${packages[0]}
  printf '%s\n' "$pkg"
}

materialize_db() {
  local dest=$1
  local base link target src
  for base in db files; do
    link=$dest/${repo_name}.${base}
    if [ -L "$link" ]; then
      target=$(readlink "$link")
      case $target in
        /*) src=$target ;;
        *) src=$dest/$target ;;
      esac
      rm -f "$link"
      cp -a "$src" "$link"
    fi
    if [ -f "$dest/${repo_name}.${base}.tar.gz.sig" ]; then
      cp -a "$dest/${repo_name}.${base}.tar.gz.sig" "$dest/${repo_name}.${base}.sig"
    fi
  done
}

add_repo() {
  local dest=$1
  local pkg=$2
  local sign_key=${3:-}
  local base db
  mkdir -p "$dest"
  cp -a "$pkg" "$dest/"
  if [ -n "${pkg}.sig" ] && [ -f "${pkg}.sig" ]; then
    cp -a "${pkg}.sig" "$dest/"
  fi
  base=$(basename "$pkg")
  db=$dest/${repo_name}.db.tar.gz
  rm -f \
    "$dest"/${repo_name}.db \
    "$dest"/${repo_name}.db.sig \
    "$dest"/${repo_name}.db.tar.gz \
    "$dest"/${repo_name}.db.tar.gz.sig \
    "$dest"/${repo_name}.files \
    "$dest"/${repo_name}.files.sig \
    "$dest"/${repo_name}.files.tar.gz \
    "$dest"/${repo_name}.files.tar.gz.sig
  if [ -n "$sign_key" ]; then
    repo-add --sign --key "$sign_key" --include-sigs "$db" "$dest/$base"
  else
    repo-add "$db" "$dest/$base"
  fi
  materialize_db "$dest"
  find "$dest" -mindepth 1 -exec chmod a+rX {} +
  if [ "$(id -u)" -eq 0 ]; then
    chmod a+rX "$dest"
  fi
}

prepare_work() {
  local mode=$1
  local src work install_script version hash
  src=$(pkgbuild_path) || exit 1
  install_script=$(dirname "$src")/omarchy-drift.install
  if [ ! -f "$install_script" ]; then
    die "omarchy-drift.install is missing beside the PKGBUILD."
  fi
  if [ "$mode" = tag ]; then
    version=${DRIFT_VERSION:-}
    version=${version#v}
    if [ -z "$version" ]; then
      version=$(git -C "$root" -c safe.directory="$root" describe --tags --exact-match) || exit 1
      version=${version#v}
    fi
    if ! [[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
      die "refusing to publish version '${version}'."
    fi
  fi
  work=$(mktemp -d)
  mkdir -p "$work/packaging"
  if [ -d "$root/plugin" ]; then
    mkdir -p "$work/plugin"
    cp -a "$root/plugin/." "$work/plugin/"
  fi
  if [ "$mode" = tag ]; then
    git -C "$root" -c safe.directory="$root" archive \
      --format=tar.gz \
      --prefix="drift-${version}/" \
      -o "$work/packaging/${repo_name}-${version}.tar.gz" \
      HEAD
    hash=$(sha256sum "$work/packaging/${repo_name}-${version}.tar.gz" | awk '{ print $1 }') || exit 1
    sed \
      -e "s/^pkgver=.*/pkgver=${version}/" \
      -e "s/^sha256sums=.*/sha256sums=('${hash}')/" \
      "$src" > "$work/packaging/PKGBUILD"
  else
    cp "$src" "$work/packaging/PKGBUILD"
  fi
  cp "$install_script" "$work/packaging/omarchy-drift.install"
  chmod -R a+rX "$work"
  printf '%s\n' "$work"
}

import_key() {
  local gnupg fpr
  load_key
  gnupg=$(mktemp -d)
  chmod 700 "$gnupg"
  printf '%s\n' 'allow-loopback-pinentry' > "$gnupg/gpg-agent.conf"
  if ! printf '%s\n' "$DRIFT_PACMAN_SIGNING_KEY" | GNUPGHOME=$gnupg gpg --batch --pinentry-mode loopback --import >/dev/null 2>&1; then
    rm -rf "$gnupg"
    die "the package signing key could not be imported."
  fi
  fpr=$(GNUPGHOME=$gnupg gpg --batch --with-colons --list-secret-keys | awk -F: '$1 == "fpr" { print $10; exit }') || {
    rm -rf "$gnupg"
    exit 1
  }
  if [ -z "$fpr" ] || [ "$fpr" != "$published_fpr" ]; then
    rm -rf "$gnupg"
    die "imported signing key does not match the published fingerprint."
  fi
  if ! printf '%s:6:\n' "$fpr" | GNUPGHOME=$gnupg gpg --batch --import-ownertrust >/dev/null 2>&1; then
    rm -rf "$gnupg"
    die "the package signing key could not be trusted for signing."
  fi
  printf '%s\n' "$fpr" > "$gnupg/fpr"
  printf '%s\n' "$gnupg"
}

cleanup_gpg() {
  if [ -n "${GNUPGHOME:-}" ] && [ -d "$GNUPGHOME" ]; then
    rm -rf "$GNUPGHOME"
    unset GNUPGHOME
  fi
}

unsigned() {
  local dest=$1
  local work pkg
  work=$(prepare_work as-written) || exit 1
  run_makepkg "$work"
  pkg=$(built_package "$work") || exit 1
  add_repo "$dest" "$pkg"
  rm -rf "$work"
  printf '%s\n' "unsigned repository written to ${dest}"
}

stage() {
  local dest=$1
  local work pkg gnupg fpr
  verify_published_key
  load_key
  work=$(prepare_work tag) || exit 1
  run_makepkg "$work"
  pkg=$(built_package "$work") || exit 1
  gnupg=$(import_key) || exit 1
  fpr=$(cat "$gnupg/fpr")
  if [ "$fpr" != "$published_fpr" ] || [ "$fpr" != "$DRIFT_PACMAN_SIGNING_KEY_ID" ]; then
    rm -rf "$gnupg"
    die "imported signing key does not match the published fingerprint."
  fi
  export GNUPGHOME=$gnupg
  gpg --batch --yes --pinentry-mode loopback --local-user "$DRIFT_PACMAN_SIGNING_KEY_ID" --detach-sign --output "${pkg}.sig" "$pkg"
  add_repo "$dest" "$pkg" "$DRIFT_PACMAN_SIGNING_KEY_ID"
  printf '%s\n' "$published_fpr" > "$dest/FINGERPRINT"
  chmod a+rX "$dest/FINGERPRINT"
  rm -rf "$work"
  cleanup_gpg
  printf '%s\n' "signed repository written to ${dest}"
}

trap cleanup_gpg EXIT

cmd=${1:-}
case $cmd in
  check-key)
    verify_published_key
    load_key
    gnupg=$(import_key) || exit 1
    fpr=$(cat "$gnupg/fpr")
    export GNUPGHOME=$gnupg
    if [ "$fpr" != "$published_fpr" ] || [ "$fpr" != "$DRIFT_PACMAN_SIGNING_KEY_ID" ]; then
      die "imported signing key does not match the published fingerprint."
    fi
    cleanup_gpg
    printf '%s\n' "package signing key is present"
    ;;
  unsigned)
    [ $# -eq 2 ] || usage
    unsigned "$2"
    ;;
  stage)
    [ $# -eq 2 ] || usage
    stage "$2"
    ;;
  *)
    usage
    ;;
esac
