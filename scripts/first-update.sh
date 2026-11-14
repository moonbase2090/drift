#!/bin/bash
set -euo pipefail

BIN=${DRIFT_BIN:?set DRIFT_BIN to the drift binary}
[ -f "$BIN" ] && [ -x "$BIN" ] || { echo "FAIL: DRIFT_BIN is not an executable file: $BIN" >&2; exit 1; }
BIN=$(cd "$(dirname "$BIN")" && pwd -P)/$(basename "$BIN")
HERE=$(cd "$(dirname "$0")/.." && pwd)
PKG=$HERE/packaging
WORK=$(mktemp -d "${TMPDIR:-/tmp}/drift-first-update.XXXXXX")
trap 'rm -rf "$WORK"' EXIT

step() { printf '\n== %s\n' "$*"; }
fail() { printf 'FAIL: %s\n' "$*" >&2; exit 1; }
expect() { grep -Fq -- "$1" <<<"$2" || fail "expected \"$1\" in:"$'\n'"$2"; }
refute() { ! grep -Fq -- "$1" <<<"$2" || fail "did not expect \"$1\" in:"$'\n'"$2"; }

write_package() {
  local root=$1 version=$2 variant=$3 files="" mtree=""
  rm -rf "$root/var/lib/pacman/local" "$root/usr/share/omarchy/config"
  mkdir -p "$root/usr/share/omarchy/config/hypr" "$root/usr/share/omarchy/config/waybar"
  tpl() { printf '%b' "$2" > "$root/usr/share/omarchy/config/$1"; }
  if [ "$variant" = v1 ]; then
    tpl hypr/looknfeel.lua 'gaps = 5\nrounding = 2\n'
    tpl hypr/monitors.lua 'scale = 1\n'
    tpl hypr/bindings.lua 'a = 1\nb = 2\nc = 3\n'
    tpl hypr/input.lua 'x = 1\n'
    tpl waybar/config.jsonc '{ "height": 30 }\n'
  else
    tpl hypr/looknfeel.lua 'gaps = 5\nrounding = 4\n'
    tpl hypr/monitors.lua 'scale = 1\n'
    tpl hypr/bindings.lua 'a = 1\nb = 2\nc = 9\n'
    tpl hypr/input.lua 'x = 3\n'
    tpl waybar/config.jsonc '{ "height": 30 }\n'
  fi
  files=$'%FILES%\nusr/share/omarchy/config/\n'
  mtree=$'#mtree\n/set type=file uid=0 gid=0 mode=644\n'
  for rel in hypr/looknfeel.lua hypr/monitors.lua hypr/bindings.lua hypr/input.lua waybar/config.jsonc; do
    local sum
    sum=$(sha256sum "$root/usr/share/omarchy/config/$rel" | cut -d' ' -f1)
    files+="usr/share/omarchy/config/$rel"$'\n'
    mtree+="./usr/share/omarchy/config/$rel time=1.0 size=1 sha256digest=$sum"$'\n'
  done
  local db="$root/var/lib/pacman/local/omarchy-settings-$version"
  mkdir -p "$db"
  printf '%%NAME%%\nomarchy-settings\n\n%%VERSION%%\n%s\n' "$version" > "$db/desc"
  printf '%s' "$files" > "$db/files"
  printf '%s' "$mtree" | gzip -c > "$db/mtree"
}

new_system() {
  local root=$1
  mkdir -p "$root/bin" "$root/home/.local/share" "$root/home/.local/state" "$root/home/.config/hypr" "$root/home/.config/waybar" "$root/runtime" "$root/var/lib/drift"
  ln -s "$BIN" "$root/bin/drift"
  for helper in drift-mark-updated drift-notify-after-upgrade; do ln -s "$PKG/$helper" "$root/bin/$helper"; done
  printf '#!/bin/sh\nprintf "%%s\\n" "$*" >> "%s/notifications.log"\n' "$root" > "$root/bin/omarchy-notification-send"
  chmod +x "$root/bin/omarchy-notification-send"
  write_package "$root" 4.5.0-2 v1
  cp "$root/usr/share/omarchy/config/hypr/looknfeel.lua" "$root/home/.config/hypr/looknfeel.lua"
  printf 'scale = 2\n' > "$root/home/.config/hypr/monitors.lua"
  printf 'a = 7\nb = 2\nc = 3\n' > "$root/home/.config/hypr/bindings.lua"
  printf 'x = 2\n' > "$root/home/.config/hypr/input.lua"
  cp "$root/usr/share/omarchy/config/waybar/config.jsonc" "$root/home/.config/waybar/config.jsonc"
}

run() {
  local root=$1
  shift
  env -i PATH="$root/bin:/usr/bin:/bin" HOME="$root/home" XDG_CONFIG_HOME="$root/home/.config" \
    XDG_DATA_HOME="$root/home/.local/share" XDG_STATE_HOME="$root/home/.local/state" \
    XDG_RUNTIME_DIR="$root/runtime" OMARCHY_PATH="$root/usr/share/omarchy" DRIFT_ROOT="$root" \
    DRIFT_UPDATED_FILE="$root/var/lib/drift/updated" DRIFT_PACMAN_LOCK="$root/var/lib/pacman/db.lck" \
    "$@" || true
}

