#!/bin/zsh
set -euo pipefail

SCRIPT_DIR=${0:A:h}
RUN_DIR="$SCRIPT_DIR/.run"
APP_NAME="GPTEasy Spike 017.app"
USER_APP="$HOME/Applications/$APP_NAME"

if [[ "$(uname -s)" != "Darwin" ]]; then
  print -u2 "Spike 017 must run on macOS."
  exit 1
fi

major=$(/usr/bin/sw_vers -productVersion | /usr/bin/awk -F. '{print $1}')
if (( major < 14 )); then
  print -u2 "macOS 14 or newer is required."
  exit 1
fi

mkdir -p "$RUN_DIR" "$HOME/Applications"
cd "$SCRIPT_DIR"

npm install
npm run tauri build -- --bundles app

bundle=$(find src-tauri/target -type d -path '*/bundle/macos/GPTEasy Spike 017.app' -print -quit)
if [[ -z "$bundle" ]]; then
  print -u2 "Built .app bundle was not found."
  exit 1
fi

rm -rf "$USER_APP"
/usr/bin/ditto "$bundle" "$USER_APP"

scope="invalid"
if [[ "$USER_APP" == "$HOME/Applications/"* ]]; then
  scope="current_user"
fi

minimum=$(/usr/libexec/PlistBuddy -c 'Print :LSMinimumSystemVersion' "$USER_APP/Contents/Info.plist")
bundle_id=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$USER_APP/Contents/Info.plist")
signature=$(/usr/bin/codesign --verify --deep --strict "$USER_APP" >/dev/null 2>&1 && print verified || print unverified)
gatekeeper=$(/usr/sbin/spctl --assess --type execute "$USER_APP" >/dev/null 2>&1 && print accepted || print rejected)

/usr/bin/open "$USER_APP"
sleep 3

cat > "$RUN_DIR/macos-host-summary.json" <<JSON
{
  "generated_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "macos_version": "$(/usr/bin/sw_vers -productVersion)",
  "architecture": "$(uname -m)",
  "bundle_id": "$bundle_id",
  "minimum_system_version": "$minimum",
  "install_scope": "$scope",
  "installed_path": "$USER_APP",
  "codesign_verify": "$signature",
  "gatekeeper": "$gatekeeper",
  "interactive_checks_required": [
    "tray icon visible",
    "window close keeps process alive",
    "tray show restores window",
    "explicit tray exit terminates process",
    "Codex or ChatGPT real process topology",
    "two-version signed updater preserves canary"
  ]
}
JSON

cat "$RUN_DIR/macos-host-summary.json"
print
print "Continue in the app UI, complete the four manual lifecycle checks, write/export the canary evidence, then retain:"
print "  $HOME/Library/Application Support/com.gpteasy.spike.macos-contract/macos-contract-evidence.json"
