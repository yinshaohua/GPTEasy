#!/bin/sh
set -eu

MODE=${1-}
TOKEN=${2-}
OPERATION=${3-}
STATE_DIR="$HOME/.codex/.gpteasy-shell"
LOCK_ROOT="$STATE_DIR/lock"
ACTIVE_LOCK="$LOCK_ROOT/active"
OWNER_FILE="$ACTIVE_LOCK/owner"
umask 077

lock_value() {
  awk -F= -v key="$2" '
    $1 == key { print substr($0, length(key) + 2); found += 1 }
    END { if (found != 1) exit 1 }
  ' "$1"
}

case "$MODE" in
  acquire)
    mkdir -p "$HOME/.codex"
    for directory in "$STATE_DIR" "$LOCK_ROOT"; do
      if [ ! -e "$directory" ]; then mkdir -m 700 "$directory"; fi
      [ -d "$directory" ] && [ ! -L "$directory" ] || exit 43
      set -- $(stat -c '%u %a %F' "$directory")
      [ "$1" = "$(id -u)" ] && [ "${2#?}" = 00 ] && [ "$3" = directory ] || exit 43
    done
    if ! mkdir -m 700 "$ACTIVE_LOCK" 2>/dev/null; then
      owner=$(lock_value "$OWNER_FILE" owner 2>/dev/null || printf unknown)
      held=$(lock_value "$OWNER_FILE" operation 2>/dev/null || printf unknown)
      case "$owner" in shell|desktop) ;; *) owner=unknown ;; esac
      case "$held" in *[!a-z_]*) held=unknown ;; esac
      printf 'busy owner=%s operation=%s\n' "$owner" "$held"
      exit 42
    fi
    pid=$$
    start=$(awk '{print $22}' "/proc/$pid/stat")
    if ! {
      printf 'owner=desktop\n'
      printf 'token=%s\n' "$TOKEN"
      printf 'pid=%s\n' "$pid"
      printf 'process_start=%s\n' "$start"
      printf 'operation=%s\n' "$OPERATION"
    } >"$OWNER_FILE"; then
      rm -f "$OWNER_FILE"
      rmdir "$ACTIVE_LOCK" 2>/dev/null || true
      exit 43
    fi
    chmod 600 "$OWNER_FILE"
    printf 'acquired\n'
    ;;
  release)
    if [ ! -e "$ACTIVE_LOCK" ] && [ ! -L "$ACTIVE_LOCK" ]; then
      printf 'absent\n'
      exit 0
    fi
    [ -d "$ACTIVE_LOCK" ] && [ ! -L "$ACTIVE_LOCK" ] || exit 43
    [ -f "$OWNER_FILE" ] && [ ! -L "$OWNER_FILE" ] || exit 43
    [ "$(lock_value "$OWNER_FILE" owner)" = desktop ] || exit 43
    [ "$(lock_value "$OWNER_FILE" token)" = "$TOKEN" ] || exit 43
    rm -f "$OWNER_FILE"
    rmdir "$ACTIVE_LOCK"
    printf 'released\n'
    ;;
  *) exit 40 ;;
esac
