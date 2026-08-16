#!/bin/sh
set -eu

TOKEN=${1-}
EXPECTED_CONFIG=${2-}
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
case "$config_length:$credential_length" in
  *[!0-9:]*|:*|*:) fail candidate_rejected 40 ;;
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
config_candidate=''
credential_candidate=''
credential_created=false
config_replaced=false
CREDENTIAL=''
cleanup() {
  rm -f "$incoming_config" "$incoming_credential"
  [ -z "$config_candidate" ] || rm -f "$config_candidate"
  [ -z "$credential_candidate" ] || rm -f "$credential_candidate"
  [ "$credential_created" = false ] || [ "$config_replaced" = true ] || rm -f "$CREDENTIAL"
}
trap cleanup EXIT HUP INT TERM

dd bs=1 count="$config_length" of="$incoming_config" 2>/dev/null
dd bs=1 count="$credential_length" of="$incoming_credential" 2>/dev/null
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

config_parent=${CONFIG_TARGET%/*}
config_candidate=$(mktemp "$config_parent/.config.gpteasy.XXXXXX")
cat "$incoming_config" >"$config_candidate"
if [ -f "$CONFIG_TARGET" ]; then chmod --reference="$CONFIG_TARGET" "$config_candidate"; else chmod 600 "$config_candidate"; fi

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
  cp -p "$CONFIG_TARGET" "$BACKUP_DIR/config-$stamp.toml"
  chmod 600 "$BACKUP_DIR/config-$stamp.toml"
else
  printf 'missing\n' >"$BACKUP_DIR/config-$stamp.missing"
  chmod 600 "$BACKUP_DIR/config-$stamp.missing"
fi
sync -f "$config_candidate"
validate_config_target || fail concurrent_change 41
if [ "$(hash_file "$CONFIG_TARGET")" != "$EXPECTED_CONFIG" ]; then
  [ "$credential_created" = false ] || rm -f "$CREDENTIAL"
  fail concurrent_change 41
fi
if ! mv "$config_candidate" "$CONFIG_TARGET"; then
  [ "$credential_created" = false ] || rm -f "$CREDENTIAL"
  fail write_failed 44
fi
config_candidate=''
config_replaced=true
sync -f "$config_parent"

find "$BACKUP_DIR" -maxdepth 1 -type f \( -name 'config-*.toml' -o -name 'config-*.missing' \) -printf '%f\n' |
  sort -r | awk 'NR > 5 { print }' | while IFS= read -r stale; do rm -f "$BACKUP_DIR/$stale"; done
printf '%s\n' '{"status":"written","helper":"gpteasy-wsl-guest-writer-v2"}'
