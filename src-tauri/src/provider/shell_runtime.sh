
gpteasy__start_marker='# >>> GPTEasy managed provider >>>'
gpteasy__end_marker='# <<< GPTEasy managed provider <<<'
gpteasy__provider_id_prefix='# GPTEasy provider-id:'
gpteasy__schema_prefix='# GPTEasy schema-version:'
gpteasy__source_id_prefix='# GPTEasy source-id:'
gpteasy__credential_file_prefix='# GPTEasy credential-file:'
gpteasy__catalog_artifact_prefix='# GPTEasy model-catalog-artifact:'
gpteasy__catalog_sha256_prefix='# GPTEasy model-catalog-sha256:'
gpteasy__catalog_fingerprint_prefix='# GPTEasy model-catalog-provider-fingerprint:'
gpteasy__catalog_protocol_prefix='# GPTEasy model-catalog-protocol:'
gpteasy__catalog_policy_prefix='# GPTEasy model-catalog-policy:'
gpteasy__catalog_file_prefix='# GPTEasy model-catalog-file:'

# Read the complete v2 catalog row so metadata and credentials cannot be
# truncated by the legacy six-column format.
gpteasy__provider_id() {
    local expected_index=$1 index=1 provider_id
    while IFS="$(printf '\t')" read -r provider_id _; do
        [[ -n "$provider_id" && "$provider_id" != \#* ]] || continue
        if [[ "$index" == "$expected_index" ]]; then
            printf '%s\n' "$provider_id"
            return 0
        fi
        index=$((index + 1))
    done < <(gpteasy__provider_catalog)
    return 1
}

gpteasy__provider_value() {
    local expected_id=$1 field=$2 provider_id name base_url model api_key reasoning_effort artifact sha256 fingerprint
    while IFS="$(printf '\t')" read -r provider_id name base_url model api_key reasoning_effort artifact sha256 fingerprint; do
        [[ -n "$provider_id" && "$provider_id" != \#* && "$provider_id" == "$expected_id" ]] || continue
        case "$field" in
            name) printf '%s\n' "$name" ;;
            model) printf '%s\n' "$model" ;;
            base_url) printf '%s\n' "$base_url" ;;
            reasoning_effort) [[ "$reasoning_effort" != '-' ]] && printf '%s\n' "$reasoning_effort" ;;
            credential) printf '%s' "$api_key" ;;
            artifact) printf '%s\n' "$artifact" ;;
            catalog_sha256) printf '%s\n' "$sha256" ;;
            verification_fingerprint) printf '%s\n' "$fingerprint" ;;
            *) return 1 ;;
        esac
        return 0
    done < <(gpteasy__provider_catalog)
    return 1
}

gpteasy__provider_name() {
    gpteasy__provider_value "$1" name
}

gpteasy__provider_model() {
    gpteasy__provider_value "$1" model
}

gpteasy__provider_base_url() {
    gpteasy__provider_value "$1" base_url
}

gpteasy__provider_reasoning_effort() {
    gpteasy__provider_value "$1" reasoning_effort
}

gpteasy__provider_catalog_artifact() {
    gpteasy__provider_value "$1" artifact
}

gpteasy__provider_catalog_sha256() {
    gpteasy__provider_value "$1" catalog_sha256
}

gpteasy__provider_verification_fingerprint() {
    gpteasy__provider_value "$1" verification_fingerprint
}

gpteasy__print_credential() {
    gpteasy__provider_value "$1" credential
}
{{GPTEASY_SHELL_SETUP}}

gpteasy__help() {
    cat <<'GPTEASY_HELP'
用法：
  gpteasy                 选择并切换供应商
  gpteasy help            显示帮助（等同于 --help、-h）
  gpteasy current         查看当前供应商
  gpteasy restore         恢复最近一次 shell 切换
  gpteasy info            查看目标环境和快照信息
  gpteasy unlock          处理失效的 shell 锁
GPTEASY_HELP
}

gpteasy__require_snapshot_safe() {
    local links kind
    if [[ -L "$gpteasy__script_path" ]]; then
        printf '%s\n' '导出文件不能是符号链接；除帮助外已拒绝执行。' >&2
        return 1
    fi
    if ! read -r links kind < <(stat -c '%h %F' -- "$gpteasy__script_path" 2>/dev/null); then
        printf '%s\n' '无法确认导出文件身份；除帮助外已拒绝执行。' >&2
        return 1
    fi
    if [[ ! -r "$gpteasy__script_path" || "$links" != 1 || "$kind" != 'regular file' ]]; then
        printf '%s\n' '导出文件必须是当前操作用户可读取的单链接普通文件；除帮助外已拒绝执行。' >&2
        return 1
    fi
}

gpteasy__config_path() {
    printf '%s\n' "${CODEX_HOME:-"$HOME/.codex"}/config.toml"
}

gpteasy__matches() {
    printf '%s\n' "$1" | awk '
        BEGIN { pattern = ARGV[1]; ARGV[1] = "" }
        NR > 1 { invalid = 1 }
        $0 ~ pattern { matched = 1 }
        END { exit !invalid && matched ? 0 : 1 }
    ' "$2"
}

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

