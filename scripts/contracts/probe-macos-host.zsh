#!/bin/zsh
set -euo pipefail

readonly EXIT_COMPLETED=0
readonly EXIT_ASSERTION_FAILED=2
readonly EXIT_STRICT_PREREQUISITE_BLOCKED=3
readonly EXIT_SECURITY_BOUNDARY_FAILED=5
readonly CANARY="GPTEASY-CONTRACT-CANARY-NONSECRET-01-12"
readonly BUNDLED_CODEX_RELATIVE_PATH="Contents/Resources/codex"

script_dir="${0:A:h}"
codex_probe="$script_dir/probe-codex-macos.zsh"
fixture_case=""
expected_version="0.146.1"
expected_arch=""
disposable_home="$HOME"
working_directory="$PWD"
official_cli=""
selected_bundle=""
canary_path=""
canary_digest=""
created_codex_directory=false

function usage() {
  print -u2 -- "usage: probe-macos-host.zsh [--fixture-case CASE] [--expected-version VERSION] [--expected-arch arm64|x86_64] [--disposable-home PATH] [--working-directory PATH] [--official-cli PATH] [--bundle PATH]"
}

function json_quote() {
  local value="$1"
  value=${value//\\/\\\\}
  value=${value//\"/\\\"}
  value=${value//$'\n'/\\n}
  value=${value//$'\r'/\\r}
  value=${value//$'\t'/\\t}
  print -nr -- "\"${value}\""
}

function sha256_file() {
  /usr/bin/shasum -a 256 "$1" | /usr/bin/awk '{print $1}'
}

function canonical_directory() {
  local path="$1"
  [[ -d "$path" ]] || return 1
  (
    cd -P -- "$path"
    pwd
  )
}

function is_native_macos_14_or_newer() {
  [[ "$os_name" == "Darwin" ]] || return 1
  [[ -n "$os_major" && "$os_major" != *[^0-9]* ]] || return 1
  (( os_major >= 14 ))
}

function cleanup_canary_fallback() {
  if [[ -n "$canary_path" &&
        -n "$canary_digest" &&
        -f "$canary_path" ]]; then
    local current_digest=""
    current_digest="$(sha256_file "$canary_path" 2>/dev/null || true)"
    if [[ "$current_digest" == "$canary_digest" ]]; then
      rm -f -- "$canary_path"
    fi
  fi
  if [[ "$created_codex_directory" == true &&
        -n "$canary_path" &&
        -d "${canary_path:h}" ]]; then
    rmdir -- "${canary_path:h}" 2>/dev/null || true
  fi
}

trap cleanup_canary_fallback EXIT INT TERM

while (( $# > 0 )); do
  case "$1" in
    --fixture-case)
      fixture_case="${2:-}"
      shift 2
      ;;
    --expected-version)
      expected_version="${2:-}"
      shift 2
      ;;
    --expected-arch)
      expected_arch="${2:-}"
      shift 2
      ;;
    --disposable-home)
      disposable_home="${2:-}"
      shift 2
      ;;
    --working-directory)
      working_directory="${2:-}"
      shift 2
      ;;
    --official-cli)
      official_cli="${2:-}"
      shift 2
      ;;
    --bundle)
      selected_bundle="${2:-}"
      shift 2
      ;;
    *)
      usage
      exit "$EXIT_ASSERTION_FAILED"
      ;;
  esac
done

if [[ ! -f "$codex_probe" ]]; then
  print -r -- '{"schema_version":1,"probe":"macos-host-codex-parity","outcome":"blocked","exit_code":3,"strict_gate_eligible":false,"test_only":false,"expected_version":"0.146.1","host_identity":null,"official_cli":null,"bundled_host":null,"parity":{"version":false,"config_root":false,"model_digest":false,"provider_digest":false,"origin_digest":false,"credential_carrier":false,"shared_user_layer":false,"all":false},"checks":[{"name":"probe_execution","outcome":"blocked","code":"MACOS_CODEX_PROBE_MISSING"}],"blocking_reasons":["MACOS_CODEX_PROBE_MISSING"]}'
  exit "$EXIT_STRICT_PREREQUISITE_BLOCKED"
fi

if [[ -z "$expected_arch" ]]; then
  expected_arch="$(uname -m 2>/dev/null || print unknown)"
