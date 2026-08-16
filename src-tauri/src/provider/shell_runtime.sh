
gpteasy__start_marker='# >>> GPTEasy managed provider >>>'
gpteasy__end_marker='# <<< GPTEasy managed provider <<<'
gpteasy__provider_id_prefix='# GPTEasy provider-id:'
gpteasy__schema_prefix='# GPTEasy schema-version:'
gpteasy__source_id_prefix='# GPTEasy source-id:'
gpteasy__credential_file_prefix='# GPTEasy credential-file:'
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

gpteasy__require_snapshot_private() {
    local owner mode links kind
    if [[ -L "$gpteasy__script_path" ]]; then
        printf '%s\n' '导出文件不能是符号链接；除帮助外已拒绝执行。' >&2
        return 1
    fi
    if ! read -r owner mode links kind < <(stat -c '%u %a %h %F' -- "$gpteasy__script_path" 2>/dev/null); then
        printf '%s\n' '无法确认导出文件权限；除帮助外已拒绝执行。' >&2
        return 1
    fi
    if [[ "$owner" != "$(id -u)" || "$links" != 1 || "$kind" != 'regular file' || "${mode: -2}" != '00' ]]; then
        printf '%s\n' '导出文件必须仅由当前用户持有和读取；除帮助外已拒绝执行。' >&2
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

gpteasy__require_codex_version() {
    local output version remainder major minor patch
    if ! command -v codex >/dev/null 2>&1; then
        printf '%s\n' '写入前需要可用的 codex-cli 0.147.0 或更高版本。' >&2
        return 1
    fi
    if ! output=$(codex --version 2>/dev/null); then
        printf '%s\n' '无法确认 Codex CLI 版本，未写入任何内容。' >&2
        return 1
    fi
    if ! gpteasy__matches "$output" '^codex-cli[[:space:]]+[0-9]+\.[0-9]+\.[0-9]+([-+][^[:space:]]+)?$'; then
        printf '%s\n' '无法识别 Codex CLI 版本，未写入任何内容。' >&2
        return 1
    fi
    version=${output#codex-cli }
    version=${version%%[-+]*}
    major=${version%%.*}
    remainder=${version#*.}
    minor=${remainder%%.*}
    patch=${remainder#*.}
    if ((major == 0 && (minor < 147 || (minor == 147 && patch < 0)))); then
        printf '%s\n' 'Codex CLI 版本低于 0.147.0，未写入任何内容。' >&2
        return 1
    fi
}

gpteasy__directory_is_owned() {
    local directory=$1 require_private=${2:-0} owner mode kind
    [[ ! -L "$directory" ]] || return 1
    read -r owner mode kind < <(stat -c '%u %a %F' -- "$directory" 2>/dev/null) || return 1
    [[ "$owner" == "$(id -u)" && "$kind" == 'directory' ]] || return 1
    if [[ "$require_private" -eq 1 ]]; then
        [[ "${mode: -2}" == '00' ]]
    else
        (( (8#$mode & 8#22) == 0 ))
    fi
}

gpteasy__ensure_private_dir() {
    local directory=$1
    if [[ ! -e "$directory" && ! -L "$directory" ]]; then
        mkdir -m 700 -- "$directory" || return
    fi
    if ! gpteasy__directory_is_owned "$directory" 1; then
        printf '私有目录权限或所有者不安全：%s\n' "$directory" >&2
        return 1
    fi
}

gpteasy__prepare_private_state() {
    local codex_home=${CODEX_HOME:-"$HOME/.codex"}
    if [[ ! -e "$codex_home" && ! -L "$codex_home" ]]; then
        mkdir -p -m 700 -- "$codex_home" || return
    fi
    if ! gpteasy__directory_is_owned "$codex_home" 0; then
        printf '%s\n' '目标 Codex 环境不是当前用户所有的普通目录。' >&2
        return 1
    fi
    gpteasy__state_root="$codex_home/.gpteasy-shell"
    gpteasy__credentials_root="$gpteasy__state_root/credentials"
    gpteasy__restore_root="$gpteasy__state_root/shell-restore"
    gpteasy__tmp_root="$gpteasy__state_root/tmp"
    gpteasy__lock_root="$gpteasy__state_root/lock"
    gpteasy__ensure_private_dir "$gpteasy__state_root" || return
    gpteasy__ensure_private_dir "$gpteasy__credentials_root" || return
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
        if ! gpteasy__directory_is_owned "$item" 1; then
            printf '%s\n' 'Linux 私有状态目录的权限或所有者不安全。' >&2
            return 1
        fi
    done < <(find "$root" -type d -print0)
    while IFS= read -r -d '' item; do
        if ! gpteasy__private_file_is_safe "$item"; then
            printf '%s\n' 'Linux 私有状态文件的权限、所有者或链接数不安全。' >&2
            return 1
        fi
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
        sha256sum -- "$1" | awk '{print $1}'
    else
        printf '%s\n' 'missing'
    fi
}

gpteasy__resolve_config_target() {
    local entry owner links kind parent
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
        read -r owner links kind < <(stat -Lc '%u %h %F' -- "$gpteasy__config_target") || return
        if [[ "$owner" != "$(id -u)" || "$links" != 1 || "$kind" != 'regular file' ]]; then
            printf '%s\n' 'config.toml 符号链接最终目标不安全。' >&2
            return 1
        fi
    elif [[ -e "$entry" ]]; then
        gpteasy__config_kind=regular
        gpteasy__config_target=$entry
        gpteasy__config_link_value=
        gpteasy__config_entry_signature=$(stat -c '%d:%i:%u:%a:%h:%F' -- "$entry") || return
        read -r owner links kind < <(stat -c '%u %h %F' -- "$entry") || return
        if [[ "$owner" != "$(id -u)" || "$links" != 1 || "$kind" != 'regular file' ]]; then
            printf '%s\n' 'config.toml 必须是当前用户所有且没有 hardlink 的普通文件。' >&2
            return 1
        fi
    else
        gpteasy__config_kind=missing
        gpteasy__config_target=$entry
        gpteasy__config_link_value=
        gpteasy__config_entry_signature=missing
    fi
    parent=${gpteasy__config_target%/*}
    if ! gpteasy__directory_is_owned "$parent" 0; then
        printf '%s\n' 'config.toml 最终目标目录不安全。' >&2
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
    local signature target target_signature owner links kind
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
        read -r owner links kind < <(stat -Lc '%u %h %F' -- "$gpteasy__config_target") || return
        [[ "$owner" == "$(id -u)" && "$links" == 1 && "$kind" == 'regular file' ]] || return 1
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

gpteasy__has_unmanaged_conflict() {
    local config=$1
    [[ -f "$config" ]] || return 1
    awk '
        { line = $0; sub(/\r$/, "", line) }
        /^[[:space:]]*\[/ { in_table = 1 }
        !in_table && line ~ /^[[:space:]]*(model|model_provider)[[:space:]]*=/ { conflict = 1 }
        line ~ /^[[:space:]]*\[model_providers\.gpteasy\][[:space:]]*$/ { conflict = 1 }
        line ~ /^[[:space:]]*model_providers\.gpteasy\./ { conflict = 1 }
        END { exit conflict ? 0 : 1 }
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
        line == "model_provider = \"gpteasy\"" { model_provider_count += 1; next }
        index(line, "model_providers.gpteasy.name = ") == 1 { name_count += 1; next }
        index(line, "model_providers.gpteasy.base_url = ") == 1 { base_url_count += 1; next }
        line == "model_providers.gpteasy.wire_api = \"responses\"" { wire_count += 1; next }
        line == "model_providers.gpteasy.supports_websockets = false" { websocket_count += 1; next }
        line == "model_providers.gpteasy.requires_openai_auth = false" { auth_mode_count += 1; next }
        line == "model_providers.gpteasy.auth.command = \"sh\"" { auth_command_count += 1; next }
        index(line, "model_providers.gpteasy.auth.args = ") == 1 { auth_args_count += 1; next }
        { invalid = 1 }
        END {
            valid = !invalid && schema_count == 1 && provider_count == 1 && source_count == 1 &&
                credential_count == 1 && model_count == 1 && model_provider_count == 1 &&
                name_count == 1 && base_url_count == 1 && wire_count == 1 && websocket_count == 1 &&
                auth_mode_count == 1 && auth_command_count == 1 && auth_args_count == 1
            exit valid ? 0 : 1
        }
    ' "$config" || return 1
    auth_args=$(gpteasy__managed_line "$config" 'model_providers.gpteasy.auth.args = ' 2>/dev/null) || return 1
    expected_auth_args="model_providers.gpteasy.auth.args = [\"-c\", 'cat -- \"\${CODEX_HOME:-\$HOME/.codex}/$relative\"']"
    [[ "$auth_args" == "$expected_auth_args" ]]
}

gpteasy__inspect_writable_config() {
    local marker_info schema schema_count
    marker_info=$(gpteasy__marker_info "$gpteasy__config_target") || return
    read -r gpteasy__starts gpteasy__ends gpteasy__start_line gpteasy__end_line <<<"$marker_info"
    if [[ "$gpteasy__starts" -eq 0 && "$gpteasy__ends" -eq 0 ]]; then
        if gpteasy__has_unmanaged_conflict "$gpteasy__config_target"; then
            printf '%s\n' '检测到管理区块外的供应商字段，需要先由桌面 GPTEasy 完成结构化迁移。' >&2
            return 1
        fi
        return
    fi
    if [[ "$gpteasy__starts" -ne 1 || "$gpteasy__ends" -ne 1 || "$gpteasy__start_line" -ge "$gpteasy__end_line" ]]; then
        printf '%s\n' 'GPTEasy 管理区块边界损坏、重复或倒置，已停止修改。' >&2
        return 1
    fi
    gpteasy__managed_metadata "$gpteasy__config_target" "$gpteasy__provider_id_prefix" >/dev/null || {
        printf '%s\n' 'GPTEasy 管理区块的供应商 ID 无效。' >&2
        return 1
    }
    schema_count=$(gpteasy__managed_metadata_count "$gpteasy__config_target" "$gpteasy__schema_prefix") || return
    if [[ "$schema_count" -eq 0 ]]; then
        return
    fi
    if [[ "$schema_count" -ne 1 ]]; then
        printf '%s\n' 'GPTEasy 管理区块 schema 重复，已停止修改。' >&2
        return 1
    fi
    schema=$(gpteasy__managed_metadata "$gpteasy__config_target" "$gpteasy__schema_prefix") || return
    if [[ "$schema" != 1 ]]; then
        printf '%s\n' 'GPTEasy 管理区块 schema 未知，已停止修改。' >&2
        return 1
    fi
    if ! gpteasy__schema_v1_is_valid "$gpteasy__config_target"; then
        printf '%s\n' 'GPTEasy 管理区块 schema v1 内容损坏，已停止修改。' >&2
        return 1
    fi
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

gpteasy__current_state() {
    local config marker_info starts ends start_line end_line provider_id schema schema_count source relative credential
    local prefix current_line expected_line
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
    if [[ "$schema" != 1 ]] || ! gpteasy__schema_v1_is_valid "$config"; then
        printf '%s\n' 'conflict'
        return
    fi
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
        expected_line=$(gpteasy__snapshot_line "$provider_id" "$prefix" 2>/dev/null) || {
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
    if cmp -s -- "$credential" <(gpteasy__print_credential "$provider_id"); then
        printf '%s\n' 'current'
    else
        printf '%s\n' 'updated'
    fi
}

gpteasy__prepare_candidate() {
    local provider_id=$1 target_dir=${gpteasy__config_target%/*} block newline
    block=$(mktemp "$target_dir/.gpteasy-block.XXXXXX") || return
    gpteasy__candidate=$(mktemp "$target_dir/.config.toml.gpteasy.XXXXXX") || {
        rm -f -- "$block"
        return 1
    }
    if ! gpteasy__print_block "$provider_id" >"$block"; then
        rm -f -- "$block" "$gpteasy__candidate"
        return 1
    fi
    newline=lf
    if [[ -f "$gpteasy__config_target" ]] && awk 'index($0, "\r") { found = 1; exit } END { exit found ? 0 : 1 }' "$gpteasy__config_target"; then
        newline=crlf
        awk '{ sub(/\r$/, "", $0); printf "%s\r\n", $0 }' "$block" >"$block.crlf" || return
        mv -f -- "$block.crlf" "$block" || return
    fi
    if [[ "$gpteasy__starts" -eq 0 ]]; then
        cat -- "$block" >"$gpteasy__candidate" || return
        [[ ! -f "$gpteasy__config_target" ]] || cat -- "$gpteasy__config_target" >>"$gpteasy__candidate" || return
    else
        if ! awk -v start="$gpteasy__start_marker" -v end="$gpteasy__end_marker" -v block="$block" '
            { line = $0; sub(/\r$/, "", line) }
            line == start {
                while ((getline replacement < block) > 0) print replacement
                close(block)
                skipping = 1
                next
            }
            skipping && line == end { skipping = 0; next }
            !skipping { print }
            END { if (skipping) exit 42 }
        ' "$gpteasy__config_target" >"$gpteasy__candidate"; then
            rm -f -- "$block" "$gpteasy__candidate"
            return 1
        fi
    fi
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
    local stamp process_id
    process_id=$$
    stamp=$(date -u +%Y%m%dT%H%M%S%N)
    gpteasy__restore_point="$gpteasy__restore_root/switch-$stamp-$process_id-${RANDOM:-0}"
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
    local file=$1 owner mode links kind
    [[ -f "$file" && ! -L "$file" ]] || return 1
    read -r owner mode links kind < <(stat -c '%u %a %h %F' -- "$file") || return 1
    [[ "$owner" == "$(id -u)" && "${mode: -2}" == '00' && "$links" == 1 && "$kind" == 'regular file' ]]
}

gpteasy__owned_regular_file_is_safe() {
    local file=$1 owner links kind
    [[ -f "$file" && ! -L "$file" ]] || return 1
    read -r owner links kind < <(stat -c '%u %h %F' -- "$file") || return 1
    [[ "$owner" == "$(id -u)" && "$links" == 1 && "$kind" == 'regular file' ]]
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
    gpteasy__resolve_config_target || return
    gpteasy__inspect_writable_config || return
    gpteasy__prepare_candidate "$provider_id" || {
        gpteasy__cleanup_failed_apply
        return 1
    }
    gpteasy__create_restore_point || {
        gpteasy__cleanup_failed_apply
        return 1
    }
    gpteasy__install_credential "$provider_id" || {
        gpteasy__cleanup_failed_apply
        return 1
    }
    if ! gpteasy__config_target_unchanged; then
        printf '%s\n' 'Codex 配置在操作期间发生变化，已停止覆盖。' >&2
        gpteasy__cleanup_failed_apply
        return 1
    fi
    target_dir=${gpteasy__config_target%/*}
    if ! mv -f -- "$gpteasy__candidate" "$gpteasy__config_target"; then
        gpteasy__cleanup_failed_apply
        return 1
    fi
    gpteasy__candidate=
    if ! sync -f "$target_dir" 2>/dev/null || [[ "$(gpteasy__file_hash "$gpteasy__config_target")" != "$gpteasy__candidate_hash" ]]; then
        printf '%s\n' '配置替换后的复核失败，请使用 restore 检查最近恢复点。' >&2
        return 1
    fi
    gpteasy__prune_restore_points || return
    gpteasy__cleanup_credentials || return
    printf '已切换到：%s\n' "$(gpteasy__provider_name "$provider_id")"
}

gpteasy__switch_provider() {
    local provider_id=$1 result
    gpteasy__require_codex_version || return
    gpteasy__prepare_private_state || return
    gpteasy__acquire_lock switch || return
    gpteasy__apply_provider_locked "$provider_id"
    result=$?
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
    local latest kind expected_link current_label target_label choice candidate= candidate_hash= target_dir
    latest=$(find "$gpteasy__restore_root" -mindepth 1 -maxdepth 1 -type d -name 'switch-*' -print | sort -r | head -n 1)
    if [[ -z "$latest" ]]; then
        printf '%s\n' '没有可恢复的 Linux 恢复点。' >&2
        return 1
    fi
    if ! gpteasy__directory_is_owned "$latest" 1 || ! gpteasy__private_file_is_safe "$latest/config-kind"; then
        printf '%s\n' '最新 Linux 恢复点的权限或所有者不安全。' >&2
        return 1
    fi
    kind=$(cat -- "$latest/config-kind") || return
    case "$kind" in
        missing) ;;
        regular)
            gpteasy__private_file_is_safe "$latest/config.toml" || return 1
            ;;
        symlink)
            gpteasy__private_file_is_safe "$latest/config.toml" || return 1
            gpteasy__private_file_is_safe "$latest/symlink-target" || return 1
            ;;
        *)
            printf '%s\n' '最新 Linux 恢复点格式损坏。' >&2
            return 1
            ;;
    esac
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
    fi
    if ! gpteasy__config_target_unchanged; then
        printf '%s\n' 'Codex 配置在确认期间发生变化，恢复已停止。' >&2
        rm -f -- "$candidate" 2>/dev/null || true
        return 1
    fi
    if [[ "$kind" == missing ]]; then
        rm -f -- "$gpteasy__config_target" || return
        [[ ! -e "$gpteasy__config_target" ]] || return 1
    else
        mv -f -- "$candidate" "$gpteasy__config_target" || {
            rm -f -- "$candidate"
            return 1
        }
        [[ "$(gpteasy__file_hash "$gpteasy__config_target")" == "$candidate_hash" ]] || return 1
    fi
    sync -f "$target_dir" 2>/dev/null || return
    gpteasy__discard_restore_point "$latest" || return
    gpteasy__cleanup_credentials || return
    printf '%s\n' '已恢复最近一次 shell 切换前的配置。'
}

gpteasy__restore() {
    local result
    gpteasy__require_codex_version || return
    gpteasy__prepare_private_state || return
    gpteasy__acquire_lock restore || return
    gpteasy__restore_locked
    result=$?
    gpteasy__release_lock || {
        printf '%s\n' '恢复已处理，但 shell 锁释放失败；请检查 gpteasy unlock。' >&2
        return 1
    }
    return "$result"
}

gpteasy__info() {
    printf '目标环境：%s\n' "${CODEX_HOME:-"$HOME/.codex"}"
    printf 'Linux 导出 ID：%s\n' "$gpteasy__export_id"
    printf '%s\n' '管理区块 schema：1'
    printf '%s\n' 'Shell：{{GPTEASY_SHELL_LABEL}}'
    printf '供应商数量：%s\n' "$gpteasy__provider_count"
    printf '%s\n' 'Codex CLI 最低版本：0.147.0'
}

gpteasy__unlock() {
    local active owner_file owner token pid process_start operation actual_start choice
    local owner_hash owner_signature
    gpteasy__require_codex_version || return
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

gpteasy() {
{{GPTEASY_FUNCTION_OPTIONS}}
    local command=${1:-}
    case "$command" in
        help | --help | -h)
            gpteasy__help
            return
            ;;
    esac
    gpteasy__require_snapshot_private || return
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
