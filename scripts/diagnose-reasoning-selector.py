"""Probe the installed Codex model/list offline, without reading or copying credentials."""
import argparse
import copy
import hashlib
import json
import os
from pathlib import Path
import queue
import subprocess
import tempfile
import threading
import time
import tomllib


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def probe(executable, root, model, catalog, effort=None):
    with tempfile.TemporaryDirectory(prefix="selector-", dir=root) as directory:
        home = Path(directory).resolve()
        assert home.is_relative_to(root.resolve()), "probe cleanup must stay in its workspace"
        config = [f"model = {json.dumps(model)}", 'model_provider = "offline_probe"']
        if catalog is not None:
            (home / "catalog.json").write_text(json.dumps(catalog), encoding="utf-8")
            config.append('model_catalog_json = "catalog.json"')
        if effort:
            config.append(f"model_reasoning_effort = {json.dumps(effort)}")
        config += ['[analytics]', 'enabled = false', '[model_providers.offline_probe]',
                   'name = "Offline selector probe"', 'base_url = "http://127.0.0.1:9/v1"',
                   'wire_api = "responses"', 'requires_openai_auth = false']
        (home / "config.toml").write_text("\n".join(config) + "\n", encoding="utf-8")
        env = os.environ.copy()
        env["CODEX_HOME"] = str(home)
        for key in ["OPENAI_API_KEY", "CODEX_API_KEY", "CODEX_API_URL"]:
            env.pop(key, None)
        process = subprocess.Popen([str(executable), "app-server", "--stdio"],
                                   env=env, cwd=home, stdin=subprocess.PIPE,
                                   stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                                   creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)
        messages = queue.Queue()
        def read():
            for line in process.stdout:
                try:
                    messages.put(json.loads(line))
                except (ValueError, UnicodeError):
                    pass
            messages.put(None)
        reader = threading.Thread(target=read, daemon=True)
        reader.start()
        def send(message):
            process.stdin.write((json.dumps(message) + "\n").encode())
            process.stdin.flush()
        def request(identifier, method, params):
            send({"id": identifier, "method": method, "params": params})
            deadline = time.monotonic() + 15
            while time.monotonic() < deadline:
                message = messages.get(timeout=max(0.01, deadline - time.monotonic()))
                if message is None:
                    return {"probe_error": "process_exited", "exit_code": process.poll()}
                if message.get("id") == identifier:
                    if "error" in message:
                        return {"probe_error": "rpc_error", "code": message["error"].get("code")}
                    return message.get("result", {})
            return {"probe_error": "timeout"}
        try:
            initialized = request(1, "initialize", {"clientInfo": {"name": "gpteasy_selector_probe", "version": "1"},
                                                      "capabilities": {"experimentalApi": False}})
            if "probe_error" in initialized:
                return initialized
            send({"method": "initialized"})
            result = request(2, "model/list", {"includeHidden": True})
            if "probe_error" in result:
                return result
            models = result.get("data", [])
            selected = next((item for item in models if item.get("model") == model or item.get("id") == model), None)
            def summary(item):
                return {key: item.get(key) for key in ["id", "model", "defaultReasoningEffort", "supportedReasoningEfforts"]}
            return {"model_count": len(models), "model_ids": [item.get("model") or item.get("id") for item in models],
                    "selected": summary(selected) if selected else None,
                    "models_with_choices": sum(bool(item.get("supportedReasoningEfforts")) for item in models),
                    "samples": [summary(item) for item in models[:2]]}
        except queue.Empty:
            return {"probe_error": "timeout"}
        finally:
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=3)
            process.stdin.close()
            reader.join(timeout=2)
            process.stdout.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--codex", type=Path, required=True)
    parser.add_argument("--matrix", action="store_true")
    parser.add_argument("--require-choices", action="store_true")
    parser.add_argument("--verify-matrix", action="store_true", help="Assert the isolated differential controls")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    if args.verify_matrix and not args.matrix:
        parser.error("--verify-matrix requires --matrix")
    home = Path(os.environ.get("CODEX_HOME", str(Path.home() / ".codex")))
    source_config = home / "config.toml"
    config = tomllib.loads(source_config.read_text(encoding="utf-8"))
    source_catalog = Path(config["model_catalog_json"])
    if not source_catalog.is_absolute():
        source_catalog = home / source_catalog
    before = {"config": sha(source_config), "catalog": sha(source_catalog)}
    catalog = json.loads(source_catalog.read_text(encoding="utf-8"))
    model = config["model"]
    root = Path(__file__).resolve().parent.parent / "src-tauri" / "target" / "selector-diagnostics"
    root.mkdir(parents=True, exist_ok=True)
    variants = {"current_catalog": (catalog, config.get("model_reasoning_effort"))}
    if args.matrix:
        variants["root_effort_high_only"] = (catalog, "high")
        declared = copy.deepcopy(catalog)
        selected = next(item for item in declared["models"] if item["slug"] == model)
        # Synthetic control ONLY: this is not evidence that the provider supports these levels.
        selected["supported_reasoning_levels"] = [{"effort": effort, "description": "Offline synthetic control"}
                                                  for effort in ["low", "medium", "high", "xhigh"]]
        variants["synthetic_choices_control"] = (declared, config.get("model_reasoning_effort"))
        default_only = copy.deepcopy(catalog)
        next(item for item in default_only["models"] if item["slug"] == model)["default_reasoning_level"] = "high"
        variants["default_level_high_only"] = (default_only, config.get("model_reasoning_effort"))
        omitted = copy.deepcopy(catalog)
        for entry in omitted["models"]:
            entry.pop("supported_reasoning_levels", None)
        variants["omitted_choices_field"] = (omitted, config.get("model_reasoning_effort"))
        variants["no_custom_catalog"] = (None, config.get("model_reasoning_effort"))
    results = {name: probe(args.codex.resolve(), root, model, value, effort)
               for name, (value, effort) in variants.items()}
    unchanged = before == {"config": sha(source_config), "catalog": sha(source_catalog)}
    report = {"selected_model": model, "root_effort": config.get("model_reasoning_effort"),
              "source_model_count": len(catalog["models"]), "user_files_unchanged": unchanged,
              "offline": True, "synthetic_control_is_not_provider_capability_evidence": True,
              "results": results}
    text = json.dumps(report, ensure_ascii=False, indent=2)
    print(text)
    if args.output:
        args.output.write_text(text + "\n", encoding="utf-8", newline="\n")
    assert unchanged, "user config or catalog changed during probe"
    if args.verify_matrix:
        assert all("probe_error" not in result for result in results.values()), "a protocol probe failed"
        def efforts(name):
            selected = results[name].get("selected") or {}
            return [item["reasoningEffort"] for item in selected.get("supportedReasoningEfforts", [])]
        baseline = efforts("current_catalog")
        assert efforts("root_effort_high_only") == baseline, "root effort unexpectedly changed selectable levels"
        assert efforts("default_level_high_only") == baseline, "default level unexpectedly changed selectable levels"
        assert efforts("synthetic_choices_control") == ["low", "medium", "high", "xhigh"], "catalog choices were not exposed"
        assert results["synthetic_choices_control"]["model_ids"] == results["current_catalog"]["model_ids"], "control changed the provider model list"
        assert results["omitted_choices_field"]["model_ids"] == results["no_custom_catalog"]["model_ids"], "omission fallback differed from builtin catalog"
        assert efforts("omitted_choices_field") == efforts("no_custom_catalog"), "omission did not use builtin levels"
        print("PASS: offline differential controls; user files unchanged")
    if args.require_choices:
        selected = results["current_catalog"].get("selected")
        assert selected and selected.get("supportedReasoningEfforts"), "FAIL: current model exposes no selectable reasoning efforts"


if __name__ == "__main__":
    main()