fi
if [[ "$expected_arch" != "arm64" && "$expected_arch" != "x86_64" ]]; then
  usage
  exit "$EXIT_ASSERTION_FAILED"
fi

typeset test_only=false
typeset os_name=""
typeset os_major="0"
typeset observed_arch=""
typeset bundle_name=""
typeset bundle_id=""
typeset install_root_category="unknown"
typeset executable_present=false
typeset bundled_executable=""
typeset host_identity_allowlisted=false
typeset canary_cleanup=true
typeset execution_error=""

function load_fixture_host() {
  test_only=true
  os_name="Darwin"
  os_major="14"
  observed_arch="$expected_arch"
  bundle_name="Codex.app"
  bundle_id="com.openai.codex.fixture"
  install_root_category="system_applications"
  executable_present=true
  host_identity_allowlisted=true
  official_cli="/fixture/bin/codex"
  bundled_executable="/fixture/Codex.app/Contents/Resources/codex"

  case "$fixture_case" in
    positive|root_mismatch|origin_mismatch|provider_mismatch|carrier_mismatch)
      ;;
    host_missing)
      executable_present=false
      bundled_executable=""
      ;;
    wrong_arch)
      if [[ "$expected_arch" == "arm64" ]]; then
        observed_arch="x86_64"
      else
        observed_arch="arm64"
      fi
      ;;
    *)
      execution_error="fixture case does not exist"
      ;;
  esac
}

