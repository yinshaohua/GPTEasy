#!/bin/sh
set -eu

TOKEN=${1-}
TARGET_DIR="${CODEX_HOME:-$HOME/.codex}"
CONFIG_ENTRY="$TARGET_DIR/config.toml"
CONFIG_TARGET="$CONFIG_ENTRY"
STATE_DIR="$TARGET_DIR/.gpteasy-shell"
CREDENTIALS_ROOT="$STATE_DIR/credentials"
SHELL_RESTORE_ROOT="$STATE_DIR/shell-restore"
DESKTOP_BACKUP_ROOT="$STATE_DIR/desktop-backups"
TMP_DIR="$STATE_DIR/tmp"
LOCK_DIR="$STATE_DIR/lock/active"
OWNER_FILE="$LOCK_DIR/owner"
REFERENCES_FILE="$LOCK_DIR/references"
umask 077

lock_value() {
  awk -F= -v key="$2" '
    $1 == key { print substr($0, length(key) + 2); found += 1 }
    END { if (found != 1) exit 1 }
  ' "$1"
}

private_directory() {
  [ -d "$1" ] && [ ! -L "$1" ] || return 1
  set -- $(stat -c '%u %a %F' "$1")
  [ "$1" = "$(id -u)" ] && [ "${2#?}" = 00 ] && [ "$3" = directory ]
}

private_file() {
  [ -f "$1" ] && [ ! -L "$1" ] || return 1
  set -- $(stat -c '%u %a %h %F' "$1")
  [ "$1" = "$(id -u)" ] && [ "${2#?}" = 00 ] && [ "$3" = 1 ] && [ "$4 $5" = 'regular file' ]
}

owned_regular_file() {
  [ -f "$1" ] && [ ! -L "$1" ] || return 1
  set -- $(stat -c '%u %h %F' "$1")
  [ "$1" = "$(id -u)" ] && [ "$2" = 1 ] && [ "$3 $4" = 'regular file' ]
}

valid_reference() {
  case "$1" in
    .gpteasy-shell/credentials/*/*.token) ;;
    *) return 1 ;;
  esac
  case "$1" in *..*|*//*|*[!A-Za-z0-9._/-]*) return 1 ;; esac
  tail=${1#'.gpteasy-shell/credentials/'}
  source=${tail%%/*}
  file=${tail#*/}
  [ -n "$source" ] && [ "$file" != "$tail" ] || return 1
  case "$file" in */*) return 1 ;; esac
}

collect_config_reference() {
  file=$1
  references=$2
  require_private=${3:-1}
  [ -e "$file" ] || return 0
  if [ "$require_private" = 1 ]; then
    private_file "$file" || return 1
  else
    owned_regular_file "$file" || return 1
  fi
  reference=$(awk '
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
  ' "$file") || return 1
  [ -n "$reference" ] || return 0
  valid_reference "$reference" || return 1
  printf '%s\n' "$reference" >>"$references"
}

private_directory "$LOCK_DIR" || exit 43
private_file "$OWNER_FILE" || exit 43
[ "$(lock_value "$OWNER_FILE" owner)" = desktop ] || exit 43
[ "$(lock_value "$OWNER_FILE" token)" = "$TOKEN" ] || exit 43
[ ! -e "$CREDENTIALS_ROOT" ] && exit 0
private_directory "$STATE_DIR" || exit 43
private_directory "$CREDENTIALS_ROOT" || exit 43
private_directory "$TMP_DIR" || exit 43
if [ -L "$CONFIG_ENTRY" ]; then
  CONFIG_TARGET=$(readlink -f "$CONFIG_ENTRY") || exit 43
  [ -f "$CONFIG_TARGET" ] && [ ! -L "$CONFIG_TARGET" ] || exit 43
elif [ -e "$CONFIG_ENTRY" ]; then
  [ -f "$CONFIG_ENTRY" ] || exit 43
fi

references=$(mktemp "$TMP_DIR/.credential-references.XXXXXX")
trap 'rm -f "$references"' EXIT HUP INT TERM
chmod 600 "$references"
collect_config_reference "$CONFIG_TARGET" "$references" 0 || exit 47

for root in "$SHELL_RESTORE_ROOT" "$DESKTOP_BACKUP_ROOT"; do
  [ -e "$root" ] || continue
  private_directory "$root" || exit 43
  [ -z "$(find "$root" -type l -print -quit)" ] || exit 43
  while IFS= read -r file; do
    [ -n "$file" ] || continue
    collect_config_reference "$file" "$references" 1 || exit 47
  done <<EOF
$(find "$root" -type f -name '*.toml' -print)
EOF
done

if [ -e "$REFERENCES_FILE" ]; then
  private_file "$REFERENCES_FILE" || exit 43
  while IFS= read -r reference; do
    [ -n "$reference" ] || continue
    valid_reference "$reference" || exit 47
    printf '%s\n' "$reference" >>"$references"
  done <"$REFERENCES_FILE"
fi

[ -z "$(find "$CREDENTIALS_ROOT" -type l -print -quit)" ] || exit 43
while IFS= read -r credential; do
  [ -n "$credential" ] || continue
  private_file "$credential" || exit 43
  relative=${credential#"$TARGET_DIR/"}
  valid_reference "$relative" || exit 43
  grep -Fqx -- "$relative" "$references" || rm -f -- "$credential"
done <<EOF
$(find "$CREDENTIALS_ROOT" -mindepth 2 -maxdepth 2 -type f -name '*.token' -print)
EOF

find "$CREDENTIALS_ROOT" -mindepth 1 -maxdepth 1 -type d -empty -exec rmdir -- {} \;
printf '%s\n' '{"status":"credentials_cleaned"}'