gpteasy__refresh_pending() {
    printf '%s\n' '配置已保存，但 CLI/共享后台服务可能仍使用旧配置。'
    printf '人工刷新目标：uid=%s，CODEX_HOME=%s。请在该用户和 home 下，按所安装原生 CLI 的官方管理说明刷新。\n' "$(gpteasy__current_uid)" "${CODEX_HOME:-"$HOME/.codex"}"
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

gpteasy__current_uid() {
    id -u
}

gpteasy__owner_label() {
    local uid=$1 name
    name=$(id -nu "$uid" 2>/dev/null || true)
    if [[ -n "$name" ]]; then
        printf '%s（uid=%s）' "$name" "$uid"
    else
        printf 'uid=%s' "$uid"
    fi
}

gpteasy__directory_is_owned() {
    local directory=$1 owner kind
    [[ ! -L "$directory" ]] || return 1
    read -r owner kind < <(stat -c '%u %F' -- "$directory" 2>/dev/null) || return 1
    [[ "$owner" == "$(gpteasy__current_uid)" && "$kind" == 'directory' && -r "$directory" && -w "$directory" && -x "$directory" ]]
}

gpteasy__warn_if_permissions_are_broad() {
    local target_path=$1 mode
    mode=$(stat -c '%a' -- "$target_path" 2>/dev/null) || return
    if (( (8#$mode & 8#77) != 0 )); then
        gpteasy__permission_warning=1
    fi
}

gpteasy__report_identity_mismatch() {
    local target_path=$1 owner sudo_user=${SUDO_USER:-}
    owner=$(stat -c '%u' -- "$target_path" 2>/dev/null) || return
    printf '目标环境身份不匹配：当前用户 %s（uid=%s）不能管理 %s（所有者 %s）。请以该所有者身份执行；例如 sudo -u <用户> -H。' \
        "$(id -un)" "$(gpteasy__current_uid)" "$target_path" "$(gpteasy__owner_label "$owner")" >&2
    if [[ -n "$sudo_user" ]]; then
        printf ' 检测到 SUDO_USER=%s；sudo 可能正在为错误的用户环境写入。' "$sudo_user" >&2
    fi
    printf '\n' >&2
}

gpteasy__require_codex_home() {
    local codex_home=${CODEX_HOME:-"$HOME/.codex"} parent owner
    case "$codex_home" in /*) ;; *) printf '%s\n' 'CODEX_HOME 必须为 Linux 绝对路径。' >&2; return 1 ;; esac
    if [[ "$codex_home" == *$'\n'* ]] || printf '%s' "$codex_home" | LC_ALL=C grep -q '[[:cntrl:]]'; then
        printf '%s\n' 'CODEX_HOME 包含不支持的控制字符。' >&2
        return 1
    fi
    gpteasy__codex_home=$codex_home
    if [[ -L "$codex_home" ]]; then
        printf 'CODEX_HOME 不能是符号链接：%s\n' "$codex_home" >&2
        return 1
    fi
    if [[ -e "$codex_home" ]]; then
        if [[ ! -d "$codex_home" ]]; then
            printf '目标 Codex 环境不是普通目录：%s\n' "$codex_home" >&2
            return 1
        fi
        owner=$(stat -c '%u' -- "$codex_home" 2>/dev/null) || return 1
        if [[ "$owner" != "$(gpteasy__current_uid)" ]]; then
            gpteasy__report_identity_mismatch "$codex_home"
            return 1
        fi
        if [[ ! -r "$codex_home" || ! -w "$codex_home" || ! -x "$codex_home" ]]; then
            printf '当前用户 %s（uid=%s）无法访问或写入目标 Codex 环境 %s（所有者 %s）。请切换到其所有者或修复权限后重试。\n' \
                "$(id -un)" "$(gpteasy__current_uid)" "$codex_home" "$(gpteasy__owner_label "$owner")" >&2
            return 1
        fi
        gpteasy__warn_if_permissions_are_broad "$codex_home"
        return
    fi
    parent=$codex_home
    while [[ ! -e "$parent" && ! -L "$parent" ]]; do
        parent=${parent%/*}
        [[ -n "$parent" ]] || parent=/
    done
    if [[ -L "$parent" || ! -d "$parent" || ! -w "$parent" || ! -x "$parent" ]]; then
        owner=$(stat -c '%u' -- "$parent" 2>/dev/null || printf '%s' unknown)
        printf '无法在目标路径创建 Codex 环境：最近存在的父目录 %s 对当前用户 %s（uid=%s）不可写或不可搜索；所有者 %s。\n' \
            "$parent" "$(id -un)" "$(gpteasy__current_uid)" "$(gpteasy__owner_label "$owner")" >&2
        return 1
    fi
    if [[ "$(gpteasy__current_uid)" == 0 ]]; then
        owner=$(stat -c '%u' -- "$parent" 2>/dev/null) || return 1
        if [[ "$owner" != 0 ]]; then
            gpteasy__report_identity_mismatch "$parent"
            return 1
        fi
    fi
}

gpteasy__ensure_private_dir() {
    local directory=$1
    if [[ ! -e "$directory" && ! -L "$directory" ]]; then
        mkdir -m 700 -- "$directory" || return
    fi
    if ! gpteasy__directory_is_owned "$directory" 1; then
        printf 'Linux 私有状态目录不属于当前用户或不可安全访问：%s\n' "$directory" >&2
        return 1
    fi
    gpteasy__warn_if_permissions_are_broad "$directory"
}

gpteasy__prepare_private_state() {
    local codex_home=${CODEX_HOME:-"$HOME/.codex"}
    if [[ ! -e "$codex_home" && ! -L "$codex_home" ]]; then
        mkdir -p -m 700 -- "$codex_home" || return
    fi
    if ! gpteasy__directory_is_owned "$codex_home"; then
        printf '目标 Codex 环境在预检后发生变化，已停止写入：%s\n' "$codex_home" >&2
        return 1
    fi
    gpteasy__state_root="$codex_home/.gpteasy-shell"
    gpteasy__credentials_root="$gpteasy__state_root/credentials"
    gpteasy__model_catalog_root="$gpteasy__state_root/model-catalogs"
    gpteasy__restore_root="$gpteasy__state_root/shell-restore"
    gpteasy__tmp_root="$gpteasy__state_root/tmp"
    gpteasy__lock_root="$gpteasy__state_root/lock"
    gpteasy__ensure_private_dir "$gpteasy__state_root" || return
    gpteasy__ensure_private_dir "$gpteasy__credentials_root" || return
    gpteasy__ensure_private_dir "$gpteasy__model_catalog_root" || return
    gpteasy__ensure_private_dir "$gpteasy__restore_root" || return
    gpteasy__ensure_private_dir "$gpteasy__tmp_root" || return
    gpteasy__ensure_private_dir "$gpteasy__lock_root" || return
}

gpteasy__require_existing_private_state_safe() {
    local codex_home=${CODEX_HOME:-"$HOME/.codex"} root item
    root="$codex_home/.gpteasy-shell"
    if [[ ! -e "$root" && ! -L "$root" ]]; then
        return
    fi
    if [[ -L "$root" || ! -d "$root" ]] || find "$root" -type l -print -quit 2>/dev/null | grep -q .; then
        printf '%s\n' 'Linux 私有状态包含不安全的符号链接。' >&2
        return 1
    fi
    if find "$root" ! -type d ! -type f ! -type l -print -quit 2>/dev/null | grep -q .; then
        printf '%s\n' 'Linux 私有状态包含不支持的文件类型。' >&2
        return 1
    fi
    while IFS= read -r -d '' item; do
        if ! gpteasy__directory_is_owned "$item"; then
            printf 'Linux 私有状态目录不属于当前用户或不可访问：%s\n' "$item" >&2
            return 1
        fi
        gpteasy__warn_if_permissions_are_broad "$item"
    done < <(find "$root" -type d -print0)
    while IFS= read -r -d '' item; do
        if ! gpteasy__private_file_is_safe "$item"; then
            printf 'Linux 私有状态文件不属于当前用户、不可访问或链接数不安全：%s\n' "$item" >&2
            return 1
        fi
        gpteasy__warn_if_permissions_are_broad "$item"
    done < <(find "$root" -type f -print0)
}

gpteasy__acquire_lock() {
    local operation=$1 active="$gpteasy__lock_root/active" process_id start owner held_operation
    if ! mkdir -m 700 -- "$active" 2>/dev/null; then
        owner=$(gpteasy__lock_value "$active/owner" owner 2>/dev/null || printf '%s' unknown)
        held_operation=$(gpteasy__lock_value "$active/owner" operation 2>/dev/null || printf '%s' unknown)
        case "$owner" in shell | desktop) ;; *) owner=unknown ;; esac
        gpteasy__matches "$held_operation" '^[a-z_]+$' || held_operation=unknown
        printf '另一个 GPTEasy 配置操作正在进行（owner=%s，operation=%s），请稍后重试。\n' "$owner" "$held_operation" >&2
        return 1
    fi
{{GPTEASY_PROCESS_ID}}
    start=$(awk '{print $22}' "/proc/$process_id/stat" 2>/dev/null) || {
        rmdir -- "$active" 2>/dev/null || true
        return 1
    }
    gpteasy__lock_token="$(date -u +%s%N)-$process_id-${RANDOM:-0}"
    if ! {
        printf 'owner=shell\n'
        printf 'token=%s\n' "$gpteasy__lock_token"
        printf 'pid=%s\n' "$process_id"
        printf 'process_start=%s\n' "$start"
        printf 'operation=%s\n' "$operation"
    } >"$active/owner"; then
        rm -f -- "$active/owner"
        rmdir -- "$active" 2>/dev/null || true
        return 1
    fi
    chmod 600 "$active/owner" || return
    gpteasy__active_lock=$active
}

gpteasy__lock_value() {
    local file=$1 key=$2
    awk -F= -v key="$key" '
        $1 == key { print substr($0, length(key) + 2); found += 1 }
        END { if (found != 1) exit 1 }
    ' "$file"
}

gpteasy__release_lock() {
    local token
    [[ -n "${gpteasy__active_lock:-}" && -f "$gpteasy__active_lock/owner" ]] || return 0
    token=$(awk -F= '$1 == "token" { print substr($0, 7); found += 1 } END { if (found != 1) exit 1 }' "$gpteasy__active_lock/owner" 2>/dev/null) || return 1
    [[ "$token" == "${gpteasy__lock_token:-}" ]] || return 1
    rm -f -- "$gpteasy__active_lock/owner" || return
    rmdir -- "$gpteasy__active_lock" || return
    gpteasy__active_lock=
    gpteasy__lock_token=
}

gpteasy__file_hash() {
    if [[ -f "$1" ]]; then
        sha256sum <"$1" | awk '{print $1}'
    else
        printf '%s\n' 'missing'
    fi
}

gpteasy__resolve_config_target() {
    local entry links kind parent owner
    entry=$(gpteasy__config_path) || return
    gpteasy__config_entry=$entry
    if [[ -L "$entry" ]]; then
        gpteasy__config_kind=symlink
        gpteasy__config_link_value=$(readlink -- "$entry") || return
        gpteasy__config_target=$(readlink -f -- "$entry") || {
            printf '%s\n' 'config.toml 符号链接目标不可用。' >&2
            return 1
        }
        gpteasy__config_entry_signature=$(stat -c '%d:%i:%u:%a:%h:%F' -- "$entry") || return
        read -r links kind < <(stat -Lc '%h %F' -- "$gpteasy__config_target") || return
        owner=$(stat -Lc '%u' -- "$gpteasy__config_target") || return
        if [[ "$owner" != "$(gpteasy__current_uid)" ]]; then
            gpteasy__report_identity_mismatch "$gpteasy__config_target"
            return 1
        fi
        if [[ ! -r "$gpteasy__config_target" || ! -w "$gpteasy__config_target" || "$links" != 1 || "$kind" != 'regular file' ]]; then
            printf 'config.toml 符号链接最终目标必须是当前用户可读写的单链接普通文件：%s（当前用户 %s，所有者 %s）。\n' \
                "$gpteasy__config_target" "$(id -un)" "$(gpteasy__owner_label "$owner")" >&2
            return 1
        fi
        gpteasy__warn_if_permissions_are_broad "$gpteasy__config_target"
    elif [[ -e "$entry" ]]; then
        gpteasy__config_kind=regular
        gpteasy__config_target=$entry
        gpteasy__config_link_value=
        gpteasy__config_entry_signature=$(stat -c '%d:%i:%u:%a:%h:%F' -- "$entry") || return
        read -r links kind < <(stat -c '%h %F' -- "$entry") || return
        owner=$(stat -c '%u' -- "$entry") || return
        if [[ "$owner" != "$(gpteasy__current_uid)" ]]; then
            gpteasy__report_identity_mismatch "$entry"
            return 1
        fi
        if [[ ! -r "$entry" || ! -w "$entry" || "$links" != 1 || "$kind" != 'regular file' ]]; then
            printf 'config.toml 必须是当前用户可读写的单链接普通文件：%s（当前用户 %s，所有者 %s）。\n' \
                "$entry" "$(id -un)" "$(gpteasy__owner_label "$owner")" >&2
            return 1
        fi
        gpteasy__warn_if_permissions_are_broad "$entry"
    else
        gpteasy__config_kind=missing
        gpteasy__config_target=$entry
        gpteasy__config_link_value=
        gpteasy__config_entry_signature=missing
    fi
    parent=${gpteasy__config_target%/*}
    if [[ -e "$parent" || -L "$parent" ]]; then
        if [[ -L "$parent" || ! -d "$parent" || ! -r "$parent" || ! -w "$parent" || ! -x "$parent" ]]; then
            printf 'config.toml 最终目标目录对当前用户不可读写或不可搜索：%s\n' "$parent" >&2
            return 1
        fi
    elif [[ "$parent" != "${CODEX_HOME:-"$HOME/.codex"}" ]]; then
        printf 'config.toml 的父目录不存在且无法安全创建：%s\n' "$parent" >&2
        return 1
    fi
    if [[ -e "$gpteasy__config_target" ]]; then
        gpteasy__config_target_signature=$(stat -Lc '%d:%i:%u:%a:%h:%F' -- "$gpteasy__config_target") || return
    else
        gpteasy__config_target_signature=missing
    fi
    gpteasy__config_original_hash=$(gpteasy__file_hash "$gpteasy__config_target") || return
}

gpteasy__config_target_unchanged() {
    local signature target target_signature links kind
    case "$gpteasy__config_kind" in
        missing)
            [[ ! -e "$gpteasy__config_entry" && ! -L "$gpteasy__config_entry" ]] || return 1
            ;;
        regular)
            [[ ! -L "$gpteasy__config_entry" && -f "$gpteasy__config_entry" ]] || return 1
            signature=$(stat -c '%d:%i:%u:%a:%h:%F' -- "$gpteasy__config_entry") || return
            [[ "$signature" == "$gpteasy__config_entry_signature" ]] || return 1
            ;;
        symlink)
            [[ -L "$gpteasy__config_entry" ]] || return 1
            signature=$(stat -c '%d:%i:%u:%a:%h:%F' -- "$gpteasy__config_entry") || return
            [[ "$signature" == "$gpteasy__config_entry_signature" ]] || return 1
            [[ "$(readlink -- "$gpteasy__config_entry")" == "$gpteasy__config_link_value" ]] || return 1
            target=$(readlink -f -- "$gpteasy__config_entry") || return
            [[ "$target" == "$gpteasy__config_target" ]] || return 1
            ;;
    esac
    if [[ -e "$gpteasy__config_target" ]]; then
        target_signature=$(stat -Lc '%d:%i:%u:%a:%h:%F' -- "$gpteasy__config_target") || return
        [[ "$target_signature" == "$gpteasy__config_target_signature" ]] || return 1
        read -r links kind < <(stat -Lc '%h %F' -- "$gpteasy__config_target") || return
        [[ -r "$gpteasy__config_target" && -w "$gpteasy__config_target" && "$links" == 1 && "$kind" == 'regular file' ]] || return 1
    fi
    [[ "$(gpteasy__file_hash "$gpteasy__config_target")" == "$gpteasy__config_original_hash" ]]
}

gpteasy__marker_info() {
    local config=$1
    if [[ ! -f "$config" ]]; then
        printf '%s\n' '0 0 0 0'
        return
    fi
    awk -v start="$gpteasy__start_marker" -v end="$gpteasy__end_marker" '
        { line = $0; sub(/\r$/, "", line) }
        line == start { starts += 1; if (start_line == 0) start_line = NR }
        line == end { ends += 1; if (end_line == 0) end_line = NR }
        END { print starts + 0, ends + 0, start_line + 0, end_line + 0 }
    ' "$config"
}

gpteasy__managed_metadata() {
    local config=$1 prefix=$2
    awk -v start="$gpteasy__start_marker" -v end="$gpteasy__end_marker" -v prefix="$prefix" '
        { line = $0; sub(/\r$/, "", line) }
        line == start { inside = 1; next }
        inside && line == end { inside = 0; next }
        inside && index(line, prefix) == 1 {
            sub("^" prefix "[[:space:]]*", "", line)
            print line
            found += 1
        }
        END { if (found != 1) exit 2 }
    ' "$config"
}

gpteasy__managed_metadata_count() {
    local config=$1 prefix=$2
    awk -v start="$gpteasy__start_marker" -v end="$gpteasy__end_marker" -v prefix="$prefix" '
        { line = $0; sub(/\r$/, "", line) }
        line == start { inside = 1; next }
        inside && line == end { inside = 0; next }
        inside && index(line, prefix) == 1 { found += 1 }
        END { print found + 0 }
    ' "$config"
}

gpteasy__schema_v1_is_valid() {
    local config=$1 provider_id source relative auth_args expected_auth_args
    provider_id=$(gpteasy__managed_metadata "$config" "$gpteasy__provider_id_prefix" 2>/dev/null) || return 1
    source=$(gpteasy__managed_metadata "$config" "$gpteasy__source_id_prefix" 2>/dev/null) || return 1
    relative=$(gpteasy__managed_metadata "$config" "$gpteasy__credential_file_prefix" 2>/dev/null) || return 1
    gpteasy__matches "$provider_id" '^[[:xdigit:]]{8}-[[:xdigit:]]{4}-[[:xdigit:]]{4}-[[:xdigit:]]{4}-[[:xdigit:]]{12}$' || return 1
    gpteasy__matches "$source" '^[[:alnum:]][[:alnum:].:_-]*$' || return 1
    [[ "$relative" == ".gpteasy-shell/credentials/$source/$provider_id.token" ]] || return 1
    [[ "$relative" != *'..'* && "$relative" != *'//'* ]] || return 1
    awk -v start="$gpteasy__start_marker" -v end="$gpteasy__end_marker" \
        -v schema="$gpteasy__schema_prefix" -v provider="$gpteasy__provider_id_prefix" \
        -v source="$gpteasy__source_id_prefix" -v credential="$gpteasy__credential_file_prefix" '
        { line = $0; sub(/\r$/, "", line) }
        line == start { inside = 1; next }
        inside && line == end { inside = 0; next }
        !inside { next }
        index(line, schema) == 1 { schema_count += 1; next }
        index(line, provider) == 1 { provider_count += 1; next }
        index(line, source) == 1 { source_count += 1; next }
        index(line, credential) == 1 { credential_count += 1; next }
        index(line, "model = ") == 1 { model_count += 1; next }
        index(line, "model_reasoning_effort = ") == 1 {
            value = line
            sub(/^model_reasoning_effort = "/, "", value)
            sub(/"$/, "", value)
            if (value !~ /^(low|medium|high|xhigh)$/) { invalid = 1 }
            reasoning_count += 1
            next
        }
        line == "model_provider = \"gpteasy\"" { model_provider_count += 1; next }
        index(line, "model_providers.gpteasy.name = ") == 1 { name_count += 1; next }
        index(line, "model_providers.gpteasy.base_url = ") == 1 { base_url_count += 1; next }
        line == "model_providers.gpteasy.wire_api = \"responses\"" { wire_count += 1; next }
        line == "model_providers.gpteasy.supports_websockets = false" { websocket_count += 1; next }
        line == "model_providers.gpteasy.auth.command = \"sh\"" { auth_command_count += 1; next }
        index(line, "model_providers.gpteasy.auth.args = ") == 1 { auth_args_count += 1; next }
        { invalid = 1 }
        END {
            valid = !invalid && schema_count == 1 && provider_count == 1 && source_count == 1 &&
                credential_count == 1 && model_count == 1 && reasoning_count <= 1 &&
                model_provider_count == 1 &&
                name_count == 1 && base_url_count == 1 && wire_count == 1 && websocket_count == 1 &&
                auth_command_count == 1 && auth_args_count == 1
            exit valid ? 0 : 1
        }
    ' "$config" || return 1
    auth_args=$(gpteasy__managed_line "$config" 'model_providers.gpteasy.auth.args = ' 2>/dev/null) || return 1
    expected_auth_args="model_providers.gpteasy.auth.args = [\"-c\", 'cat -- \"\${CODEX_HOME:-\$HOME/.codex}/$relative\"']"
    [[ "$auth_args" == "$expected_auth_args" ]]
}

gpteasy__catalog_path_for() {
    local source=$1 artifact=$2
    gpteasy__provider_id_is_safe "$source" && gpteasy__provider_id_is_safe "$artifact" || return 1
    printf '%s\n' "${CODEX_HOME:-"$HOME/.codex"}/.gpteasy-shell/model-catalogs/$source/$artifact.json"
}

gpteasy__catalog_binding_is_valid() {
    local provider_id=$1 source=$2 artifact=$3 expected_hash=$4 expected_fingerprint=$5
    local binding binding_provider binding_model binding_artifact binding_hash binding_fingerprint binding_source binding_protocol binding_policy expected_model actual_fingerprint base_url credential
    [[ "$source" == "$gpteasy__export_id" ]] || return 1
    gpteasy__catalog_path_for "$source" "$artifact" >/dev/null || return 1
    gpteasy__matches "$expected_hash" '^[[:xdigit:]]{64}$' || return 1
    expected_model=$(gpteasy__provider_model "$provider_id") || return 1
    base_url=$(gpteasy__provider_base_url "$provider_id") || return 1
    credential=$(gpteasy__print_credential "$provider_id") || return 1
    actual_fingerprint=$(printf 'gpteasy-provider-combination-v1\0%s\0%s\0%s' "$base_url" "$expected_model" "$credential" | sha256sum | awk '{print $1}') || return 1
    [[ "$actual_fingerprint" == "$expected_fingerprint" ]] || return 1
    binding=$(gpteasy__provider_catalog_binding "$provider_id") || return 1
    IFS="$(printf '\t')" read -r binding_provider binding_model binding_artifact binding_hash binding_fingerprint binding_source binding_protocol binding_policy <<<"$binding"
    [[ "$binding_provider" == "$provider_id" && "$binding_model" == "$expected_model" &&
        "$binding_artifact" == "$artifact" && "$binding_hash" == "$expected_hash" && "$binding_fingerprint" == "$expected_fingerprint" &&
        "$binding_source" == "$source" && "$binding_protocol" == "$gpteasy__catalog_protocol" && "$binding_policy" == "$gpteasy__catalog_policy" ]]
}

gpteasy__catalog_directory_is_private() {
    local directory=$1 mode
    gpteasy__directory_is_owned "$directory" || return 1
    mode=$(stat -c '%a' -- "$directory") || return 1
    (( (8#$mode & 8#77) == 0 ))
}

gpteasy__catalog_file_is_valid() {
    # Committed artifacts are checked independently of the currently sourced export.
    # Restore may reference another export or a supplier no longer in this snapshot.
    local provider_id=$1 source=$2 artifact=$3 expected_hash=$4 expected_fingerprint=$5 target_path mode directory
    gpteasy__provider_id_is_safe "$provider_id" || return 1
    gpteasy__matches "$expected_hash" '^[[:xdigit:]]{64}$' || return 1
    gpteasy__matches "$expected_fingerprint" '^[[:xdigit:]]{64}$' || return 1
    target_path=$(gpteasy__catalog_path_for "$source" "$artifact") || return 1
    directory=${target_path%/*}
    gpteasy__catalog_directory_is_private "${directory%/*/*}" &&
        gpteasy__catalog_directory_is_private "${directory%/*}" &&
        gpteasy__catalog_directory_is_private "$directory" || return 1
    gpteasy__private_file_is_safe "$target_path" || return 1
    mode=$(stat -c '%a' -- "$target_path" 2>/dev/null) || return 1
    (( (8#$mode & 8#77) == 0 )) || return 1
    [[ "$(gpteasy__file_hash "$target_path")" == "$expected_hash" ]]
}

gpteasy__install_model_catalog() {
    local provider_id=$1 source artifact expected_hash fingerprint payload target_path directory temporary
    source="$gpteasy__export_id"
    artifact=$(gpteasy__provider_catalog_artifact "$provider_id") || return 1
    expected_hash=$(gpteasy__provider_catalog_sha256 "$provider_id") || return 1
    fingerprint=$(gpteasy__provider_verification_fingerprint "$provider_id") || return 1
    gpteasy__catalog_binding_is_valid "$provider_id" "$source" "$artifact" "$expected_hash" "$fingerprint" || {
        printf '%s\n' '模型目录载荷与供应商快照绑定不一致，已停止写入。' >&2
        return 1
    }
    payload=$(gpteasy__provider_catalog_payload "$provider_id") || return 1
    [[ "$(printf '%s' "$payload" | sha256sum | awk '{print $1}')" == "$expected_hash" ]] || {
        printf '%s\n' '模型目录载荷摘要校验失败，已停止写入。' >&2
        return 1
    }
    directory="$gpteasy__model_catalog_root/$source"
    gpteasy__ensure_private_dir "$directory" || return 1
    gpteasy__catalog_directory_is_private "${directory%/*/*}" &&
        gpteasy__catalog_directory_is_private "${directory%/*}" &&
        gpteasy__catalog_directory_is_private "$directory" || return 1
    target_path="$directory/$artifact.json"
    gpteasy__model_catalog_created=0
    if [[ -e "$target_path" || -L "$target_path" ]]; then
        if ! gpteasy__catalog_file_is_valid "$provider_id" "$source" "$artifact" "$expected_hash" "$fingerprint" ||
            ! cmp -s -- "$target_path" <(printf '%s' "$payload"); then
            printf '%s\n' '已有 Linux 模型目录工件不安全或内容不一致。' >&2
            return 1
        fi
    else
        temporary=$(mktemp "$directory/.model-catalog.XXXXXX") || return 1
        if ! printf '%s' "$payload" >"$temporary" || ! chmod 600 "$temporary" || ! sync -f "$temporary"; then
            rm -f -- "$temporary"
            return 1
        fi
        mv -n -- "$temporary" "$target_path" || {
            rm -f -- "$temporary"
            return 1
        }
        if [[ -e "$temporary" ]]; then
            rm -f -- "$temporary"
            return 1
        fi
        gpteasy__model_catalog_created=1
        sync -f "$directory" || return 1
    fi
    gpteasy__catalog_file_is_valid "$provider_id" "$source" "$artifact" "$expected_hash" "$fingerprint" || return 1
    gpteasy__model_catalog_path="$target_path"
    gpteasy__evidence catalog_commit ready
}

gpteasy__schema_v2_is_valid() {
    local config=$1 provider_id source relative artifact expected_hash fingerprint catalog_path expected_catalog_line auth_args expected_auth_args
    provider_id=$(gpteasy__managed_metadata "$config" "$gpteasy__provider_id_prefix" 2>/dev/null) || return 1
    source=$(gpteasy__managed_metadata "$config" "$gpteasy__source_id_prefix" 2>/dev/null) || return 1
    relative=$(gpteasy__managed_metadata "$config" "$gpteasy__credential_file_prefix" 2>/dev/null) || return 1
    artifact=$(gpteasy__managed_metadata "$config" "$gpteasy__catalog_artifact_prefix" 2>/dev/null) || return 1
    expected_hash=$(gpteasy__managed_metadata "$config" "$gpteasy__catalog_sha256_prefix" 2>/dev/null) || return 1
    fingerprint=$(gpteasy__managed_metadata "$config" "$gpteasy__catalog_fingerprint_prefix" 2>/dev/null) || return 1
    [[ "$(gpteasy__managed_metadata "$config" "$gpteasy__catalog_protocol_prefix" 2>/dev/null)" == "$gpteasy__catalog_protocol" &&
        "$(gpteasy__managed_metadata "$config" "$gpteasy__catalog_policy_prefix" 2>/dev/null)" == "$gpteasy__catalog_policy" &&
        "$(gpteasy__managed_metadata "$config" "$gpteasy__catalog_file_prefix" 2>/dev/null)" == ".gpteasy-shell/model-catalogs/$source/$artifact.json" ]] || return 1
    [[ "$(awk '/^[[:space:]]*model_catalog_json[[:space:]]*=/ {n++} END {print n+0}' "$config")" == 1 ]] || return 1
    catalog_path=$(gpteasy__managed_line "$config" 'model_catalog_json = ' 2>/dev/null) || return 1
    expected_catalog_line="model_catalog_json = $(gpteasy__toml_string "$(gpteasy__catalog_path_for "$source" "$artifact")")"
    [[ "$catalog_path" == "$expected_catalog_line" ]] || return 1
    gpteasy__matches "$provider_id" '^[[:xdigit:]]{8}-[[:xdigit:]]{4}-[[:xdigit:]]{4}-[[:xdigit:]]{4}-[[:xdigit:]]{12}$' || return 1
    [[ "$relative" == ".gpteasy-shell/credentials/$source/$provider_id.token" ]] || return 1
    [[ "$relative" != *'..'* && "$relative" != *'//'* ]] || return 1
    gpteasy__catalog_file_is_valid "$provider_id" "$source" "$artifact" "$expected_hash" "$fingerprint" || return 1
    awk -v start="$gpteasy__start_marker" -v end="$gpteasy__end_marker" \
        -v schema="$gpteasy__schema_prefix" -v provider="$gpteasy__provider_id_prefix" \
        -v source="$gpteasy__source_id_prefix" -v credential="$gpteasy__credential_file_prefix" \
        -v artifact="$gpteasy__catalog_artifact_prefix" -v hash="$gpteasy__catalog_sha256_prefix" \
        -v fingerprint="$gpteasy__catalog_fingerprint_prefix" \
        -v protocol="$gpteasy__catalog_protocol_prefix" -v policy="$gpteasy__catalog_policy_prefix" \
        -v catalog_file="$gpteasy__catalog_file_prefix" '
        { line = $0; sub(/\r$/, "", line) }
        line == start { inside = 1; next }
        inside && line == end { inside = 0; next }
        !inside { next }
        index(line, schema) == 1 { schema_count += 1; next }
        index(line, provider) == 1 { provider_count += 1; next }
        index(line, source) == 1 { source_count += 1; next }
        index(line, credential) == 1 { credential_count += 1; next }
        index(line, artifact) == 1 { artifact_count += 1; next }
        index(line, hash) == 1 { hash_count += 1; next }
        index(line, fingerprint) == 1 { fingerprint_count += 1; next }
        index(line, protocol) == 1 { protocol_count += 1; next }
        index(line, policy) == 1 { policy_count += 1; next }
        index(line, catalog_file) == 1 { catalog_file_count += 1; next }
        index(line, "model = ") == 1 { model_count += 1; next }
        index(line, "model_reasoning_effort = ") == 1 {
            value = line
            sub(/^model_reasoning_effort = "/, "", value)
            sub(/"$/, "", value)
            if (value !~ /^(low|medium|high|xhigh)$/) { invalid = 1 }
            reasoning_count += 1
            next
        }
        line == "model_provider = \"gpteasy\"" { model_provider_count += 1; next }
        index(line, "model_providers.gpteasy.name = ") == 1 { name_count += 1; next }
        index(line, "model_providers.gpteasy.base_url = ") == 1 { base_url_count += 1; next }
        line == "model_providers.gpteasy.wire_api = \"responses\"" { wire_count += 1; next }
        line == "model_providers.gpteasy.supports_websockets = false" { websocket_count += 1; next }
        line == "model_providers.gpteasy.auth.command = \"sh\"" { auth_command_count += 1; next }
        index(line, "model_providers.gpteasy.auth.args = ") == 1 { auth_args_count += 1; next }
        index(line, "model_catalog_json = ") == 1 { catalog_count += 1; next }
        { invalid = 1 }
        END {
            valid = !invalid && schema_count == 1 && provider_count == 1 && source_count == 1 &&
                credential_count == 1 && artifact_count == 1 && hash_count == 1 && fingerprint_count == 1 &&
                protocol_count == 1 && policy_count == 1 && catalog_file_count == 1 &&
                model_count == 1 && reasoning_count <= 1 && model_provider_count == 1 &&
                name_count == 1 && base_url_count == 1 && wire_count == 1 && websocket_count == 1 &&
                auth_command_count == 1 && auth_args_count == 1 && catalog_count == 1
            exit valid ? 0 : 1
        }
    ' "$config" || return 1
    auth_args=$(gpteasy__managed_line "$config" 'model_providers.gpteasy.auth.args = ' 2>/dev/null) || return 1
    expected_auth_args="model_providers.gpteasy.auth.args = [\"-c\", 'cat -- \"\${CODEX_HOME:-\$HOME/.codex}/$relative\"']"
    [[ "$auth_args" == "$expected_auth_args" ]]
}

gpteasy__current_provider_id() {
    local config marker_info starts ends start_line end_line
    config=$(gpteasy__config_path) || return
    [[ -f "$config" ]] || return 1
    marker_info=$(gpteasy__marker_info "$config") || return
    read -r starts ends start_line end_line <<<"$marker_info"
    [[ "$starts" -eq 1 && "$ends" -eq 1 && "$start_line" -lt "$end_line" ]] || return 2
    gpteasy__managed_metadata "$config" "$gpteasy__provider_id_prefix"
}

gpteasy__managed_line() {
    local config=$1 prefix=$2
    awk -v start="$gpteasy__start_marker" -v end="$gpteasy__end_marker" -v prefix="$prefix" '
        { line = $0; sub(/\r$/, "", line) }
        line == start { inside = 1; next }
        inside && line == end { inside = 0; next }
        inside && index(line, prefix) == 1 { print line; found += 1 }
        END { if (found != 1) exit 2 }
    ' "$config"
}

gpteasy__snapshot_line() {
    local provider_id=$1 prefix=$2
    gpteasy__print_block "$provider_id" | awk -v prefix="$prefix" '
        index($0, prefix) == 1 { print; found += 1 }
        END { if (found != 1) exit 2 }
    '
}

gpteasy__expected_snapshot_line() {
    local provider_id=$1 prefix=$2 value
    case "$prefix" in
        'model = ')
            value=$(gpteasy__provider_model "$provider_id") || return 1
            printf 'model = %s\n' "$(gpteasy__toml_string "$value")"
            ;;
        'model_providers.gpteasy.name = ')
            value=$(gpteasy__provider_name "$provider_id") || return 1
            printf 'model_providers.gpteasy.name = %s\n' "$(gpteasy__toml_string "$value")"
            ;;
        'model_providers.gpteasy.base_url = ')
            value=$(gpteasy__provider_base_url "$provider_id") || return 1
            printf 'model_providers.gpteasy.base_url = %s\n' "$(gpteasy__toml_string "$value")"
            ;;
        *)
            gpteasy__snapshot_line "$provider_id" "$prefix"
            ;;
    esac
}

gpteasy__catalog_content_is_current() {
    local provider_id=$1 catalog_path=$2 expected_hash actual_hash
    expected_hash=$(gpteasy__provider_catalog_sha256 "$provider_id") || return 1
    actual_hash=$(gpteasy__file_hash "$catalog_path") || return 1
    [[ "$actual_hash" == "$expected_hash" ]]
}

gpteasy__current_state() {
    local config marker_info starts ends start_line end_line provider_id schema schema_count source relative credential catalog_line
    local prefix current_line expected_line
    gpteasy__model_catalog_path=
    config=$(gpteasy__config_path) || return
    if [[ ! -f "$config" ]]; then
        printf '%s\n' 'external'
        return
    fi
    marker_info=$(gpteasy__marker_info "$config") || return
    read -r starts ends start_line end_line <<<"$marker_info"
    if [[ "$starts" -eq 0 && "$ends" -eq 0 ]]; then
        printf '%s\n' 'external'
        return
    fi
    if [[ "$starts" -ne 1 || "$ends" -ne 1 || "$start_line" -ge "$end_line" ]]; then
        printf '%s\n' 'conflict'
        return
    fi
    provider_id=$(gpteasy__managed_metadata "$config" "$gpteasy__provider_id_prefix" 2>/dev/null) || {
        printf '%s\n' 'conflict'
        return
    }
    schema_count=$(gpteasy__managed_metadata_count "$config" "$gpteasy__schema_prefix") || return
    if [[ "$schema_count" -eq 0 ]]; then
        printf '%s\n' 'legacy'
        return
    fi
    if [[ "$schema_count" -ne 1 ]]; then
        printf '%s\n' 'conflict'
        return
    fi
    schema=$(gpteasy__managed_metadata "$config" "$gpteasy__schema_prefix" 2>/dev/null) || {
        printf '%s\n' 'conflict'
        return
    }
    case "$schema" in
        1)
            gpteasy__schema_v1_is_valid "$config" || {
                printf '%s\n' 'conflict'
                return
            }
            ;;
        2)
            gpteasy__schema_v2_is_valid "$config" || {
                printf '%s\n' 'conflict'
                return
            }
            catalog_line=$(gpteasy__managed_line "$config" 'model_catalog_json = ' 2>/dev/null) || {
                printf '%s\n' 'conflict'
                return
            }
            gpteasy__model_catalog_path=$(gpteasy__catalog_path_for \
                "$(gpteasy__managed_metadata "$config" "$gpteasy__source_id_prefix")" \
                "$(gpteasy__managed_metadata "$config" "$gpteasy__catalog_artifact_prefix")") || return 1
            ;;
        *)
            printf '%s\n' 'conflict'
            return
            ;;
    esac
    source=$(gpteasy__managed_metadata "$config" "$gpteasy__source_id_prefix" 2>/dev/null) || {
        printf '%s\n' 'conflict'
        return
    }
    [[ -n "$source" ]] || {
        printf '%s\n' 'conflict'
        return
    }
    if ! gpteasy__provider_name "$provider_id" >/dev/null 2>&1; then
        printf '%s\n' 'current'
        return
    fi
    for prefix in 'model = ' 'model_providers.gpteasy.name = ' 'model_providers.gpteasy.base_url = '; do
        current_line=$(gpteasy__managed_line "$config" "$prefix" 2>/dev/null) || {
            printf '%s\n' 'conflict'
            return
        }
        if [[ "$schema" == 1 ]]; then
            expected_line=$(gpteasy__expected_snapshot_line "$provider_id" "$prefix" 2>/dev/null)
        else
            expected_line=$(gpteasy__snapshot_line "$provider_id" "$prefix" 2>/dev/null)
        fi
        [[ -n "$expected_line" ]] || {
            printf '%s\n' 'conflict'
            return
        }
        if [[ "$current_line" != "$expected_line" ]]; then
            printf '%s\n' 'updated'
            return
        fi
    done
    for prefix in \
        'model_provider = "gpteasy"' \
        'model_providers.gpteasy.wire_api = "responses"' \
        'model_providers.gpteasy.auth.command = "sh"'; do
        gpteasy__managed_line "$config" "$prefix" >/dev/null 2>&1 || {
            printf '%s\n' 'conflict'
            return
        }
    done
    relative=$(gpteasy__managed_metadata "$config" "$gpteasy__credential_file_prefix" 2>/dev/null) || {
        printf '%s\n' 'conflict'
        return
    }
    case "$relative" in
        .gpteasy-shell/credentials/*/"$provider_id.token") ;;
        *)
            printf '%s\n' 'conflict'
            return
            ;;
    esac
    if [[ "$relative" == *'..'* || "$relative" == *'//'* ]]; then
        printf '%s\n' 'conflict'
        return
    fi
    credential="${CODEX_HOME:-"$HOME/.codex"}/$relative"
    if ! gpteasy__private_file_is_safe "$credential"; then
        printf '%s\n' 'conflict'
        return
    fi
    if [[ "$schema" == 2 ]] && ! gpteasy__catalog_content_is_current "$provider_id" "$gpteasy__model_catalog_path"; then
        printf '%s\n' 'updated'
        return
    fi
    if cmp -s -- "$credential" <(gpteasy__print_credential "$provider_id"); then
        printf '%s\n' 'current'
    else
        printf '%s\n' 'updated'
    fi
}

gpteasy__prepare_candidate() {
    local provider_id=$1 target_dir=${gpteasy__config_target%/*} block
    block=$(mktemp "$target_dir/.gpteasy-block.XXXXXX") || return
    gpteasy__candidate=$(mktemp "$target_dir/.config.toml.gpteasy.XXXXXX") || {
        rm -f -- "$block"
        return 1
    }
    if ! gpteasy__print_block "$provider_id" >"$block"; then
        rm -f -- "$block" "$gpteasy__candidate"
        return 1
    fi
    cat -- "$block" >"$gpteasy__candidate" || return
    rm -f -- "$block"
    if [[ -f "$gpteasy__config_target" ]]; then
        chmod --reference="$gpteasy__config_target" "$gpteasy__candidate" 2>/dev/null || chmod 600 "$gpteasy__candidate"
    else
        chmod 600 "$gpteasy__candidate"
    fi
    sync -f "$gpteasy__candidate" || return
    gpteasy__candidate_hash=$(gpteasy__file_hash "$gpteasy__candidate") || return
}

gpteasy__create_restore_point() {
    local stamp process_id previous sequence=0
    process_id=$$
    # Wall clocks can move backwards; the shared lock serializes this ordinal.
    previous=$(find "$gpteasy__restore_root" -mindepth 1 -maxdepth 1 -type d -name 'switch-v2-*' -print | sort -r | head -n 1) || return
    if [[ -n "$previous" ]]; then
        sequence=${previous##*/switch-v2-}
        sequence=${sequence%%-*}
        gpteasy__matches "$sequence" '^[0-9]{18}$' || return 1
        sequence=$((10#$sequence))
    fi
    (( sequence < 999999999999999999 )) || return 1
    sequence=$(printf '%018d' "$((sequence + 1))") || return
    stamp=$(date -u +%Y%m%dT%H%M%S%N) || return
    gpteasy__restore_point="$gpteasy__restore_root/switch-v2-$sequence-$stamp-$process_id-${RANDOM:-0}"
    mkdir -m 700 -- "$gpteasy__restore_point" || return
    printf '%s\n' "$gpteasy__config_kind" >"$gpteasy__restore_point/config-kind" || return
    if [[ -f "$gpteasy__config_target" ]]; then
        cat -- "$gpteasy__config_target" >"$gpteasy__restore_point/config.toml" || return
        chmod 600 "$gpteasy__restore_point/config.toml" || return
        sync -f "$gpteasy__restore_point/config.toml" || return
    fi
    if [[ "$gpteasy__config_kind" == symlink ]]; then
        printf '%s' "$gpteasy__config_link_value" >"$gpteasy__restore_point/symlink-target" || return
        chmod 600 "$gpteasy__restore_point/symlink-target" || return
    fi
    chmod 600 "$gpteasy__restore_point/config-kind" || return
    sync -f "$gpteasy__restore_point/config-kind" || return
    if [[ "$gpteasy__config_kind" == symlink ]]; then
        sync -f "$gpteasy__restore_point/symlink-target" || return
    fi
    sync -f "$gpteasy__restore_point" && sync -f "$gpteasy__restore_root" || return
    [[ "$(gpteasy__file_hash "$gpteasy__restore_point/config.toml")" == "$gpteasy__config_original_hash" ]] || return 1
}

gpteasy__discard_restore_point() {
    local point=${1:-${gpteasy__restore_point:-}}
    [[ -n "$point" && "$point" == "$gpteasy__restore_root/"* && -d "$point" && ! -L "$point" ]] || return 1
    rm -f -- "$point/config.toml" "$point/config-kind" "$point/symlink-target" || return
    rmdir -- "$point"
}

gpteasy__prune_restore_points() {
    local old
    while IFS= read -r old; do
        [[ -n "$old" ]] || continue
        gpteasy__discard_restore_point "$old" || return
    done < <(find "$gpteasy__restore_root" -mindepth 1 -maxdepth 1 -type d -name 'switch-*' -print | sort -r | awk 'NR > 5')
}

gpteasy__private_file_is_safe() {
    local file=$1 mode links kind owner
    [[ -f "$file" && ! -L "$file" ]] || return 1
    read -r owner mode links kind < <(stat -c '%u %a %h %F' -- "$file") || return 1
    [[ -r "$file" && -w "$file" && "$owner" == "$(gpteasy__current_uid)" && "$links" == 1 && "$kind" == 'regular file' ]]
}

gpteasy__owned_regular_file_is_safe() {
    local file=$1 links kind owner
    [[ -f "$file" && ! -L "$file" ]] || return 1
    read -r owner links kind < <(stat -c '%u %h %F' -- "$file") || return 1
    [[ -r "$file" && -w "$file" && "$owner" == "$(gpteasy__current_uid)" && "$links" == 1 && "$kind" == 'regular file' ]]
}

gpteasy__install_credential() {
    local provider_id=$1 directory temporary destination
    directory="$gpteasy__credentials_root/$gpteasy__export_id"
    gpteasy__ensure_private_dir "$directory" || return
    destination="$directory/$provider_id.token"
    temporary=$(mktemp "$directory/.credential.XXXXXX") || return
    if ! gpteasy__print_credential "$provider_id" >"$temporary"; then
        rm -f -- "$temporary"
        return 1
    fi
    chmod 600 "$temporary" || return
    sync -f "$temporary" || return
    gpteasy__credential_created=0
    if [[ -e "$destination" || -L "$destination" ]]; then
        if ! gpteasy__private_file_is_safe "$destination" || ! cmp -s -- "$temporary" "$destination"; then
            printf '%s\n' '已有 Linux 凭据工件不安全或内容不一致。' >&2
            rm -f -- "$temporary"
            return 1
        fi
        rm -f -- "$temporary"
    else
        mv -- "$temporary" "$destination" || {
            rm -f -- "$temporary"
            return 1
        }
        gpteasy__credential_created=1
    fi
    gpteasy__credential_path=$destination
}

gpteasy__credential_reference_is_valid() {
    local reference=$1 tail source file
    case "$reference" in
        .gpteasy-shell/credentials/*/*.token) ;;
        *) return 1 ;;
    esac
    case "$reference" in *..* | *//* | *[!A-Za-z0-9._/-]*) return 1 ;; esac
    tail=${reference#'.gpteasy-shell/credentials/'}
    source=${tail%%/*}
    file=${tail#*/}
    [[ -n "$source" && "$file" != "$tail" && "$file" != */* ]]
}

gpteasy__collect_config_credential_reference() {
    local file=$1 references=$2 require_private=${3:-1} reference
    [[ -e "$file" ]] || return 0
    if [[ "$require_private" -eq 1 ]]; then
        gpteasy__private_file_is_safe "$file" || return 1
    else
        gpteasy__owned_regular_file_is_safe "$file" || return 1
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
    [[ -n "$reference" ]] || return 0
    gpteasy__credential_reference_is_valid "$reference" || return 1
    printf '%s\n' "$reference" >>"$references"
}

gpteasy__cleanup_credentials() {
    local references root file reference credential relative codex_home
    codex_home=${CODEX_HOME:-"$HOME/.codex"}
    references=$(mktemp "$gpteasy__tmp_root/.credential-references.XXXXXX") || return
    chmod 600 "$references" || {
        rm -f -- "$references"
        return 1
    }
    gpteasy__collect_config_credential_reference "$gpteasy__config_target" "$references" 0 || {
        rm -f -- "$references"
        return 1
    }
    for root in "$gpteasy__restore_root" "$gpteasy__state_root/desktop-backups"; do
        [[ -e "$root" ]] || continue
        gpteasy__directory_is_owned "$root" 1 || {
            rm -f -- "$references"
            return 1
        }
        while IFS= read -r file; do
            [[ -n "$file" ]] || continue
            gpteasy__collect_config_credential_reference "$file" "$references" 1 || {
                rm -f -- "$references"
                return 1
            }
        done < <(find "$root" -type f -name '*.toml' -print)
    done
    if [[ -e "$gpteasy__lock_root/active/references" ]]; then
        gpteasy__private_file_is_safe "$gpteasy__lock_root/active/references" || {
            rm -f -- "$references"
            return 1
        }
        while IFS= read -r reference; do
            [[ -n "$reference" ]] || continue
            gpteasy__credential_reference_is_valid "$reference" || {
                rm -f -- "$references"
                return 1
            }
            printf '%s\n' "$reference" >>"$references"
        done <"$gpteasy__lock_root/active/references"
    fi
    while IFS= read -r credential; do
        [[ -n "$credential" ]] || continue
        gpteasy__private_file_is_safe "$credential" || {
            rm -f -- "$references"
            return 1
        }
        relative=${credential#"$codex_home/"}
        gpteasy__credential_reference_is_valid "$relative" || {
            rm -f -- "$references"
            return 1
        }
        grep -Fqx -- "$relative" "$references" || rm -f -- "$credential" || {
            rm -f -- "$references"
            return 1
        }
    done < <(find "$gpteasy__credentials_root" -mindepth 2 -maxdepth 2 -type f -name '*.token' -print)
    rm -f -- "$references" || return
    find "$gpteasy__credentials_root" -mindepth 1 -maxdepth 1 -type d -empty -exec rmdir -- {} \;
}

gpteasy__cleanup_failed_apply() {
    rm -f -- "${gpteasy__candidate:-}" 2>/dev/null || true
    if [[ "${gpteasy__credential_created:-0}" -eq 1 && -n "${gpteasy__credential_path:-}" ]]; then
        rm -f -- "$gpteasy__credential_path" 2>/dev/null || true
    fi
    # Keep complete unreferenced catalogs: no catalog GC in this protocol.
    if [[ -n "${gpteasy__restore_point:-}" && -d "$gpteasy__restore_point" ]]; then
        gpteasy__discard_restore_point "$gpteasy__restore_point" 2>/dev/null || true
    fi
}

gpteasy__apply_provider_locked() {
    local provider_id=$1 target_dir
    gpteasy__candidate=
    gpteasy__restore_point=
    gpteasy__credential_created=0
    gpteasy__credential_path=
    gpteasy__model_catalog_created=0
    gpteasy__model_catalog_path=
    gpteasy__check_codex_compatibility || return
    gpteasy__resolve_config_target || return
    gpteasy__create_restore_point || {
        gpteasy__cleanup_failed_apply
        gpteasy__evidence restore_point failed
        return 1
    }
    gpteasy__evidence restore_point ready
    gpteasy__install_model_catalog "$provider_id" || {
        gpteasy__cleanup_failed_apply
        gpteasy__evidence catalog_commit rejected
        return 1
    }
    gpteasy__prepare_candidate "$provider_id" || {
        gpteasy__cleanup_failed_apply
        gpteasy__evidence config_candidate failed
        return 1
    }
    gpteasy__install_credential "$provider_id" || {
        gpteasy__cleanup_failed_apply
        return 1
    }
    if ! gpteasy__schema_v2_is_valid "$gpteasy__candidate" || ! gpteasy__config_target_unchanged; then
        gpteasy__evidence config_precommit changed
        printf '%s\n' 'Codex 配置在操作期间发生变化，已停止覆盖。' >&2
        gpteasy__cleanup_failed_apply
        return 1
    fi
    target_dir=${gpteasy__config_target%/*}
    if ! mv -f -- "$gpteasy__candidate" "$gpteasy__config_target"; then
        gpteasy__cleanup_failed_apply
        gpteasy__evidence config_commit failed
        return 1
    fi
    gpteasy__candidate=
    gpteasy__model_catalog_created=0
    gpteasy__evidence config_commit committed
    if ! sync -f "$target_dir" 2>/dev/null || [[ "$(gpteasy__file_hash "$gpteasy__config_target")" != "$gpteasy__candidate_hash" ]] ||
        ! gpteasy__schema_v2_is_valid "$gpteasy__config_target"; then
        gpteasy__evidence config_readback indeterminate
        gpteasy__refresh_pending >&2
        printf '%s\n' '配置替换后的复核失败，请使用 restore 检查最近恢复点。' >&2
        return 1
    fi
    gpteasy__evidence config_readback verified
    gpteasy__refresh_pending
    gpteasy__prune_restore_points || return
    if ! gpteasy__cleanup_credentials; then
        printf '%s\n' '警告：旧凭据清理无法安全完成，已保留相关文件。配置已保存，但 CLI/共享后台服务可能仍使用旧配置。' >&2
    fi
    printf '已切换到：%s\n' "$(gpteasy__provider_name "$provider_id")"
    if [[ "${gpteasy__permission_warning:-0}" -eq 1 ]]; then
        printf '%s\n' '警告：目标环境中已有文件或目录权限允许其他用户访问，请按需收紧；本次新建的凭据和恢复点仍使用私有权限。' >&2
    fi
    if [[ "$(gpteasy__current_uid)" == 0 && -n "${SUDO_USER:-}" && -z "${CODEX_HOME:-}" ]]; then
        printf '当前正在管理 root 的 Codex 环境：%s。若要配置原用户，请退出 sudo 后运行，或使用 sudo -u <用户> -H。\n' "$HOME/.codex" >&2
    fi
}

gpteasy__switch_provider() {
    local provider_id=$1 result
    gpteasy__require_codex_home || return
    gpteasy__resolve_config_target || return
    gpteasy__require_existing_private_state_safe || return
    gpteasy__prepare_private_state || return
    gpteasy__acquire_lock switch || return
    if gpteasy__apply_provider_locked "$provider_id"; then result=0; else result=$?; fi
    gpteasy__release_lock || {
        printf '%s\n' '配置已处理，但 shell 锁释放失败；请检查 gpteasy unlock。' >&2
        return 1
    }
    return "$result"
}

gpteasy__select_provider() {
    local current= state= choice provider_id name model marker index
    current=$(gpteasy__current_provider_id 2>/dev/null || true)
    state=$(gpteasy__current_state 2>/dev/null || true)
    printf '%s\n' '可用供应商：'
    index=1
    while [[ "$index" -le "$gpteasy__provider_count" ]]; do
        provider_id=$(gpteasy__provider_id "$index") || return
        name=$(gpteasy__provider_name "$provider_id") || return
        model=$(gpteasy__provider_model "$provider_id") || return
        marker=
        if [[ "$current" == "$provider_id" ]]; then
            case "$state" in
                current) marker=' [当前]' ;;
                updated) marker=' [当前，有更新]' ;;
                legacy) marker=' [当前，旧格式]' ;;
            esac
        fi
        printf '  %s) %s (%s)%s\n' "$index" "$name" "$model" "$marker"
        index=$((index + 1))
    done
{{GPTEASY_SELECT_READ}}
    case "$choice" in
        '' | q | Q)
            printf '%s\n' '已取消，不修改配置。'
            return
            ;;
        *[!0-9]*)
            printf '%s\n' '无效的供应商编号。' >&2
            return 2
            ;;
    esac
    provider_id=$(gpteasy__provider_id "$choice") || {
        printf '%s\n' '无效的供应商编号。' >&2
        return 2
    }
    gpteasy__switch_provider "$provider_id"
}

gpteasy__provider_label_for_file() {
    local file=$1 provider_id name marker_info starts ends start_line end_line
    if [[ ! -f "$file" ]]; then
        printf '%s\n' '未配置'
        return
    fi
    marker_info=$(gpteasy__marker_info "$file") || return
    read -r starts ends start_line end_line <<<"$marker_info"
    if [[ "$starts" -eq 0 && "$ends" -eq 0 ]]; then
        printf '%s\n' '外部配置'
        return
    fi
    if [[ "$starts" -ne 1 || "$ends" -ne 1 || "$start_line" -ge "$end_line" ]]; then
        printf '%s\n' '管理冲突'
        return
    fi
    if ! provider_id=$(gpteasy__managed_metadata "$file" "$gpteasy__provider_id_prefix" 2>/dev/null); then
        printf '%s\n' '管理冲突'
        return
    fi
    if name=$(gpteasy__provider_name "$provider_id" 2>/dev/null); then
        printf '%s\n' "$name"
    else
        printf '不在此快照中的供应商 %s\n' "$provider_id"
    fi
}

gpteasy__restore_locked() {
    local latest kind expected_link current_label target_label choice candidate= candidate_hash= target_dir backup_hash= backup_identity= kind_hash= link_hash=
    latest=$(find "$gpteasy__restore_root" -mindepth 1 -maxdepth 1 -type d -name 'switch-*' -print | sort -r | head -n 1)
    if [[ -z "$latest" ]]; then
        printf '%s\n' '没有可恢复的 Linux 恢复点。' >&2
        return 1
    fi
    if ! gpteasy__directory_is_owned "$latest" 1 || ! gpteasy__private_file_is_safe "$latest/config-kind"; then
        printf '%s\n' '最新 Linux 恢复点的权限或访问能力不安全。' >&2
        return 1
    fi
    kind=$(cat -- "$latest/config-kind") || return
    case "$kind" in
        missing) ;;
        regular)
            gpteasy__private_file_is_safe "$latest/config.toml" || return 1
            if [[ "$(gpteasy__managed_metadata "$latest/config.toml" "$gpteasy__schema_prefix" 2>/dev/null || true)" == 2 ]] &&
                ! gpteasy__schema_v2_is_valid "$latest/config.toml"; then
                printf '%s\n' '恢复点引用的 Linux 模型目录缺失、篡改或绑定失效，恢复已停止。' >&2
                return 1
            fi
            ;;
        symlink)
            gpteasy__private_file_is_safe "$latest/config.toml" || return 1
            gpteasy__private_file_is_safe "$latest/symlink-target" || return 1
            if [[ "$(gpteasy__managed_metadata "$latest/config.toml" "$gpteasy__schema_prefix" 2>/dev/null || true)" == 2 ]] &&
                ! gpteasy__schema_v2_is_valid "$latest/config.toml"; then
                printf '%s\n' '恢复点引用的 Linux 模型目录缺失、篡改或绑定失效，恢复已停止。' >&2
                return 1
            fi
            ;;
        *)
            printf '%s\n' '最新 Linux 恢复点格式损坏。' >&2
            return 1
            ;;
    esac
    kind_hash=$(gpteasy__file_hash "$latest/config-kind") || return
    if [[ "$kind" != missing ]]; then
        backup_hash=$(gpteasy__file_hash "$latest/config.toml") || return
        backup_identity=$(stat -c '%d:%i:%u:%a:%h:%F' -- "$latest/config.toml") || return
    fi
    if [[ "$kind" == symlink ]]; then
        link_hash=$(gpteasy__file_hash "$latest/symlink-target") || return
    fi
    gpteasy__resolve_config_target || return
    if [[ "$kind" == symlink ]]; then
        expected_link=$(cat -- "$latest/symlink-target") || return
        if [[ "$gpteasy__config_kind" != symlink || "$gpteasy__config_link_value" != "$expected_link" ]]; then
            printf '%s\n' 'config.toml 符号链接目标已变化，恢复已停止。' >&2
            return 1
        fi
    elif [[ "$kind" == regular && "$gpteasy__config_kind" != regular ]]; then
        printf '%s\n' 'config.toml 文件类型已变化，恢复已停止。' >&2
        return 1
    elif [[ "$kind" == missing && "$gpteasy__config_kind" != regular ]]; then
        printf '%s\n' 'config.toml 文件类型已变化，恢复已停止。' >&2
        return 1
    fi
    current_label=$(gpteasy__provider_label_for_file "$gpteasy__config_target") || return
    if [[ "$kind" == missing ]]; then
        target_label=未配置
    else
        target_label=$(gpteasy__provider_label_for_file "$latest/config.toml") || return
    fi
    printf '当前状态：%s\n' "$current_label"
    printf '恢复目标：%s\n' "$target_label"
    printf '%s\n' '警告：恢复可能覆盖桌面 GPTEasy 或其它脚本之后完成的修改。'
{{GPTEASY_RESTORE_READ}}
    case "$choice" in
        y | Y) ;;
        *)
            printf '%s\n' '已取消，不修改配置。'
            return
            ;;
    esac
    if ! gpteasy__private_file_is_safe "$latest/config-kind" ||
        [[ "$(gpteasy__file_hash "$latest/config-kind")" != "$kind_hash" ]] ||
        { [[ "$kind" != missing ]] && { ! gpteasy__private_file_is_safe "$latest/config.toml" ||
            [[ "$(gpteasy__file_hash "$latest/config.toml")" != "$backup_hash" ]] ||
            [[ "$(stat -c '%d:%i:%u:%a:%h:%F' -- "$latest/config.toml")" != "$backup_identity" ]]; }; } ||
        { [[ "$kind" == symlink ]] && { ! gpteasy__private_file_is_safe "$latest/symlink-target" ||
            [[ "$(gpteasy__file_hash "$latest/symlink-target")" != "$link_hash" ]]; }; }; then
        printf '%s\n' '恢复点在确认期间发生变化，恢复已停止。' >&2
        gpteasy__evidence restore_precommit backup_rejected
        return 1
    fi
    target_dir=${gpteasy__config_target%/*}
    if [[ "$kind" != missing ]]; then
        candidate=$(mktemp "$target_dir/.config.toml.gpteasy-restore.XXXXXX") || return
        if ! cat -- "$latest/config.toml" >"$candidate"; then
            rm -f -- "$candidate"
            return 1
        fi
        if [[ -f "$gpteasy__config_target" ]]; then
            chmod --reference="$gpteasy__config_target" "$candidate" 2>/dev/null || chmod 600 "$candidate"
        else
            chmod 600 "$candidate"
        fi
        sync -f "$candidate" || {
            rm -f -- "$candidate"
            return 1
        }
        candidate_hash=$(gpteasy__file_hash "$candidate") || return
        if [[ "$candidate_hash" != "$backup_hash" ]]; then
            rm -f -- "$candidate"
            gpteasy__evidence restore_precommit backup_rejected
            return 1
        fi
    fi
    if [[ "$kind" != missing && "$(gpteasy__managed_metadata "$candidate" "$gpteasy__schema_prefix" 2>/dev/null || true)" == 2 ]] &&
        ! gpteasy__schema_v2_is_valid "$candidate"; then
        printf '%s\n' '恢复点引用的 Linux 模型目录在确认期间变化，恢复已停止。' >&2
        gpteasy__evidence restore_precommit catalog_rejected
        rm -f -- "$candidate"
        return 1
    fi
    if ! gpteasy__config_target_unchanged; then
        printf '%s\n' 'Codex 配置在确认期间发生变化，恢复已停止。' >&2
        rm -f -- "$candidate" 2>/dev/null || true
        return 1
    fi
    if [[ "$kind" == missing ]]; then
        rm -f -- "$gpteasy__config_target" || {
            gpteasy__evidence restore_commit failed
            return 1
        }
    else
        mv -f -- "$candidate" "$gpteasy__config_target" || {
            rm -f -- "$candidate"
            gpteasy__evidence restore_commit failed
            return 1
        }
    fi
    gpteasy__evidence restore_commit committed
    gpteasy__refresh_pending
    sync -f "$target_dir" 2>/dev/null || {
        gpteasy__evidence restore_readback sync_failed
        return 1
    }
    if { [[ "$kind" == missing ]] && [[ -e "$gpteasy__config_target" || -L "$gpteasy__config_target" ]]; } ||
        { [[ "$kind" != missing ]] && [[ "$(gpteasy__file_hash "$gpteasy__config_target")" != "$candidate_hash" ]]; }; then
        gpteasy__evidence restore_readback config_changed
        return 1
    fi
    if [[ "$kind" != missing && "$(gpteasy__managed_metadata "$gpteasy__config_target" "$gpteasy__schema_prefix" 2>/dev/null || true)" == 2 ]]; then
        gpteasy__schema_v2_is_valid "$gpteasy__config_target" || {
            gpteasy__evidence restore_readback catalog_rejected
            return 1
        }
    fi
    gpteasy__discard_restore_point "$latest" || return
    if ! gpteasy__cleanup_credentials; then
        printf '%s\n' '警告：旧凭据清理无法安全完成，已保留相关文件。配置已保存，但 CLI/共享后台服务可能仍使用旧配置。' >&2
    fi
    gpteasy__evidence restore_readback verified
    printf '%s\n' '已恢复最近一次 shell 切换前的配置。'
}

gpteasy__restore() {
    local result
    gpteasy__require_codex_home || return
    gpteasy__prepare_private_state || return
    gpteasy__acquire_lock restore || return
    if gpteasy__restore_locked; then result=0; else result=$?; fi
    gpteasy__release_lock || {
        printf '%s\n' '恢复已处理，但 shell 锁释放失败；请检查 gpteasy unlock。' >&2
        return 1
    }
    return "$result"
}

gpteasy__info() {
    local codex_home=${CODEX_HOME:-"$HOME/.codex"} config
    config="$codex_home/config.toml"
    [[ ! -e "$codex_home" || -L "$codex_home" ]] || gpteasy__warn_if_permissions_are_broad "$codex_home"
    [[ ! -e "$config" ]] || gpteasy__warn_if_permissions_are_broad "$config"
    printf '目标环境：%s\n' "${CODEX_HOME:-"$HOME/.codex"}"
    printf 'Linux 导出 ID：%s\n' "$gpteasy__export_id"
    printf '%s\n' '管理区块 schema：2'
    printf '%s\n' 'Shell：{{GPTEASY_SHELL_LABEL}}'
    printf '供应商数量：%s\n' "$gpteasy__provider_count"
    printf '%s\n' 'Codex CLI：明确切换时核验原生 Linux 目录/schema 能力'
    if [[ "${gpteasy__permission_warning:-0}" -eq 1 ]]; then
        printf '%s\n' '权限风险：是（已有文件或目录允许其他用户访问）'
    else
        printf '%s\n' '权限风险：否'
    fi
}

gpteasy__unlock() {
    local active owner_file owner token pid process_start operation actual_start choice
    local owner_hash owner_signature
    gpteasy__require_codex_home || return
    gpteasy__prepare_private_state || return
    active="$gpteasy__lock_root/active"
    owner_file="$active/owner"
    if [[ ! -e "$active" && ! -L "$active" ]]; then
        printf '%s\n' '当前没有 shell owner 锁。'
        return
    fi
    if ! gpteasy__directory_is_owned "$active" 1 || ! gpteasy__private_file_is_safe "$owner_file"; then
        printf '%s\n' '锁目录或 owner 文件不安全，拒绝解锁。' >&2
        return 1
    fi
    owner=$(gpteasy__lock_value "$owner_file" owner 2>/dev/null) || return 1
    token=$(gpteasy__lock_value "$owner_file" token 2>/dev/null) || return 1
    pid=$(gpteasy__lock_value "$owner_file" pid 2>/dev/null) || return 1
    process_start=$(gpteasy__lock_value "$owner_file" process_start 2>/dev/null) || return 1
    operation=$(gpteasy__lock_value "$owner_file" operation 2>/dev/null) || return 1
    if [[ "$owner" == desktop ]]; then
        printf '%s\n' '桌面 owner 锁只能由桌面 WSL2 Saga 恢复，shell 不会删除。' >&2
        return 1
    fi
    if [[ "$owner" != shell ]] || ! gpteasy__matches "$pid" '^[0-9]+$' || ! gpteasy__matches "$process_start" '^[0-9]+$' || ! gpteasy__matches "$operation" '^[a-z_]+$' || [[ -z "$token" ]]; then
        printf '%s\n' 'shell owner 锁格式损坏，拒绝自动删除。' >&2
        return 1
    fi
    actual_start=$(awk '{print $22}' "/proc/$pid/stat" 2>/dev/null || true)
    if [[ -n "$actual_start" && "$actual_start" == "$process_start" ]]; then
        printf 'shell owner 锁仍处于活动状态（operation=%s），拒绝删除。\n' "$operation" >&2
        return 1
    fi
    owner_hash=$(gpteasy__file_hash "$owner_file") || return
    owner_signature=$(stat -c '%d:%i:%u:%a:%h:%F' -- "$owner_file") || return
    printf '检测到失效的 shell owner 锁（operation=%s）。\n' "$operation"
{{GPTEASY_UNLOCK_READ}}
    case "$choice" in
        y | Y) ;;
        *)
            printf '%s\n' '已取消，不修改锁。'
            return
            ;;
    esac
    if [[ ! -f "$owner_file" || "$(gpteasy__file_hash "$owner_file")" != "$owner_hash" || "$(stat -c '%d:%i:%u:%a:%h:%F' -- "$owner_file")" != "$owner_signature" ]]; then
        printf '%s\n' '锁在确认期间发生变化，拒绝删除。' >&2
        return 1
    fi
    rm -f -- "$owner_file" || return
    rmdir -- "$active" || return
    printf '%s\n' '已删除失效的 shell 锁。'
}

gpteasy__current() {
    local config current name state suffix=
    config=$(gpteasy__config_path) || return
    if [[ ! -e "$config" && ! -L "$config" ]]; then
        printf '%s\n' '当前未配置供应商。'
        return
    fi
    if ! current=$(gpteasy__current_provider_id); then
        printf '%s\n' '当前配置不包含可识别的 GPTEasy 管理区块。'
        return 1
    fi
    if name=$(gpteasy__provider_name "$current" 2>/dev/null); then
        state=$(gpteasy__current_state 2>/dev/null || true)
        case "$state" in
            updated) suffix='（配置有更新）' ;;
            legacy) suffix='（旧格式）' ;;
            conflict) suffix='（管理冲突）' ;;
        esac
        printf '当前供应商：%s%s\n' "$name" "$suffix"
    else
        printf '当前供应商不在此 Linux 供应商快照中：%s\n' "$current"
    fi
}

function codex-full() {
    command codex --dangerously-bypass-approvals-and-sandbox "$@"
}

gpteasy() {
{{GPTEASY_FUNCTION_OPTIONS}}
    local command=${1:-}
    case "$command" in
        help | --help | -h)
            gpteasy__help
            return
            ;;
    esac
    gpteasy__permission_warning=0
    gpteasy__require_snapshot_safe || return
    gpteasy__require_existing_private_state_safe || return
    case "$command" in
        '')
            gpteasy__select_provider
            ;;
        current)
            gpteasy__current
            ;;
        restore)
            gpteasy__restore
            ;;
        info)
            gpteasy__info
            ;;
        unlock)
            gpteasy__unlock
            ;;
        *)
            printf '未知命令：%s\n' "$command" >&2
            gpteasy__help >&2
            return 2
            ;;
    esac
}

{{GPTEASY_DIRECT_EXECUTION}}
