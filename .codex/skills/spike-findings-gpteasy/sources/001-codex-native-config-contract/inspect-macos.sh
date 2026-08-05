#!/usr/bin/env bash
set -eu

root="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
out="$root/.run/macos-evidence.json"
mkdir -p "$(dirname "$out")"

python3 - "$out" <<'PY'
import json
import os
import platform
import shutil
import subprocess
import sys
from pathlib import Path

out = Path(sys.argv[1])
home = Path.home()
codex_home = Path(os.environ.get("CODEX_HOME", home / ".codex"))

def run(*args):
    try:
        return subprocess.check_output(args, text=True, stderr=subprocess.DEVNULL).strip()
    except (OSError, subprocess.CalledProcessError):
        return ""

processes = []
for name in ("codex", "Codex", "ChatGPT", "ChatGPT.app"):
    if shutil.which("pgrep"):
        output = run("pgrep", "-alf", name)
        if output:
            processes.extend(
                {"match": name, "line": line, "command_line_has_codex_home": "CODEX_HOME" in line}
                for line in output.splitlines()
            )

evidence = {
    "os": "macos",
    "captured_at": run("date", "-u", "+%Y-%m-%dT%H:%M:%SZ"),
    "os_version": platform.platform(),
    "architecture": platform.machine(),
    "home": str(home),
    "codex_home": str(codex_home),
    "config_toml": str(codex_home / "config.toml"),
    "auth_json": str(codex_home / "auth.json"),
    "codex_cli": run("codex", "--version") if shutil.which("codex") else "",
    "processes": processes,
}
out.write_text(json.dumps(evidence, indent=2) + "\n", encoding="utf-8")
print(out.read_text(encoding="utf-8"))
PY
