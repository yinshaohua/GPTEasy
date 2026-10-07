#!/bin/sh
set -eu

TOKEN=${1-}
EXPECTED_CONFIG=${2-}
EXPECTED_CATALOG=${3-}
BUNDLE_MAGIC='GPTEASY_WSL_BUNDLE_V2'
TARGET_DIR="$HOME/.codex"
STATE_DIR="$TARGET_DIR/.gpteasy-shell"
CONFIG_ENTRY="$TARGET_DIR/config.toml"
BACKUP_DIR="$STATE_DIR/desktop-backups"
TMP_DIR="$STATE_DIR/tmp"
LOCK_DIR="$STATE_DIR/lock/active"
OWNER_FILE="$LOCK_DIR/owner"
REFERENCES_FILE="$LOCK_DIR/references"
umask 077

fail() {
  printf '{"status":"%s"}\n' "$1"
  exit "$2"
}

lock_value() {
  awk -F= -v key="$2" '
    $1 == key { print substr($0, length(key) + 2); found += 1 }
    END { if (found != 1) exit 1 }
  ' "$1"
}

[ -d "$LOCK_DIR" ] && [ ! -L "$LOCK_DIR" ] || fail lock_lost 43
[ -f "$OWNER_FILE" ] && [ ! -L "$OWNER_FILE" ] || fail lock_lost 43
[ "$(lock_value "$OWNER_FILE" owner)" = desktop ] || fail lock_lost 43
[ "$(lock_value "$OWNER_FILE" token)" = "$TOKEN" ] || fail lock_lost 43

read -r magic
[ "$magic" = "$BUNDLE_MAGIC" ] || fail candidate_rejected 40
read -r config_length
read -r credential_length
read -r catalog_length
case "$config_length:$credential_length:$catalog_length" in
  *[!0-9:]*|:*|*:|*::*) fail candidate_rejected 40 ;;
esac

mkdir -p "$TARGET_DIR"
for directory in "$STATE_DIR" "$BACKUP_DIR" "$TMP_DIR"; do
  if [ ! -e "$directory" ]; then mkdir -m 700 "$directory"; fi
  [ -d "$directory" ] && [ ! -L "$directory" ] || fail unsafe_path 43
  set -- $(stat -c '%u %a %F' "$directory")
  [ "$1" = "$(id -u)" ] && [ "${2#?}" = '00' ] && [ "$3" = directory ] || fail unsafe_path 43
done

incoming_config=$(mktemp "$TMP_DIR/.config.XXXXXX")
incoming_credential=$(mktemp "$TMP_DIR/.credential.XXXXXX")
incoming_catalog=$(mktemp "$TMP_DIR/.catalog.XXXXXX")
config_candidate=''
credential_candidate=''
rollback_candidate=''
catalog_candidate=''
credential_created=false
config_replaced=false
backup_path=''
original_missing=false
original_config_mode=''
CREDENTIAL=''
catalog_backup=''
catalog_replaced=false
original_catalog_missing=true
cleanup() {
  rm -f "$incoming_config" "$incoming_credential" "$incoming_catalog"
  [ -z "$config_candidate" ] || rm -f "$config_candidate"
  [ -z "$credential_candidate" ] || rm -f "$credential_candidate"
  [ -z "$rollback_candidate" ] || rm -f "$rollback_candidate"
  [ -z "$catalog_candidate" ] || rm -f "$catalog_candidate"
  [ -z "$catalog_backup" ] || rm -f "$catalog_backup"
  [ "$credential_created" = false ] || [ "$config_replaced" = true ] || rm -f "$CREDENTIAL"
}
trap cleanup EXIT HUP INT TERM

dd bs=1 count="$config_length" of="$incoming_config" 2>/dev/null
dd bs=1 count="$credential_length" of="$incoming_credential" 2>/dev/null
dd bs=1 count="$catalog_length" of="$incoming_catalog" 2>/dev/null
start_count=$(sed 's/\r$//' "$incoming_config" | grep -c '^# >>> GPTEasy managed provider >>>$' || true)
end_count=$(sed 's/\r$//' "$incoming_config" | grep -c '^# <<< GPTEasy managed provider <<<$' || true)
[ "$start_count" -eq 1 ] && [ "$end_count" -eq 1 ] || fail candidate_rejected 40
schema_count=$(sed 's/\r$//' "$incoming_config" | grep -c '^# GPTEasy schema-version: 1$' || true)
[ "$schema_count" -eq 1 ] || fail candidate_rejected 40

