# GPTEasy standalone provider switch functions for Bash 4+.
# This file intentionally contains plaintext provider credentials.

gpteasy__start_marker='# >>> GPTEasy managed provider >>>'
gpteasy__end_marker='# <<< GPTEasy managed provider <<<'
gpteasy__id_prefix='# GPTEasy provider-id:'

gpteasy__config_path() {
    printf '%s\n' "${GPTEASY_CODEX_HOME:-"$HOME/.codex"}/config.toml"
}

gpteasy__provider_id() {
    case "$1" in
        1) printf '%s\n' 'provider-alpha' ;;
        2) printf '%s\n' 'provider-beta' ;;
        *) return 1 ;;
    esac
}

gpteasy__provider_name() {
    case "$1" in
        provider-alpha) printf '%s\n' 'Alpha Provider' ;;
        provider-beta) printf '%s\n' '测试供应商 Beta "Quoted"' ;;
        *) return 1 ;;
    esac
}

gpteasy__print_block() {
    case "$1" in
        provider-alpha)
            cat <<'GPTEASY_BLOCK'
# >>> GPTEasy managed provider >>>
# GPTEasy provider-id: provider-alpha
model = "alpha-model"
model_provider = "gpteasy"
model_providers.gpteasy.name = "Alpha Provider"
model_providers.gpteasy.base_url = "https://alpha.example/v1"
model_providers.gpteasy.wire_api = "responses"
model_providers.gpteasy.experimental_bearer_token = "fake-alpha-key"
# <<< GPTEasy managed provider <<<
GPTEASY_BLOCK
            ;;
        provider-beta)
            cat <<'GPTEASY_BLOCK'
# >>> GPTEasy managed provider >>>
# GPTEasy provider-id: provider-beta
model = "beta-model"
model_provider = "gpteasy"
model_providers.gpteasy.name = "测试供应商 Beta \"Quoted\""
model_providers.gpteasy.base_url = "https://beta.example/v1"
model_providers.gpteasy.wire_api = "responses"
model_providers.gpteasy.experimental_bearer_token = "fake-$-quote-\"-slash-\\-unicode-密钥"
# <<< GPTEasy managed provider <<<
GPTEASY_BLOCK
            ;;
        *)
            printf '未知供应商 ID：%s\n' "$1" >&2
            return 1
            ;;
    esac
}

gpteasy__marker_info() {
    local config=$1
    if [[ ! -f "$config" ]]; then
        printf '0 0 0 0\n'
        return
    fi
    awk -v start="$gpteasy__start_marker" -v end="$gpteasy__end_marker" '
        $0 == start { start_count += 1; if (start_line == 0) start_line = NR }
        $0 == end { end_count += 1; if (end_line == 0) end_line = NR }
        END { print start_count + 0, end_count + 0, start_line + 0, end_line + 0 }
    ' "$config"
}