hook_exec() {
  sed -n 's/^Exec = //p' "$PKG/$1" | sed 's#/usr/bin/##'
}

pacman_upgrade() {
  local root=$1 with_hooks=$2
  if [ "$with_hooks" = yes ]; then
    printf 'PreTransaction hook: %s\n' "$(hook_exec 10-drift-snapshot.hook)"
    run "$root" sh -c "$(hook_exec 10-drift-snapshot.hook)"
  fi
  write_package "$root" 4.5.0-3 v2
  mkdir -p "$root/var/log"
  printf '[2026-11-13T12:00:00+0000] [ALPM] upgraded omarchy-settings (4.5.0-2 -> 4.5.0-3)\n' >> "$root/var/log/pacman.log"
  printf 'package upgraded: omarchy-settings 4.5.0-2 -> 4.5.0-3 (pacman.log records it)\n'
  if [ "$with_hooks" = yes ]; then
    printf 'PostTransaction hook: %s\n' "$(hook_exec 20-drift-updated.hook)"
    run "$root" sh -c "$(hook_exec 20-drift-updated.hook)"
    printf 'then the user unit runs: drift-notify-after-upgrade\n'
    run "$root" drift-notify-after-upgrade
  fi
}

step "0. the hooks that ship with Drift watch the settings package"
for hook in 10-drift-snapshot.hook 20-drift-updated.hook; do
  expect "Target = omarchy-settings" "$(cat "$PKG/$hook")"
  expect "Operation = Upgrade" "$(cat "$PKG/$hook")"
done
expect "PreTransaction" "$(cat "$PKG/10-drift-snapshot.hook")"
expect "PostTransaction" "$(cat "$PKG/20-drift-updated.hook")"
echo "ok: snapshot runs before the upgrade, the stamp and notice after it"

step "1. fresh install: no cached previous package, no snapshots, no recorded bases"
FRESH=$WORK/with-drift
new_system "$FRESH"
[ ! -e "$FRESH/var/cache/pacman" ] && [ ! -e "$FRESH/var/lib/drift/snapshots" ] || fail "the fixture is not fresh"
before=$(run "$FRESH" drift scan)
echo "$before"
expect "(base: none)" "$before"
refute "(base: snapshot)" "$before"
expect "local override" "$before"
expect "base unknown" "$before"

step "2. control: the same upgrade without Drift's hook leaves every file without a base"
CONTROL=$WORK/without-hook
new_system "$CONTROL"
pacman_upgrade "$CONTROL" no
control=$(run "$CONTROL" drift scan)
echo "$control"
refute "(base: snapshot)" "$control"
expect "(base: none)" "$control"

step "3. install Drift's hooks and upgrade omarchy-settings 4.5.0-2 to 4.5.0-3"
pacman_upgrade "$FRESH" yes
[ -d "$FRESH/var/lib/drift/snapshots/4.5.0-2" ] || fail "no snapshot of 4.5.0-2 was taken"
[ -s "$FRESH/var/lib/drift/updated" ] || fail "the upgrade stamp was not written"
echo "snapshot: $(cd "$FRESH/var/lib/drift/snapshots/4.5.0-2" && find . -type f | sort | tr '\n' ' ')"
echo "stamp: $(cat "$FRESH/var/lib/drift/updated")"

step "4. exact bases: every file reports where its base came from"
after=$(run "$FRESH" drift scan)
echo "$after"
expect "(base: snapshot)" "$after"
without=$(grep '\.config/' <<<"$after" | grep -v '^unchanged' | grep -vF '(base: snapshot)' || true)
[ -z "$without" ] || fail "files that differ from the template still have no exact base:"$'\n'"$without"
echo "ok: every file that differs from its template has a snapshot base"
expect "upstream changed  .config/hypr/looknfeel.lua  (base: snapshot)" "$after"
expect "conflict" "$after"
expect ".config/hypr/input.lua  (base: snapshot)" "$after"
expect "merges cleanly" "$after"
expect "local override" "$after"
expect "unchanged" "$after"

step "5. the diff names the base source and the three sides"
diff_out=$(run "$FRESH" drift diff .config/hypr/bindings.lua)
echo "$diff_out"
expect "snapshot" "$diff_out"
expect "a = 7" "$diff_out"
expect "c = 9" "$diff_out"
json=$(run "$FRESH" drift diff .config/hypr/input.lua --json)
expect '"base_provenance":"snapshot"' "$(tr -d ' \n' <<<"$json")"

step "6. the notification"
notice=$(cat "$FRESH/notifications.log" 2>/dev/null || true)
echo "$notice"
expect "3 configs drifted" "$notice"
expect "--app-name" "$notice"
[ ! -e "$CONTROL/notifications.log" ] || fail "the control system sent a notice without the hook"

step "7. keeping a file records an exact base too"
run "$FRESH" drift resolve .config/hypr/input.lua --keep --yes
kept=$(run "$FRESH" drift scan)
expect ".config/hypr/input.lua  (base: reviewed)" "$kept"
expect "local override    .config/hypr/input.lua" "$kept"
expect "2 upstream changed" "$kept"

printf '\nfirst-update scenario passed\n'
