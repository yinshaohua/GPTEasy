#!/bin/sh
set -eu

EXPECTED_HASH=${1:?expected hash is required}
MODE=${2:-normal}
TARGET="$HOME/.codex/config.toml"
DIR=${TARGET%/*}
BACKUP_DIR="$DIR/backups"
SECRET_SENTINEL='spike-013-secret-value'

umask 077
mkdir -p "$DIR" "$BACKUP_DIR"
TMP=$(mktemp "$DIR/.config.gpteasy.XXXXXX")
cleanup() {
  rm -f "$TMP"
}
trap cleanup EXIT HUP INT TERM

cat > "$TMP"

start_count=$(grep -c '^# >>> GPTEasy managed provider >>>$' "$TMP" || true)
end_count=$(grep -c '^# <<< GPTEasy managed provider <<<$' "$TMP" || true)
if [ "$start_count" -ne 1 ] || [ "$end_count" -ne 1 ]; then
  printf '{"status":"candidate_rejected","reason":"managed_marker_count"}\n'
  exit 40
fi

if [ -f "$TARGET" ]; then
  ORIGINAL_HASH=$(sha256sum "$TARGET" | awk '{print $1}')
else
  ORIGINAL_HASH=$(printf '' | sha256sum | awk '{print $1}')
fi
if [ "$ORIGINAL_HASH" != "$EXPECTED_HASH" ]; then
  printf '{"status":"concurrent_change","phase":"initial_hash"}\n'
  exit 41
fi

backup=''
if [ -f "$TARGET" ]; then
  stamp=$(date -u +%Y%m%dT%H%M%S%N)
  backup="$BACKUP_DIR/config-$stamp-$$.toml"
  cp -p "$TARGET" "$backup"
  chmod 600 "$backup"
  chmod --reference="$TARGET" "$TMP"
else
  chmod 600 "$TMP"
fi

sync -f "$TMP"

if [ "$MODE" = 'fail-before-replace' ]; then
  printf '{"status":"injected_failure","phase":"before_replace"}\n'
  exit 42
fi
if [ "$MODE" = 'delay-before-replace' ]; then
  sleep 2
fi

if [ -f "$TARGET" ]; then
  CURRENT_HASH=$(sha256sum "$TARGET" | awk '{print $1}')
else
  CURRENT_HASH=$(printf '' | sha256sum | awk '{print $1}')
fi
if [ "$CURRENT_HASH" != "$EXPECTED_HASH" ]; then
  printf '{"status":"concurrent_change","phase":"pre_replace"}\n'
  exit 43
fi

mv "$TMP" "$TARGET"
trap - EXIT HUP INT TERM
sync -f "$DIR"

find "$BACKUP_DIR" -maxdepth 1 -type f -name 'config-*.toml' -printf '%f\n' |
  sort -r |
  awk 'NR > 5 { print }' |
  while IFS= read -r stale; do
    rm -f "$BACKUP_DIR/$stale"
  done

self_leak=false
parent_leak=false
if tr '\000' '\n' < "/proc/$$/cmdline" | grep -F "$SECRET_SENTINEL" >/dev/null 2>&1; then
  self_leak=true
fi
if [ -r "/proc/$PPID/cmdline" ] &&
  tr '\000' '\n' < "/proc/$PPID/cmdline" | grep -F "$SECRET_SENTINEL" >/dev/null 2>&1; then
  parent_leak=true
fi

backup_count=$(find "$BACKUP_DIR" -maxdepth 1 -type f -name 'config-*.toml' | wc -l | awk '{print $1}')
mode=$(stat -c '%a' "$TARGET")
printf '{"status":"written","backup_count":%s,"mode":"%s","self_cmdline_secret":%s,"parent_cmdline_secret":%s}\n' \
  "$backup_count" "$mode" "$self_leak" "$parent_leak"