gpteasy__has_unmanaged_conflict() {
    local config=$1
    [[ -f "$config" ]] || return 1
    awk '
        BEGIN { in_table = 0; conflict = 0 }
        /^[[:space:]]*\[/ { in_table = 1 }
        !in_table && /^[[:space:]]*(model|model_provider)[[:space:]]*=/ { conflict = 1 }
        /^[[:space:]]*\[model_providers\.gpteasy\][[:space:]]*$/ { conflict = 1 }
        /^[[:space:]]*model_providers\.gpteasy\./ { conflict = 1 }
        END { exit conflict ? 0 : 1 }
    ' "$config"
}

gpteasy__fingerprint() {
    local config=$1
    if [[ -f "$config" ]]; then
        cksum <"$config"
    else
        printf '%s\n' 'missing'
    fi
}

gpteasy__prune_backups() {
    local backup_dir=$1
    ls -1 "$backup_dir"/config-*.toml 2>/dev/null |
        sort -r |
        awk 'NR > 5' |
        while IFS= read -r old_backup; do
            rm -f -- "$old_backup"
        done
}

gpteasy__apply_provider() {
    local provider_id=$1
    local config config_dir marker_info start_count end_count start_line end_line
    local block candidate original_fingerprint backup_dir backup stamp

    config=$(gpteasy__config_path) || return
    config_dir=${config%/*}
    mkdir -p -- "$config_dir" || return
    original_fingerprint=$(gpteasy__fingerprint "$config") || return

    marker_info=$(gpteasy__marker_info "$config") || return
    read -r start_count end_count start_line end_line <<<"$marker_info"
    if [[ "$start_count" -eq 0 && "$end_count" -eq 0 ]]; then
        if gpteasy__has_unmanaged_conflict "$config"; then
            printf '检测到管理区块外的供应商键，需要先由 GPTEasy 完成结构化迁移。\n' >&2
            return 2
        fi
    elif [[ "$start_count" -ne 1 || "$end_count" -ne 1 || "$start_line" -ge "$end_line" ]]; then
        printf 'GPTEasy 管理区块标记缺失、重复或倒置，已停止修改。\n' >&2
        return 2
    fi

    block=$(mktemp "$config_dir/.gpteasy-block.XXXXXX") || return
    candidate=$(mktemp "$config_dir/.config.toml.gpteasy.XXXXXX") || {
        rm -f -- "$block"
        return 1
    }
    if ! gpteasy__print_block "$provider_id" >"$block"; then
        rm -f -- "$block" "$candidate"
        return 1
    fi

    if [[ "$start_count" -eq 0 ]]; then
        cat "$block" >"$candidate" || {
            rm -f -- "$block" "$candidate"
            return 1
        }
        [[ ! -f "$config" ]] || cat "$config" >>"$candidate" || {
            rm -f -- "$block" "$candidate"
            return 1
        }
    else
        if ! awk -v start="$gpteasy__start_marker" -v end="$gpteasy__end_marker" -v block="$block" '
            $0 == start {
                while ((getline line < block) > 0) print line
                close(block)
                skipping = 1
                next
            }
            skipping && $0 == end { skipping = 0; next }
            !skipping { print }
            END { if (skipping) exit 42 }
        ' "$config" >"$candidate"; then
            rm -f -- "$block" "$candidate"
            return 2
        fi
    fi
    rm -f -- "$block"

    backup_dir="$config_dir/.gpteasy-backups"
    mkdir -p -- "$backup_dir" || {
        rm -f -- "$candidate"
        return 1
    }
    stamp=$(date -u +%Y%m%dT%H%M%S%N)
    backup="$backup_dir/config-$stamp-$$-${RANDOM:-0}.toml"
    if [[ -f "$config" ]]; then
        cp -p -- "$config" "$backup" || {
            rm -f -- "$candidate"
            return 1
        }
        chmod --reference="$config" "$candidate" 2>/dev/null || chmod 600 "$candidate"
    else
        : >"$backup" || {
            rm -f -- "$candidate"
            return 1
        }
        chmod 600 "$backup" "$candidate"
    fi
    gpteasy__prune_backups "$backup_dir"

    if [[ "$(gpteasy__fingerprint "$config")" != "$original_fingerprint" ]]; then
        printf 'Codex 配置在切换过程中被外部修改，已停止覆盖。\n' >&2
        rm -f -- "$candidate"
        return 3
    fi
    mv -f -- "$candidate" "$config" || {
        rm -f -- "$candidate"
        return 1
    }
    printf '已切换到：%s\n' "$(gpteasy__provider_name "$provider_id")"
}

gpteasy_current_provider() {
    local config
    config=$(gpteasy__config_path) || return
    [[ -f "$config" ]] || return 1
    awk -v start="$gpteasy__start_marker" -v end="$gpteasy__end_marker" -v prefix="$gpteasy__id_prefix" '
        $0 == start { inside = 1; next }
        inside && $0 == end { exit }
        inside && index($0, prefix) == 1 {
            sub("^" prefix "[[:space:]]*", "", $0)
            print
            exit
        }
    ' "$config"
}

gpteasy_restore_latest() {
    local config config_dir backup_dir latest candidate
    config=$(gpteasy__config_path) || return
    config_dir=${config%/*}
    backup_dir="$config_dir/.gpteasy-backups"
    latest=$(ls -1 "$backup_dir"/config-*.toml 2>/dev/null | sort -r | head -n 1)
    if [[ -z "$latest" ]]; then
        printf '没有可恢复的 GPTEasy 配置备份。\n' >&2
        return 1
    fi
    candidate=$(mktemp "$config_dir/.config.toml.gpteasy-restore.XXXXXX") || return
    cat "$latest" >"$candidate" || {
        rm -f -- "$candidate"
        return 1
    }
    [[ ! -f "$config" ]] || chmod --reference="$config" "$candidate" 2>/dev/null || true
    mv -f -- "$candidate" "$config"
}

gpteasy_select_provider() {
    local current choice provider_id
    current=$(gpteasy_current_provider 2>/dev/null || true)
    printf '可用供应商：\n'
    printf '  1) Alpha Provider%s\n' "$([[ "$current" == provider-alpha ]] && printf ' [当前]')"
    printf '  2) 测试供应商 Beta "Quoted"%s\n' "$([[ "$current" == provider-beta ]] && printf ' [当前]')"
    read -r -p '请选择供应商编号，或输入 q 取消：' choice
    case "$choice" in
        '' | q | Q)
            printf '已取消，不修改配置。\n'
            return 0
            ;;
        1 | 2)
            provider_id=$(gpteasy__provider_id "$choice") || return
            gpteasy__apply_provider "$provider_id"
            ;;
        *)
            printf '无效选择：%s\n' "$choice" >&2
            return 2
            ;;
    esac
}
