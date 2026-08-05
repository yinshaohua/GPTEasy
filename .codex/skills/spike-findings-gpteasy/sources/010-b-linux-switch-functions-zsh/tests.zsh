#!/usr/bin/env zsh
emulate -R zsh
setopt errexit nounset pipefail

script_path=${(%):-%N}
script_dir=$(cd -- "$(dirname -- "$script_path")" && pwd)
run_dir="${TMPDIR:-/tmp}/gpteasy-spike-010b-${UID:-0}-$$/test workspace"
summary="$script_dir/.run/summary.json"
mkdir -p -- "$script_dir/.run"
rm -rf -- "$run_dir"
mkdir -p -- "$run_dir"

passed=0
total=0
names=()

record() {
    local name=$1
    shift
    total=$((total + 1))
    names+=("$name")
    if "$@"; then
        passed=$((passed + 1))
        printf 'PASS %s\n' "$name"
    else
        printf 'FAIL %s\n' "$name" >&2
    fi
}

fingerprint() {
    if [[ -f "$1" ]]; then cksum <"$1"; else printf 'missing\n'; fi
}

outside_block() {
    awk -v start='# >>> GPTEasy managed provider >>>' -v end='# <<< GPTEasy managed provider <<<' '
        $0 == start { skipping = 1; next }
        skipping && $0 == end { skipping = 0; next }
        !skipping { print }
    ' "$1"
}

export GPTEASY_CODEX_HOME="$run_dir/codex home"
config="$GPTEASY_CODEX_HOME/config.toml"
mkdir -p -- "$GPTEASY_CODEX_HOME"
printf '# user config\ncustom_flag = true\n\n[projects.demo]\ntrust_level = "trusted"\n' >"$config"
chmod 640 "$config"

before=$(fingerprint "$config")
source "$script_dir/gpteasy-switch.zsh"
after=$(fingerprint "$config")
record "source-has-no-side-effects" test "$before" = "$after"

before=$(fingerprint "$config")
gpteasy_select_provider <<<"q" >/dev/null
after=$(fingerprint "$config")
record "cancel-does-not-write" test "$before" = "$after"

gpteasy_select_provider <<<"1" >/dev/null
record "select-alpha-establishes-managed-block" grep -Fq '# GPTEasy provider-id: provider-alpha' "$config"
record "original-mode-is-preserved" test "$(stat -c %a "$config")" = "640"

outside_before="$run_dir/outside-before"
outside_after="$run_dir/outside-after"
outside_block "$config" >"$outside_before"
gpteasy_select_provider <<<"2" >/dev/null
outside_block "$config" >"$outside_after"
record "subsequent-switch-preserves-outside-bytes" cmp -s "$outside_before" "$outside_after"
record "special-characters-are-preescaped" grep -Fq 'experimental_bearer_token = "fake-$-quote-\"-slash-\\-unicode-密钥"' "$config"
record "current-provider-is-readable" test "$(gpteasy_current_provider)" = "provider-beta"

cp -- "$config" "$run_dir/valid"
printf '# >>> GPTEasy managed provider >>>\nmodel = "broken"\n' >"$config"
before=$(fingerprint "$config")
if gpteasy__apply_provider provider-alpha >/dev/null 2>&1; then damaged_ok=false; else damaged_ok=true; fi
after=$(fingerprint "$config")
record "damaged-marker-stops-without-write" test "$damaged_ok" = true -a "$before" = "$after"

printf 'model = "external"\nmodel_provider = "external"\n' >"$config"
before=$(fingerprint "$config")
if gpteasy__apply_provider provider-alpha >/dev/null 2>&1; then conflict_ok=false; else conflict_ok=true; fi
after=$(fingerprint "$config")
record "unmanaged-provider-keys-require-migration" test "$conflict_ok" = true -a "$before" = "$after"

printf 'custom_flag = true\n' >"$config"
for _ in 1 2 3 4 5 6 7; do
    gpteasy__apply_provider provider-alpha >/dev/null
    gpteasy__apply_provider provider-beta >/dev/null
done
backup_count=$(find "$GPTEASY_CODEX_HOME/.gpteasy-backups" -maxdepth 1 -type f -name 'config-*.toml' | wc -l)
record "only-five-backups-are-retained" test "$backup_count" -eq 5

gpteasy__apply_provider provider-alpha >/dev/null
cp -- "$config" "$run_dir/restore-expected"
gpteasy__apply_provider provider-beta >/dev/null
gpteasy_restore_latest
record "restore-latest-is-atomic-and-exact" cmp -s "$config" "$run_dir/restore-expected"

if grep -Eq '(^|[;&|[:space:]])(python3?|node|jq|perl|ruby)([;&|[:space:]]|$)' "$script_dir/gpteasy-switch.zsh"; then
    no_runtime=false
else
    no_runtime=true
fi
record "no-extra-runtime-dependencies" test "$no_runtime" = true

{
    printf '{\n  "shell": "zsh",\n  "version": "%s",\n' "${ZSH_VERSION//\"/\\\"}"
    printf '  "passed": %d,\n  "total": %d,\n  "tests": [\n' "$passed" "$total"
    for ((i = 1; i <= ${#names[@]}; i++)); do
        comma=','
        [[ $i -eq ${#names[@]} ]] && comma=''
        printf '    "%s"%s\n' "${names[$i]}" "$comma"
    done
    printf '  ]\n}\n'
} >"$summary"

if [[ "$passed" -eq "$total" ]]; then
    rm -rf -- "${run_dir%/test workspace}"
    exit 0
fi
exit 1