credential_relative=$(awk '
  { sub(/\r$/, "", $0) }
  index($0, "# GPTEasy credential-file:") == 1 {
    value = substr($0, length("# GPTEasy credential-file:") + 1)
    sub(/^[[:space:]]+/, "", value)
    found += 1
  }
  END { if (found != 1) exit 1; print value }
' "$incoming_config") || fail candidate_rejected 40
case "$credential_relative" in
  .gpteasy-shell/credentials/*/*.token) ;;
  *) fail candidate_rejected 40 ;;
esac
case "$credential_relative" in *..*|*//*|*[!A-Za-z0-9._/-]*) fail candidate_rejected 40 ;; esac
credential_tail=${credential_relative#'.gpteasy-shell/credentials/'}
credential_source=${credential_tail%%/*}
credential_file=${credential_tail#*/}
[ -n "$credential_source" ] && [ "$credential_file" != "$credential_tail" ] || fail candidate_rejected 40
case "$credential_file" in */*) fail candidate_rejected 40 ;; esac

CONFIG_TARGET=$CONFIG_ENTRY
CATALOG_TARGET="$TARGET_DIR/gpteasy-model-catalog.json"
CONFIG_IS_SYMLINK=false
if [ -L "$CONFIG_ENTRY" ]; then
  CONFIG_IS_SYMLINK=true
  CONFIG_TARGET=$(readlink -f "$CONFIG_ENTRY") || fail unsafe_path 43
fi

validate_config_target() {
  if [ "$CONFIG_IS_SYMLINK" = true ]; then
    [ -L "$CONFIG_ENTRY" ] || return 1
    [ "$(readlink -f "$CONFIG_ENTRY")" = "$CONFIG_TARGET" ] || return 1
  fi
  if [ -e "$CONFIG_TARGET" ]; then
    [ -f "$CONFIG_TARGET" ] && [ ! -L "$CONFIG_TARGET" ] || return 1
    set -- $(stat -Lc '%u %h %F' "$CONFIG_TARGET")
    [ "$1" = "$(id -u)" ] && [ "$2" = 1 ] && [ "$3 $4" = 'regular file' ] || return 1
  else
    [ "$CONFIG_IS_SYMLINK" = false ] || return 1
  fi
}
validate_config_target || fail unsafe_path 43

validate_catalog_target() {
  if [ -e "$CATALOG_TARGET" ] || [ -L "$CATALOG_TARGET" ]; then
    [ -f "$CATALOG_TARGET" ] && [ ! -L "$CATALOG_TARGET" ] || return 1
    set -- $(stat -Lc '%u %h %F' "$CATALOG_TARGET")
    [ "$1" = "$(id -u)" ] && [ "$2" = 1 ] && [ "$3 $4" = 'regular file' ] || return 1
  fi
}
validate_catalog_target || fail unsafe_path 43

old_credential_relative=''
if [ -f "$CONFIG_TARGET" ]; then
  old_credential_relative=$(awk '
    { sub(/\r$/, "", $0) }
    index($0, "# GPTEasy credential-file:") == 1 {
      value = substr($0, length("# GPTEasy credential-file:") + 1)
      sub(/^[[:space:]]+/, "", value)
      found += 1
    }
    END {
      if (found > 1) exit 2
      if (found == 1) print value
    }
  ' "$CONFIG_TARGET") || fail candidate_rejected 40
fi
for reference in "$credential_relative" "$old_credential_relative"; do
  [ -n "$reference" ] || continue
  case "$reference" in
    .gpteasy-shell/credentials/*/*.token) ;;
    *) fail candidate_rejected 40 ;;
  esac
  case "$reference" in *..*|*//*|*[!A-Za-z0-9._/-]*) fail candidate_rejected 40 ;; esac
done
{
  printf '%s\n' "$credential_relative"
  [ -z "$old_credential_relative" ] || printf '%s\n' "$old_credential_relative"
} >"$REFERENCES_FILE"
chmod 600 "$REFERENCES_FILE"
sync -f "$REFERENCES_FILE"

hash_file() {
  if [ -f "$1" ]; then sha256sum "$1" | awk '{print $1}'; else printf 'missing\n'; fi
}
[ "$(hash_file "$CONFIG_TARGET")" = "$EXPECTED_CONFIG" ] || fail concurrent_change 41
[ "$(hash_file "$CATALOG_TARGET")" = "$EXPECTED_CATALOG" ] || fail concurrent_change 41

catalog_backup=$(mktemp "$TMP_DIR/.catalog.backup.XXXXXX")
if [ -f "$CATALOG_TARGET" ]; then
  original_catalog_missing=false
  cp -p "$CATALOG_TARGET" "$catalog_backup"
