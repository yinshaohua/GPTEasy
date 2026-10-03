#!/bin/sh
set -eu

TOKEN=${1-}
EXPECTED_CONFIG=${2-}
BUNDLE_MAGIC='GPTEASY_WSL_BUNDLE_V3'
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
read -r catalog_relative
read -r catalog_artifact
read -r catalog_sha256
read -r catalog_fingerprint
case "$config_length:$credential_length:$catalog_length" in
  *[!0-9:]*|:*|*::*) fail candidate_rejected 40 ;;
esac
for length in "$config_length" "$credential_length" "$catalog_length"; do
  [ -n "$length" ] && [ "$length" -gt 0 ] && [ "$length" -le 67108864 ] || fail candidate_rejected 40
done

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
catalog_candidate=''
config_candidate=''
credential_candidate=''
rollback_candidate=''
credential_created=false
config_replaced=false
backup_path=''
original_missing=false
original_config_mode=''
CREDENTIAL=''
cleanup() {
  rm -f "$incoming_config" "$incoming_credential" "$incoming_catalog"
  [ -z "$catalog_candidate" ] || rm -f "$catalog_candidate"
  [ -z "$config_candidate" ] || rm -f "$config_candidate"
  [ -z "$credential_candidate" ] || rm -f "$credential_candidate"
  [ -z "$rollback_candidate" ] || rm -f "$rollback_candidate"
  [ "$credential_created" = false ] || [ "$config_replaced" = true ] || rm -f "$CREDENTIAL"
}
trap cleanup EXIT HUP INT TERM

dd bs=1 count="$config_length" of="$incoming_config" 2>/dev/null
dd bs=1 count="$credential_length" of="$incoming_credential" 2>/dev/null
dd bs=1 count="$catalog_length" of="$incoming_catalog" 2>/dev/null
[ "$(wc -c <"$incoming_config")" -eq "$config_length" ] &&
  [ "$(wc -c <"$incoming_credential")" -eq "$credential_length" ] &&
  [ "$(wc -c <"$incoming_catalog")" -eq "$catalog_length" ] || fail candidate_rejected 40
[ "$(dd bs=1 count=1 2>/dev/null | wc -c)" -eq 0 ] || fail candidate_rejected 40
start_count=$(sed 's/\r$//' "$incoming_config" | grep -c '^# >>> GPTEasy managed provider >>>$' || true)
end_count=$(sed 's/\r$//' "$incoming_config" | grep -c '^# <<< GPTEasy managed provider <<<$' || true)
[ "$start_count" -eq 1 ] && [ "$end_count" -eq 1 ] || fail candidate_rejected 40
schema_count=$(sed 's/\r$//' "$incoming_config" | grep -c '^# GPTEasy schema-version: 2$' || true)
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

metadata() {
  awk -v prefix="# GPTEasy $1:" '
    { sub(/\r$/, "", $0) }
    index($0, prefix) == 1 { value = substr($0, length(prefix) + 1); sub(/^[[:space:]]+/, "", value); found++ }
    END { if (found != 1 || value == "") exit 1; print value }
  ' "$incoming_config"
}
# Decode the generated single-line TOML strings without evaluating shell text.
# Rust owns full TOML/schema validation; this check binds the actual guest bytes.
toml_string() {
  awk -v prefix="$1 = " '
    index($0, prefix) == 1 {
      found++; value = substr($0, length(prefix) + 1); sub(/\r$/, "", value)
      quote = substr(value, 1, 1)
      if (quote != "\"" && quote != sprintf("%c", 39)) exit 1
      if (substr(value, length(value), 1) != quote) exit 1
      value = substr(value, 2, length(value)-2)
      if (quote == sprintf("%c", 39)) { result=value; next }
      for (i=1; i<=length(value); i++) {
        c=substr(value,i,1)
        if (c == "\\") {
          c=substr(value,++i,1)
          if (c == "\\" || c == "\"") result=result c
          else if (c == "t") result=result "\t"
          else if (c == "n") result=result "\n"
          else if (c == "r") result=result "\r"
          else exit 1
        } else result=result c
      }
    }
    END { if (found != 1) exit 1; printf "%s", result }
  ' "$incoming_config"
}
provider_id=$(metadata provider-id) || fail catalog_binding_invalid 48
source_id=$(metadata source-id) || fail catalog_binding_invalid 48
[ "$credential_relative" = ".gpteasy-shell/credentials/$source_id/$provider_id.token" ] || fail catalog_binding_invalid 48
[ "$(metadata model-catalog-protocol)" = codex-model-catalog-v1 ] &&
  [ "$(metadata model-catalog-policy)" = common-reasoning-selector-v1 ] &&
  [ "$(metadata model-catalog-file)" = "$catalog_relative" ] &&
  [ "$(metadata model-catalog-artifact)" = "$catalog_artifact" ] &&
  [ "$(metadata model-catalog-sha256)" = "$catalog_sha256" ] &&
  [ "$(metadata model-catalog-provider-fingerprint)" = "$catalog_fingerprint" ] || fail catalog_binding_invalid 48
