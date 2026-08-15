#!/bin/sh
set -eu

RELATIVE=${1-}
case "$RELATIVE" in
  .gpteasy-shell/credentials/*/*.token) ;;
  *) exit 43 ;;
esac
case "$RELATIVE" in
  *..*|*//*|*[!A-Za-z0-9._/-]*) exit 43 ;;
esac

CODEX_DIR="$HOME/.codex"
STATE_DIR="$CODEX_DIR/.gpteasy-shell"
CREDENTIALS_DIR="$STATE_DIR/credentials"
CREDENTIAL_TAIL=${RELATIVE#'.gpteasy-shell/credentials/'}
SOURCE_DIR="$CREDENTIALS_DIR/${CREDENTIAL_TAIL%%/*}"
for DIRECTORY in "$STATE_DIR" "$CREDENTIALS_DIR" "$SOURCE_DIR"; do
  [ -d "$DIRECTORY" ] && [ ! -L "$DIRECTORY" ] || exit 43
  set -- $(stat -c '%u %a %F' "$DIRECTORY")
  [ "$1" = "$(id -u)" ] && [ "${2#?}" = 00 ] && [ "$3" = directory ] || exit 43
done

TARGET="$CODEX_DIR/$RELATIVE"
if [ ! -e "$TARGET" ] && [ ! -L "$TARGET" ]; then
  exit 44
fi
[ -f "$TARGET" ] && [ ! -L "$TARGET" ] || exit 43
set -- $(stat -c '%u %a %h %F' "$TARGET")
[ "$1" = "$(id -u)" ] && [ "$2" = 600 ] && [ "$3" = 1 ] && [ "$4 $5" = 'regular file' ] || exit 43
cat -- "$TARGET"