function discover_live_host() {
  os_name="$(uname -s)"
  observed_arch="$(uname -m)"
  if [[ "$os_name" == "Darwin" ]]; then
    os_major="$(/usr/bin/sw_vers -productVersion | /usr/bin/awk -F. '{print $1}')"
  fi

  if [[ -z "$official_cli" ]]; then
    official_cli="$(command -v codex 2>/dev/null || true)"
  fi

  typeset -a bundle_candidates
  bundle_candidates=(
    "/Applications/Codex.app"
    "/Applications/ChatGPT.app"
    "$HOME/Applications/Codex.app"
    "$HOME/Applications/ChatGPT.app"
  )
  if [[ -n "$selected_bundle" ]]; then
    bundle_candidates=("$selected_bundle" "${bundle_candidates[@]}")
  fi

  local bundle=""
  for bundle in "${bundle_candidates[@]}"; do
    if [[ -x "$bundle/$BUNDLED_CODEX_RELATIVE_PATH" ]]; then
      selected_bundle="$bundle"
      break
    fi
  done

  if [[ -z "$selected_bundle" ||
        ! -x "$selected_bundle/$BUNDLED_CODEX_RELATIVE_PATH" ]]; then
    return
  fi

  bundled_executable="$selected_bundle/$BUNDLED_CODEX_RELATIVE_PATH"
  executable_present=true
  bundle_name="${selected_bundle:t}"
  case "$selected_bundle" in
    /Applications/*)
      install_root_category="system_applications"
      ;;
    "$HOME/Applications/"*)
      install_root_category="current_user"
      ;;
    *)
      install_root_category="other_app_bundle"
      ;;
  esac
  if [[ -f "$selected_bundle/Contents/Info.plist" ]]; then
    bundle_id=$(
      /usr/libexec/PlistBuddy \
        -c 'Print :CFBundleIdentifier' \
        "$selected_bundle/Contents/Info.plist" \
        2>/dev/null || true
    )
  fi
  if [[ "$bundle_name" == "Codex.app" || "$bundle_name" == "ChatGPT.app" ]]; then
    host_identity_allowlisted=true
  fi
}

function write_canary() {
  local canonical_home=""
  local canonical_current_home=""
  canonical_home="$(canonical_directory "$disposable_home")" || return 1
  canonical_current_home="$(canonical_directory "$HOME")" || return 1
  [[ "$canonical_home" == "$canonical_current_home" ]] || return 1
  [[ -d "$working_directory" ]] || return 1

  local codex_directory="$canonical_home/.codex"
  if [[ -e "$codex_directory/config.toml" ]]; then
    return 1
  fi
  if [[ ! -d "$codex_directory" ]]; then
    mkdir -m 700 -- "$codex_directory"
    created_codex_directory=true
  fi

  canary_path="$codex_directory/config.toml"
  umask 077
  cat > "$canary_path" <<EOF
model = "gpteasy-contract-model-01-12"
model_provider = "gpteasy_contract"

[model_providers.gpteasy_contract]
name = "GPTEasy Contract Canary"
base_url = "https://127.0.0.1.invalid/v1"
wire_api = "responses"
requires_openai_auth = false
supports_websockets = false
experimental_bearer_token = "$CANARY"
EOF
  canary_digest="$(sha256_file "$canary_path")"
}

function remove_canary() {
  [[ -n "$canary_path" && -f "$canary_path" ]] || return 1
  local codex_directory="${canary_path:h}"
  local current_digest=""
  current_digest="$(sha256_file "$canary_path")" || return 1
  [[ "$current_digest" == "$canary_digest" ]] || return 1
  rm -f -- "$canary_path"
  canary_path=""
  canary_digest=""
  if [[ "$created_codex_directory" == true ]]; then
    rmdir -- "$codex_directory" 2>/dev/null || true
    created_codex_directory=false
  fi
}

function probe_summary_json() {
  local line="$1"
  local probe_outcome=""
  local probe_exit=""
  local probe_strict=""
  local probe_test=""
  local probe_version=""
  local probe_binary=""
  local probe_schema=""
  local probe_root=""
  local probe_initialize=""
  local probe_initialized=""
  local probe_read=""
  local probe_layers=""
  local probe_model=""
  local probe_provider=""
  local probe_origin=""
  local probe_env=""
  local probe_direct=""
  local probe_missing=""
  local probe_shared=""
  IFS=$'\t' read -r \
    probe_outcome \
    probe_exit \
    probe_strict \
    probe_test \
    probe_version \
    probe_binary \
    probe_schema \
    probe_root \
    probe_initialize \
    probe_initialized \
    probe_read \
    probe_layers \
    probe_model \
    probe_provider \
    probe_origin \
    probe_env \
    probe_direct \
    probe_missing \
    probe_shared <<< "$line"

  print -nr -- "{\"outcome\":$(json_quote "$probe_outcome"),\"exit_code\":${probe_exit:-3},\"version\":$(json_quote "$probe_version"),\"binary_sha256\":$(json_quote "$probe_binary"),\"schema_sha256\":$(json_quote "$probe_schema"),\"config_root_category\":$(json_quote "$probe_root"),\"protocol\":{\"initialize\":${probe_initialize:-false},\"initialized\":${probe_initialized:-false},\"config_read\":${probe_read:-false},\"include_layers\":${probe_layers:-false}},\"model_sha256\":$(json_quote "$probe_model"),\"provider_sha256\":$(json_quote "$probe_provider"),\"origin_sha256\":$(json_quote "$probe_origin"),\"credential_carrier\":{\"env_key\":${probe_env:-false},\"direct_bearer\":${probe_direct:-false},\"missing\":${probe_missing:-true}},\"shared_user_layer\":${probe_shared:-false}}"
}

if [[ -n "$fixture_case" ]]; then
  load_fixture_host
else
  discover_live_host
fi

typeset cli_output=""
typeset bundled_output=""
typeset cli_exit="$EXIT_STRICT_PREREQUISITE_BLOCKED"
typeset bundled_exit="$EXIT_STRICT_PREREQUISITE_BLOCKED"

if [[ "$executable_present" == true &&
      "$host_identity_allowlisted" == true &&
      -z "$execution_error" ]]; then
  typeset child_fixture="positive"
  if [[ "$test_only" == true ]]; then
    case "$fixture_case" in
      root_mismatch|origin_mismatch|provider_mismatch|carrier_mismatch)
        child_fixture="$fixture_case"
        ;;
    esac
  else
    if ! write_canary; then
      execution_error="disposable default user config cannot be prepared"
    fi
  fi

  if [[ -z "$execution_error" ]]; then
    typeset -a common_arguments
    common_arguments=(
      --disposable-home "$disposable_home"
      --working-directory "$working_directory"
      --expected-version "$expected_version"
      --expected-arch "$expected_arch"
      --output-format tsv
    )
    if [[ "$test_only" == true ]]; then
      common_arguments+=(--fixture-case positive)
    fi

    cli_exit=0
    cli_output=$(
      zsh "$codex_probe" \
        --role official_cli \
        --codex-executable "$official_cli" \
        "${common_arguments[@]}"
    ) || cli_exit="$?"

    if [[ "$test_only" == true ]]; then
      common_arguments[-1]="$child_fixture"
    fi
    bundled_exit=0
    bundled_output=$(
      zsh "$codex_probe" \
        --role bundled_host \
        --codex-executable "$bundled_executable" \
        "${common_arguments[@]}"
    ) || bundled_exit="$?"

    if [[ "$test_only" == false ]] && ! remove_canary; then
      canary_cleanup=false
    fi
  fi
fi

typeset cli_outcome=""
typeset cli_strict=""
typeset cli_test=""
typeset cli_version=""
typeset cli_binary=""
typeset cli_schema=""
typeset cli_root=""
typeset cli_initialize=""
typeset cli_initialized=""
typeset cli_read=""
typeset cli_layers=""
typeset cli_model=""
typeset cli_provider=""
typeset cli_origin=""
typeset cli_env=""
typeset cli_direct=""
typeset cli_missing=""
typeset cli_shared=""
typeset host_outcome=""
typeset host_strict=""
typeset host_test=""
typeset host_version=""
typeset host_binary=""
typeset host_schema=""
typeset host_root=""
typeset host_initialize=""
typeset host_initialized=""
typeset host_read=""
typeset host_layers=""
typeset host_model=""
typeset host_provider=""
typeset host_origin=""
typeset host_env=""
typeset host_direct=""
typeset host_missing=""
typeset host_shared=""

if [[ -n "$cli_output" ]]; then
  IFS=$'\t' read -r \
    cli_outcome cli_exit cli_strict cli_test cli_version cli_binary cli_schema \
    cli_root cli_initialize cli_initialized cli_read cli_layers cli_model \
    cli_provider cli_origin cli_env cli_direct cli_missing cli_shared <<< "$cli_output"
fi
if [[ -n "$bundled_output" ]]; then
  IFS=$'\t' read -r \
    host_outcome bundled_exit host_strict host_test host_version host_binary host_schema \
    host_root host_initialize host_initialized host_read host_layers host_model \
    host_provider host_origin host_env host_direct host_missing host_shared <<< "$bundled_output"
fi

typeset parity_version=false
typeset parity_config_root=false
typeset parity_model_digest=false
typeset parity_provider_digest=false
typeset parity_origin_digest=false
typeset parity_credential_carrier=false
typeset parity_shared_user_layer=false
typeset parity_all=false

[[ -n "$cli_version" &&
   "$cli_version" == "$host_version" &&
   "$cli_version" == "$expected_version" ]] && parity_version=true
[[ "$cli_root" == "default_user" &&
   "$cli_root" == "$host_root" ]] && parity_config_root=true
[[ -n "$cli_model" && "$cli_model" == "$host_model" ]] && parity_model_digest=true
[[ -n "$cli_provider" && "$cli_provider" == "$host_provider" ]] && parity_provider_digest=true
[[ -n "$cli_origin" && "$cli_origin" == "$host_origin" ]] && parity_origin_digest=true
[[ "$cli_env" == "$host_env" &&
   "$cli_direct" == "$host_direct" &&
   "$cli_missing" == "$host_missing" ]] && parity_credential_carrier=true
[[ "$cli_shared" == true && "$host_shared" == true ]] && parity_shared_user_layer=true

typeset native_macos=false
typeset native_arch=false
is_native_macos_14_or_newer && native_macos=true
[[ "$observed_arch" == "$expected_arch" ]] && native_arch=true

if [[ "$native_macos" == true &&
      "$native_arch" == true &&
      "$host_identity_allowlisted" == true &&
      "$executable_present" == true &&
      "$parity_version" == true &&
      "$parity_config_root" == true &&
      "$parity_model_digest" == true &&
      "$parity_provider_digest" == true &&
      "$parity_origin_digest" == true &&
      "$parity_credential_carrier" == true &&
      "$parity_shared_user_layer" == true &&
      "$cli_exit" -eq 0 &&
      "$bundled_exit" -eq 0 &&
      "$canary_cleanup" == true &&
      -z "$execution_error" ]]; then
  parity_all=true
fi

typeset -a checks
typeset -a blocking_reasons
checks=()
blocking_reasons=()

function add_check() {
  local name="$1"
  local passed="$2"
  local failure_code="$3"
  local failed_outcome="${4:-failed}"
  if [[ "$passed" == true ]]; then
    checks+=("{\"name\":$(json_quote "$name"),\"outcome\":\"passed\",\"code\":\"OK\"}")
  else
    checks+=("{\"name\":$(json_quote "$name"),\"outcome\":$(json_quote "$failed_outcome"),\"code\":$(json_quote "$failure_code")}")
    blocking_reasons+=("$failure_code")
  fi
}

add_check "native_macos" "$native_macos" "MACOS_NATIVE_HOST_REQUIRED"
add_check "native_arch" "$native_arch" "MACOS_ARCH_MISMATCH"
add_check "host_bundle_allowlisted" "$host_identity_allowlisted" "HOST_BUNDLE_IDENTITY_MISMATCH"
add_check "host_executable_present" "$executable_present" "HOST_CODEX_MISSING" "blocked"
add_check "host_cli_parity" "$parity_all" "HOST_CLI_PARITY_MISMATCH"
add_check "canary_cleanup" "$canary_cleanup" "CANARY_CLEANUP_UNPROVEN"
if [[ -n "$execution_error" ]]; then
  add_check "probe_execution" false "MACOS_HOST_PROBE_UNAVAILABLE" "blocked"
fi

typeset exit_code="$EXIT_COMPLETED"
typeset outcome="passed"
if [[ "$executable_present" == false ||
      "$cli_exit" -eq "$EXIT_STRICT_PREREQUISITE_BLOCKED" ||
      "$bundled_exit" -eq "$EXIT_STRICT_PREREQUISITE_BLOCKED" ||
      -n "$execution_error" ]]; then
  exit_code="$EXIT_STRICT_PREREQUISITE_BLOCKED"
  outcome="blocked"
elif [[ "$parity_all" == false ]]; then
  exit_code="$EXIT_SECURITY_BOUNDARY_FAILED"
  outcome="failed"
fi

typeset strict_gate_eligible=false
if [[ "$exit_code" -eq 0 && "$test_only" == false ]]; then
  strict_gate_eligible=true
fi

typeset cli_json="null"
typeset bundled_json="null"
[[ -n "$cli_output" ]] && cli_json="$(probe_summary_json "$cli_output")"
[[ -n "$bundled_output" ]] && bundled_json="$(probe_summary_json "$bundled_output")"
typeset checks_json="${(j:,:)checks}"
typeset reasons_json=""
if (( ${#blocking_reasons} > 0 )); then
  typeset -a quoted_reasons
  quoted_reasons=()
  typeset reason=""
  for reason in "${blocking_reasons[@]}"; do
    quoted_reasons+=("$(json_quote "$reason")")
  done
  reasons_json="${(j:,:)quoted_reasons}"
fi

print -r -- "{\"schema_version\":1,\"probe\":\"macos-host-codex-parity\",\"outcome\":$(json_quote "$outcome"),\"exit_code\":${exit_code},\"strict_gate_eligible\":${strict_gate_eligible},\"test_only\":${test_only},\"expected_version\":$(json_quote "$expected_version"),\"host_identity\":{\"bundle_name\":$(json_quote "$bundle_name"),\"bundle_id\":$(json_quote "$bundle_id"),\"install_root_category\":$(json_quote "$install_root_category"),\"bundled_relative_path\":$(json_quote "$BUNDLED_CODEX_RELATIVE_PATH"),\"executable_present\":${executable_present}},\"official_cli\":${cli_json},\"bundled_host\":${bundled_json},\"parity\":{\"version\":${parity_version},\"config_root\":${parity_config_root},\"model_digest\":${parity_model_digest},\"provider_digest\":${parity_provider_digest},\"origin_digest\":${parity_origin_digest},\"credential_carrier\":${parity_credential_carrier},\"shared_user_layer\":${parity_shared_user_layer},\"all\":${parity_all}},\"checks\":[${checks_json}],\"blocking_reasons\":[${reasons_json}]}"

exit "$exit_code"