fi
config_parent=${CONFIG_TARGET%/*}
catalog_parent=${CATALOG_TARGET%/*}
rollback_config() {
  if [ "$catalog_replaced" = true ]; then
    if [ "$original_catalog_missing" = true ]; then
      rm -f "$CATALOG_TARGET" || return 1
    else
      catalog_candidate=$(mktemp "$catalog_parent/.catalog.gpteasy.rollback.XXXXXX") || return 1
      cp -p "$catalog_backup" "$catalog_candidate" || return 1
      sync -f "$catalog_candidate" || return 1
      mv "$catalog_candidate" "$CATALOG_TARGET" || return 1
      catalog_candidate=''
    fi
    sync -f "$catalog_parent" || return 1
  fi
  if [ "$original_missing" = true ]; then
    rm -f "$CONFIG_TARGET" || return 1
  else
    rollback_candidate=$(mktemp "$config_parent/.config.gpteasy.rollback.XXXXXX") || return 1
    cat "$backup_path" >"$rollback_candidate" || return 1
    chmod "$original_config_mode" "$rollback_candidate" || return 1
    sync -f "$rollback_candidate" || return 1
    mv "$rollback_candidate" "$CONFIG_TARGET" || return 1
    rollback_candidate=''
  fi
  sync -f "$config_parent"
}
config_candidate=$(mktemp "$config_parent/.config.gpteasy.XXXXXX")
catalog_candidate=$(mktemp "$catalog_parent/.catalog.gpteasy.XXXXXX")
cat "$incoming_config" >"$config_candidate"
cat "$incoming_catalog" >"$catalog_candidate"
if [ -f "$CONFIG_TARGET" ]; then chmod --reference="$CONFIG_TARGET" "$config_candidate"; else chmod 600 "$config_candidate"; fi
chmod 600 "$catalog_candidate"

CREDENTIAL="$TARGET_DIR/$credential_relative"
credential_directory=${CREDENTIAL%/*}
credentials_root="$STATE_DIR/credentials"
for directory in "$credentials_root" "$credential_directory"; do
  if [ ! -e "$directory" ]; then mkdir -m 700 "$directory"; fi
  [ -d "$directory" ] && [ ! -L "$directory" ] || fail unsafe_path 43
  set -- $(stat -c '%u %a %F' "$directory")
  [ "$1" = "$(id -u)" ] && [ "${2#?}" = '00' ] && [ "$3" = directory ] || fail unsafe_path 43
done
if [ -e "$CREDENTIAL" ] || [ -L "$CREDENTIAL" ]; then
  [ -f "$CREDENTIAL" ] && [ ! -L "$CREDENTIAL" ] || fail credential_conflict 46
  set -- $(stat -c '%u %a %h %F' "$CREDENTIAL")
  [ "$1" = "$(id -u)" ] && [ "${2#?}" = '00' ] && [ "$3" = 1 ] && [ "$4 $5" = 'regular file' ] || fail credential_conflict 46
  cmp -s "$incoming_credential" "$CREDENTIAL" || fail credential_conflict 46
else
  credential_candidate=$(mktemp "$credential_directory/.credential.XXXXXX")
  cat "$incoming_credential" >"$credential_candidate"
  chmod 600 "$credential_candidate"
  sync -f "$credential_candidate"
  mv "$credential_candidate" "$CREDENTIAL"
  credential_candidate=''
  credential_created=true
fi

stamp=$(date -u +%Y%m%dT%H%M%S%N)-$$
if [ -f "$CONFIG_TARGET" ]; then
  backup_path="$BACKUP_DIR/config-$stamp.toml"
  original_config_mode=$(stat -c '%a' "$CONFIG_TARGET")
  cp -p "$CONFIG_TARGET" "$backup_path"
  chmod 600 "$backup_path"
else
  original_missing=true
  backup_path="$BACKUP_DIR/config-$stamp.missing"
  printf 'missing\n' >"$backup_path"
  chmod 600 "$backup_path"
fi
sync -f "$backup_path"
sync -f "$BACKUP_DIR"
sync -f "$config_candidate"
sync -f "$catalog_candidate"
validate_catalog_target || fail concurrent_change 41
[ "$(hash_file "$CATALOG_TARGET")" = "$EXPECTED_CATALOG" ] || fail concurrent_change 41
validate_config_target || fail concurrent_change 41
if [ "$(hash_file "$CONFIG_TARGET")" != "$EXPECTED_CONFIG" ]; then
  [ "$credential_created" = false ] || rm -f "$CREDENTIAL"
  fail concurrent_change 41
fi
if ! mv "$config_candidate" "$CONFIG_TARGET"; then
  [ "$credential_created" = false ] || rm -f "$CREDENTIAL"
  fail write_failed 44
fi

if ! mv "$catalog_candidate" "$CATALOG_TARGET"; then
  if rollback_config; then
    config_replaced=false
    fail write_failed 44
  fi
  fail rollback_failed 45
fi
catalog_candidate=''
catalog_replaced=true
config_candidate=''
config_replaced=true
if ! sync -f "$catalog_parent" || ! sync -f "$config_parent"; then
  if rollback_config; then
    config_replaced=false
    fail write_failed 44
  fi
  fail rollback_failed 45
fi

find "$BACKUP_DIR" -maxdepth 1 -type f \( -name 'config-*.toml' -o -name 'config-*.missing' \) -printf '%f\n' |
  sort -r | awk 'NR > 5 { print }' | while IFS= read -r stale; do rm -f "$BACKUP_DIR/$stale"; done || true
printf '%s\n' '{"status":"written","helper":"gpteasy-wsl-guest-writer-v3"}' || true