case "$catalog_relative" in *..*|*//*|*[!A-Za-z0-9._/-]*) fail catalog_binding_invalid 48 ;; esac
[ "$catalog_relative" = ".gpteasy-shell/model-catalogs/$source_id/$catalog_artifact.json" ] || fail catalog_binding_invalid 48
for value in "$provider_id" "$catalog_artifact"; do
  printf '%s\n' "$value" | grep -Eq '^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$' || fail catalog_binding_invalid 48
done
for value in "$catalog_sha256" "$catalog_fingerprint"; do
  printf '%s\n' "$value" | grep -Eq '^[0-9a-f]{64}$' || fail catalog_binding_invalid 48
done
[ "$(sha256sum "$incoming_catalog" | awk '{print $1}')" = "$catalog_sha256" ] || fail catalog_corrupt 48
[ "$(toml_string model_catalog_json)" = "$TARGET_DIR/$catalog_relative" ] || fail catalog_binding_invalid 48
base_url=$(toml_string model_providers.gpteasy.base_url) || fail catalog_binding_invalid 48
model=$(toml_string model) || fail catalog_binding_invalid 48
actual_fingerprint=$({ printf 'gpteasy-provider-combination-v1\0%s\0%s\0' "$base_url" "$model"; cat "$incoming_credential"; } | sha256sum | awk '{print $1}')
[ "$actual_fingerprint" = "$catalog_fingerprint" ] || fail catalog_binding_invalid 48

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
rollback_config() {
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
# Catalogs remain immutable, including unreferenced files left by interruption.
CATALOG="$TARGET_DIR/$catalog_relative"
catalog_root="$STATE_DIR/model-catalogs"
catalog_directory=${CATALOG%/*}
for directory in "$catalog_root" "$catalog_directory"; do
  if [ ! -e "$directory" ]; then mkdir -m 700 "$directory"; fi
  [ -d "$directory" ] && [ ! -L "$directory" ] || fail unsafe_path 43
  set -- $(stat -c '%u %a %F' "$directory")
  [ "$1" = "$(id -u)" ] && [ "${2#?}" = '00' ] && [ "$3" = directory ] || fail unsafe_path 43
done
catalog_is_safe() {
  [ -f "$CATALOG" ] && [ ! -L "$CATALOG" ] || return 1
  set -- $(stat -c '%u %a %h %F' "$CATALOG")
  [ "$1" = "$(id -u)" ] && [ "$2" = 600 ] && [ "$3" = 1 ] && [ "$4 $5" = 'regular file' ]
}
if [ -e "$CATALOG" ] || [ -L "$CATALOG" ]; then
  catalog_is_safe && cmp -s "$incoming_catalog" "$CATALOG" || fail catalog_conflict 47
else
  catalog_candidate=$(mktemp "$catalog_directory/.catalog.XXXXXX")
  cat "$incoming_catalog" >"$catalog_candidate"
  chmod 600 "$catalog_candidate"
  sync -f "$catalog_candidate"
  mv -n "$catalog_candidate" "$CATALOG" || fail catalog_conflict 47
  [ ! -e "$catalog_candidate" ] || fail catalog_conflict 47
  catalog_candidate=''
  sync -f "$catalog_directory"
fi
catalog_is_safe && cmp -s "$incoming_catalog" "$CATALOG" || fail catalog_conflict 47
printf '%s\n' '[GPTEasy] stage=catalog_commit catalog_state=ready' >&2
sync -f "$config_candidate"
validate_config_target || fail concurrent_change 41
if [ "$(hash_file "$CONFIG_TARGET")" != "$EXPECTED_CONFIG" ]; then
  [ "$credential_created" = false ] || rm -f "$CREDENTIAL"
  fail concurrent_change 41
fi
catalog_is_safe && cmp -s "$incoming_catalog" "$CATALOG" || fail catalog_conflict 47
if ! mv "$config_candidate" "$CONFIG_TARGET"; then
  [ "$credential_created" = false ] || rm -f "$CREDENTIAL"
  fail write_failed 44
fi
config_candidate=''
config_replaced=true
if ! sync -f "$config_parent"; then
  if rollback_config; then
    config_replaced=false
    fail write_failed 44
  fi
  fail rollback_failed 45
fi

printf '%s\n' '[GPTEasy] stage=config_commit catalog_state=ready' >&2
validate_config_target && cmp -s "$incoming_config" "$CONFIG_TARGET" &&
  cmp -s "$incoming_credential" "$CREDENTIAL" &&
  catalog_is_safe && cmp -s "$incoming_catalog" "$CATALOG" || fail reread_failed 49
printf '%s\n' '[GPTEasy] stage=artifact_reread catalog_state=verified' >&2

find "$BACKUP_DIR" -maxdepth 1 -type f \( -name 'config-*.toml' -o -name 'config-*.missing' \) -printf '%f\n' |
  sort -r | awk 'NR > 5 { print }' | while IFS= read -r stale; do rm -f "$BACKUP_DIR/$stale"; done || true
printf '%s\n' '{"status":"written","helper":"gpteasy-wsl-guest-writer-v3"}' || true
