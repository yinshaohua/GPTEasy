# WSL explicit apply only. Isolated Linux CLI probe; never controls a user daemon.
gpteasy__check_codex_compatibility() {
    local entry resolved magic
    # Search PATH through an external shell: aliases/functions cannot select the CLI.
    entry=$(sh -c 'command -v codex' 2>/dev/null) || {
        printf '%s\n' '未找到原生 Linux Codex CLI；目录能力未核验，未修改配置。' >&2
        gpteasy__evidence cli_probe unavailable
        return 1
    }
    case "$entry" in /*) ;; *) return 1 ;; esac
    resolved=$(readlink -f -- "$entry") || return 1
    magic=$(head -c 2 -- "$resolved" 2>/dev/null) || return 1
    case "$resolved" in *.exe | *.EXE | *.cmd | *.bat) magic=MZ ;; esac
    if [[ "$magic" == MZ || ! -f "$resolved" || ! -x "$resolved" ]]; then
        printf '%s\n' '拒绝 Windows 互操作入口；请在目标 Linux 用户环境安装原生 Codex CLI。' >&2
        gpteasy__evidence cli_probe interop_rejected
        return 1
    fi
    gpteasy__native_codex=$resolved
    if ! gpteasy__probe_catalog_capability; then
        printf '%s\n' 'Codex CLI 的模型目录/schema 能力核验失败，未修改配置。请按该 CLI 的官方说明检查兼容性。' >&2
        gpteasy__evidence cli_probe schema_rejected
        return 1
    fi
    gpteasy__evidence cli_probe compatible
}

gpteasy__evidence() {
    # Only fixed stage/state labels enter public diagnostics; never provider data.
    printf '[GPTEasy] stage=%s catalog_state=%s\n' "$1" "$2" >&2
}

gpteasy__probe_reply_is_valid() {
    # Parse JSON for observation only. No JSON is reconstructed and no supplier
    # text is executed. The isolated probe must return exactly its one model.
    awk -v expected_id="${1:-2}" '
        function fail() { bad = 1; exit 1 }
        function ws() { while (substr(text, pos, 1) ~ /[ \t\r\n]/ && pos <= length(text)) pos++ }
        function string(    start,c,escaped) {
            if (substr(text,pos++,1) != "\"") fail()
            start = pos
            while (pos <= length(text)) {
                c = substr(text,pos++,1)
                if (c == "\"") return substr(text,start,pos-start-1)
                if (c == "\\") {
                    c = substr(text,pos++,1)
                    if (c == "u") {
                        if (substr(text,pos,4) !~ /^[0-9a-fA-F][0-9a-fA-F][0-9a-fA-F][0-9a-fA-F]$/) fail()
                        pos += 4
                    } else if (c !~ /^["\\\/bfnrt]$/) fail()
                } else if (c ~ /[[:cntrl:]]/) fail()
            }
            fail()
        }
        function value(path,depth,    c,k,n,v,start,token) {
            if (depth > 32) fail()
            ws(); c = substr(text,pos,1)
            if (c == "{") {
                pos++; ws()
                if (substr(text,pos,1) == "}") { pos++; return }
                do {
                    ws(); k = string(); ws()
                    if (seen[path "/" k]++) fail()
                    if (substr(text,pos++,1) != ":") fail()
                    value(path "/" k,depth+1); ws(); c = substr(text,pos++,1)
                    if (c != "," && c != "}") fail()
                } while (c == ",")
            } else if (c == "[") {
                pos++; ws(); n = 0
                if (substr(text,pos,1) == "]") { pos++; sizes[path]=0; return }
                do {
                    value(path "/" n++,depth+1); ws(); c = substr(text,pos++,1)
                    if (c != "," && c != "]") fail()
                } while (c == ",")
                sizes[path] = n
            } else if (c == "\"") {
                leaves[path] = string()
            } else {
                start = pos
                while (pos <= length(text) && substr(text,pos,1) !~ /[]},[:space:]]/) pos++
                token = substr(text,start,pos-start)
                if (token !~ /^(true|false|null|-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?)$/) fail()
                leaves[path] = token
            }
        }
        { text = text $0 }
        END {
            if (bad) exit 1
            pos = 1; value("",0); ws()
            if (expected_id == 1) exit pos > length(text) && leaves["/id"] == "1" && seen["/result"] && !seen["/error"] ? 0 : 1
            valid = pos > length(text) && leaves["/id"] == "2" && !seen["/error"] &&
                sizes["/result/data"] == 1 && leaves["/result/data/0/model"] == "gpteasy-catalog-schema-probe-v1" &&
                leaves["/result/data/0/defaultReasoningEffort"] == "high" &&
                sizes["/result/data/0/supportedReasoningEfforts"] == 4 &&
                leaves["/result/data/0/supportedReasoningEfforts/0/reasoningEffort"] == "low" &&
                leaves["/result/data/0/supportedReasoningEfforts/1/reasoningEffort"] == "medium" &&
                leaves["/result/data/0/supportedReasoningEfforts/2/reasoningEffort"] == "high" &&
                leaves["/result/data/0/supportedReasoningEfforts/3/reasoningEffort"] == "xhigh"
            exit valid ? 0 : 1
        }
    '
}

gpteasy__probe_catalog_capability() (
    local probe pid= line attempts=0 initialized=0
    command -v timeout >/dev/null 2>&1 || return 1
    probe=$(mktemp -d "$gpteasy__tmp_root/.catalog-probe.XXXXXX") || return 1
    trap 'if [[ -n "$pid" ]]; then kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true; fi; exec 8>&- 9>&-; rm -rf -- "$probe"' EXIT
    chmod 700 "$probe" || return 1
    gpteasy__catalog_probe_payload >"$probe/catalog.json" || return 1
    {
        printf '%s\n' 'model = "gpteasy-catalog-schema-probe-v1"'
        printf 'model_catalog_json = %s\n' "$(gpteasy__toml_string "$probe/catalog.json")"
        printf '%s\n' 'model_provider = "gpteasy_probe"' '[model_providers.gpteasy_probe]' 'name = "GPTEasy schema probe"' \
            'base_url = "http://127.0.0.1:9/v1"' 'wire_api = "responses"' 'requires_openai_auth = true' 'supports_websockets = false'
    } >"$probe/config.toml"
    printf '%s' '{"auth_mode":"apikey","OPENAI_API_KEY":"gpteasy-isolated-probe"}' >"$probe/auth.json"
    chmod 600 "$probe/catalog.json" "$probe/config.toml" "$probe/auth.json" || return 1
    mkfifo -m 600 "$probe/input" "$probe/output" || return 1
    exec 8<>"$probe/input" 9<>"$probe/output" || return 1
    cd -- "$probe" || return 1
    CODEX_HOME="$probe" timeout -k 1 8 "$gpteasy__native_codex" app-server --listen stdio:// <"$probe/input" >"$probe/output" 2>"$probe/diagnostics" &
    pid=$!
    printf '%s\n' '{"id":1,"method":"initialize","params":{"clientInfo":{"name":"gpteasy_catalog_probe","version":"1"}}}' >&8
    while (( attempts < 30 )); do
        if ! IFS= read -r -t 1 -u 9 line; then
            kill -0 "$pid" 2>/dev/null || return 1
        elif [[ "$initialized" == 0 ]] && printf '%s\n' "$line" | gpteasy__probe_reply_is_valid 1; then
            initialized=1
            printf '%s\n' '{"method":"initialized"}' '{"id":2,"method":"model/list","params":{"includeHidden":true,"limit":100}}' >&8
        elif [[ "$initialized" == 1 ]] && printf '%s\n' "$line" | gpteasy__probe_reply_is_valid 2; then
            exec 9>&-
            return 0
        fi
        attempts=$((attempts + 1))
    done
    exec 9>&-
    return 1
)


gpteasy__toml_string() {
    printf '"%s"\n' "$(printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g')"
}
gpteasy__tmp_root=/tmp
gpteasy__check_codex_compatibility || exit 48
printf '%s\n' '__GPTEASY_CATALOG_COMPATIBLE__'
