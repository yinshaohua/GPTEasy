#!/bin/zsh
set -euo pipefail

readonly EXIT_COMPLETED=0
readonly EXIT_ASSERTION_FAILED=2
readonly EXIT_STRICT_PREREQUISITE_BLOCKED=3
readonly EXIT_SECURITY_BOUNDARY_FAILED=5
readonly EXPECTED_MODEL="gpteasy-contract-model-01-12"
readonly EXPECTED_PROVIDER="gpteasy_contract"

role=""
disposable_home=""
working_directory=""
codex_executable=""
fixture_case=""
expected_version="0.146.1"
expected_arch=""
output_format="json"
coproc_pid=""
temporary_root=""

function usage() {
  print -u2 -- "usage: probe-codex-macos.zsh --role official_cli|bundled_host --disposable-home PATH --working-directory PATH [--codex-executable PATH] [--fixture-case CASE] [--expected-version VERSION] [--expected-arch arm64|x86_64] [--output-format json|tsv]"
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

function json_nullable_string() {
  local value="$1"
  if [[ -z "$value" ]]; then
    print -nr -- "null"
  else
    json_quote "$value"
  fi
}

function sha256_string() {
  print -rn -- "$1" | /usr/bin/shasum -a 256 | /usr/bin/awk '{print $1}'
}

function sha256_file() {
  /usr/bin/shasum -a 256 "$1" | /usr/bin/awk '{print $1}'
}

function is_sha256() {
  [[ ${#1} -eq 64 && "$1" != *[^0-9a-f]* ]]
}

function is_native_macos_14_or_newer() {
  [[ "$os_name" == "Darwin" ]] || return 1
  [[ -n "$os_major" && "$os_major" != *[^0-9]* ]] || return 1
  (( os_major >= 14 ))
}

function canonical_directory() {
  local path="$1"
  [[ -d "$path" ]] || return 1
  (
    cd -P -- "$path"
    pwd
  )
}

function cleanup() {
  if [[ -n "$coproc_pid" ]] && kill -0 "$coproc_pid" 2>/dev/null; then
    kill "$coproc_pid" 2>/dev/null || true
    wait "$coproc_pid" 2>/dev/null || true
  fi
  if [[ -n "$temporary_root" && -d "$temporary_root" ]]; then
    case "$temporary_root" in
      "${TMPDIR:-/tmp}"/gpteasy-codex-macos-probe.*)
        rm -rf -- "$temporary_root"
        ;;
    esac
  fi
}

trap cleanup EXIT INT TERM

while (( $# > 0 )); do
  case "$1" in
    --role)
      role="${2:-}"
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
    --codex-executable)
      codex_executable="${2:-}"
      shift 2
      ;;
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
    --output-format)
      output_format="${2:-}"
      shift 2
      ;;
    *)
      usage
      exit "$EXIT_ASSERTION_FAILED"
      ;;
  esac
done

if [[ "$role" != "official_cli" && "$role" != "bundled_host" ]]; then
  usage
  exit "$EXIT_ASSERTION_FAILED"
fi
if [[ "$output_format" != "json" && "$output_format" != "tsv" ]]; then
  usage
  exit "$EXIT_ASSERTION_FAILED"
fi
if [[ -z "$expected_arch" ]]; then
  expected_arch="$(uname -m 2>/dev/null || print unknown)"
fi
if [[ "$expected_arch" != "arm64" && "$expected_arch" != "x86_64" ]]; then
  usage
  exit "$EXIT_ASSERTION_FAILED"
fi

typeset os_name=""
typeset os_major="0"
typeset observed_arch=""
typeset executable_present=false
typeset version=""
typeset binary_sha256=""
typeset schema_sha256=""
typeset config_root_category="unknown"
typeset protocol_initialize=false
typeset protocol_initialized=false
typeset protocol_config_read=false
typeset protocol_include_layers=false
typeset model_sha256=""
typeset provider_sha256=""
typeset origin_sha256=""
typeset carrier_env_key=false
typeset carrier_direct_bearer=false
typeset carrier_missing=true
typeset shared_user_layer=false
typeset canary_model_match=false
typeset canary_provider_match=false
typeset test_only=false
typeset execution_error=""

function load_fixture_observation() {
  test_only=true
  os_name="Darwin"
  os_major="14"
  observed_arch="$expected_arch"
  executable_present=true
  version="$expected_version"
  if [[ "$role" == "official_cli" ]]; then
    binary_sha256="$(sha256_string "fixture-official-cli")"
  else
    binary_sha256="$(sha256_string "fixture-bundled-host")"
  fi
  schema_sha256="$(sha256_string "fixture-schema")"
  config_root_category="default_user"
  protocol_initialize=true
  protocol_initialized=true
  protocol_config_read=true
  protocol_include_layers=true
  model_sha256="$(sha256_string "fixture-model")"
  provider_sha256="$(sha256_string "fixture-provider")"
  origin_sha256="$(sha256_string "fixture-origin")"
  carrier_env_key=false
  carrier_direct_bearer=true
  carrier_missing=false
  shared_user_layer=true
  canary_model_match=true
  canary_provider_match=true

  case "$fixture_case" in
    positive)
      ;;
    wrong_arch)
      if [[ "$expected_arch" == "arm64" ]]; then
        observed_arch="x86_64"
      else
        observed_arch="arm64"
      fi
      ;;
    root_mismatch)
      config_root_category="custom"
      shared_user_layer=false
      ;;
    provider_mismatch)
      provider_sha256="$(sha256_string "fixture-provider-mismatch")"
      canary_provider_match=false
      ;;
    origin_mismatch)
      origin_sha256="$(sha256_string "fixture-origin-mismatch")"
      shared_user_layer=false
      ;;
    carrier_mismatch)
      carrier_env_key=true
      carrier_direct_bearer=false
      carrier_missing=false
      provider_sha256="$(sha256_string "fixture-carrier-mismatch")"
      ;;
    *)
      execution_error="fixture case does not exist"
      ;;
  esac
}

function schema_tree_digest() {
  local schema_root="$1"
  local digest_input="$schema_root/digest-input.bin"
  : > "$digest_input"
  while IFS= read -r schema_file; do
    local relative_path="${schema_file#${schema_root}/}"
    print -rn -- "$relative_path" >> "$digest_input"
    print -rn -- $'\0' >> "$digest_input"
    cat -- "$schema_file" >> "$digest_input"
    print -rn -- $'\0' >> "$digest_input"
  done < <(
    find "$schema_root" -type f ! -name 'digest-input.bin' -print |
      LC_ALL=C sort
  )
  sha256_file "$digest_input"
}

function read_coproc_response() {
  local wanted_id="$1"
  local response_line=""
  local remaining=20
  while (( remaining > 0 )); do
    if IFS= read -r -t 1 -p response_line; then
      if [[ "$response_line" == *"\"id\":${wanted_id}"* ||
            "$response_line" == *"\"id\": ${wanted_id}"* ]]; then
        REPLY="$response_line"
        return 0
      fi
    elif [[ -n "$coproc_pid" ]] && ! kill -0 "$coproc_pid" 2>/dev/null; then
      break
    fi
    (( remaining -= 1 ))
  done
  return 1
}

function parse_app_server_observation() {
  local response_file="$1"
  local expected_codex_home="$2"
  local jxa_file="$temporary_root/parse-response.js"
  cat > "$jxa_file" <<'JXA'
ObjC.import("Foundation");

function standardizePath(value) {
  if (!value) {
    return "";
  }
  return ObjC.unwrap($(value).stringByStandardizingPath);
}

function originType(origin) {
  if (!origin || !origin.name || !origin.name.type) {
    return "";
  }
  return String(origin.name.type);
}

function run(argv) {
  const path = argv[0];
  const expectedHome = standardizePath(argv[1]);
  const source = ObjC.unwrap(
    $.NSString.stringWithContentsOfFileEncodingError(
      path,
      $.NSUTF8StringEncoding,
      null
    )
  );
  const messages = source
    .split(/\r?\n/)
    .filter(Boolean)
    .map((line) => JSON.parse(line));
  const initialize = messages.find((message) => Number(message.id) === 1);
  const configRead = messages.find((message) => Number(message.id) === 2);
  if (!initialize || !initialize.result || !configRead || !configRead.result) {
    throw new Error("required app-server responses are missing");
  }

  const result = configRead.result;
  const config = result.config || {};
  const providerKey = String(config.model_provider || "");
  const provider = (config.model_providers || {})[providerKey] || {};
  const envKey = Boolean(provider.env_key);
  const directBearer = Boolean(provider.experimental_bearer_token);
  const carrier = {
    env_key: envKey,
    direct_bearer: directBearer,
    missing: !envKey && !directBearer
  };
  const providerSummary = {
    provider_key: providerKey,
    name: String(provider.name || ""),
    base_url: String(provider.base_url || ""),
    wire_api: String(provider.wire_api || ""),
    requires_openai_auth: Boolean(provider.requires_openai_auth),
    supports_websockets: Boolean(provider.supports_websockets),
    carrier
  };
  const origins = result.origins || {};
  const layers = (result.layers || []).map(originType);
  const modelOrigin = originType(origins.model);
  const providerOrigin = originType(origins.model_provider);
  const originSummary = {
    model: modelOrigin,
    provider: providerOrigin,
    layers
  };
  const codexHome = standardizePath(initialize.result.codexHome);
  const sharedUserLayer =
    codexHome === expectedHome &&
    modelOrigin === "user" &&
    providerOrigin === "user" &&
    layers.includes("user");
  return [
    codexHome === expectedHome ? "default_user" : "custom",
    String(config.model || ""),
    providerKey,
    JSON.stringify(providerSummary),
    JSON.stringify(originSummary),
    envKey ? "true" : "false",
    directBearer ? "true" : "false",
    !envKey && !directBearer ? "true" : "false",
    sharedUserLayer ? "true" : "false"
  ].join("\u001f");
}
JXA

  local parsed=""
  parsed=$(
    /usr/bin/osascript -l JavaScript \
      "$jxa_file" \
      "$response_file" \
      "$expected_codex_home" \
      2>/dev/null
  )

  local model=""
  local provider_key=""
  local provider_summary=""
  local origin_summary=""
  IFS=$'\x1f' read -r \
    config_root_category \
    model \
    provider_key \
    provider_summary \
    origin_summary \
    carrier_env_key \
    carrier_direct_bearer \
    carrier_missing \
    shared_user_layer <<< "$parsed"

  model_sha256="$(sha256_string "$model")"
  provider_sha256="$(sha256_string "$provider_summary")"
  origin_sha256="$(sha256_string "$origin_summary")"
  [[ "$model" == "$EXPECTED_MODEL" ]] && canary_model_match=true
  [[ "$provider_key" == "$EXPECTED_PROVIDER" ]] && canary_provider_match=true
}

function load_live_observation() {
  os_name="$(uname -s)"
  observed_arch="$(uname -m)"
  if [[ "$os_name" == "Darwin" ]]; then
    os_major="$(/usr/bin/sw_vers -productVersion | /usr/bin/awk -F. '{print $1}')"
  fi

  if [[ -z "$disposable_home" || -z "$working_directory" ]]; then
    execution_error="live probe requires disposable home and working directory"
    return
  fi
  if [[ ! -d "$disposable_home" || ! -d "$working_directory" ]]; then
    execution_error="live probe directories are unavailable"
    return
  fi
  if [[ -z "$codex_executable" || ! -x "$codex_executable" ]]; then
    return
  fi

  local canonical_home=""
  local canonical_current_home=""
  canonical_home="$(canonical_directory "$disposable_home")" || {
    execution_error="disposable home cannot be resolved"
    return
  }
  canonical_current_home="$(canonical_directory "$HOME")" || {
    execution_error="current home cannot be resolved"
    return
  }
  if [[ "$canonical_home" != "$canonical_current_home" ]]; then
    execution_error="live probe requires the current disposable OS user"
    return
  fi
  if [[ ! -f "$canonical_home/.codex/config.toml" ]]; then
    execution_error="canary user config is unavailable"
    return
  fi

  executable_present=true
  local version_output=""
  version_output=$("$codex_executable" --version 2>/dev/null) || {
    execution_error="Codex version probe failed"
    return
  }
  version="$(print -r -- "$version_output" |
    /usr/bin/sed -nE 's/.*([0-9]+\.[0-9]+\.[0-9]+).*/\1/p' |
    /usr/bin/head -n 1)"
  binary_sha256="$(sha256_file "$codex_executable")"

  temporary_root="$(mktemp -d "${TMPDIR:-/tmp}/gpteasy-codex-macos-probe.XXXXXX")"
  chmod 700 "$temporary_root"
  local schema_root="$temporary_root/schema"
  mkdir -p "$schema_root"
  "$codex_executable" app-server generate-json-schema --out "$schema_root" \
    >/dev/null 2>&1 || {
      execution_error="Codex schema generation failed"
      return
    }
  schema_sha256="$(schema_tree_digest "$schema_root")"

  local cwd_json=""
  cwd_json="$(json_quote "$working_directory")"
  local initialize_message='{"id":1,"method":"initialize","params":{"clientInfo":{"name":"gpteasy-contract-probe","version":"1.0.0"},"capabilities":{"experimentalApi":true}}}'
  local initialized_message='{"method":"initialized","params":{}}'
  local config_message="{\"id\":2,\"method\":\"config/read\",\"params\":{\"cwd\":${cwd_json},\"includeLayers\":true}}"
  local -a app_server_arguments
  if [[ "$role" == "bundled_host" ]]; then
    app_server_arguments=(
      -c
      "features.code_mode_host=true"
      app-server
      --analytics-default-enabled
    )
  else
    app_server_arguments=(app-server)
  fi

  coproc env -u CODEX_HOME \
    HOME="$canonical_home" \
    "$codex_executable" \
    "${app_server_arguments[@]}" \
    2>/dev/null
  coproc_pid="$!"
  print -p -r -- "$initialize_message"
  local initialize_response=""
  if ! read_coproc_response 1; then
    execution_error="initialize response was unavailable"
    return
  fi
  initialize_response="$REPLY"
  protocol_initialize=true

  print -p -r -- "$initialized_message"
  protocol_initialized=true
  print -p -r -- "$config_message"
  local config_response=""
  if ! read_coproc_response 2; then
    execution_error="config/read response was unavailable"
    return
  fi
  config_response="$REPLY"
  protocol_config_read=true
  protocol_include_layers=true

  if kill -0 "$coproc_pid" 2>/dev/null; then
    kill "$coproc_pid" 2>/dev/null || true
  fi
  wait "$coproc_pid" 2>/dev/null || true
  coproc_pid=""

  local response_file="$temporary_root/app-server-responses.jsonl"
  umask 077
  print -r -- "$initialize_response" > "$response_file"
  print -r -- "$config_response" >> "$response_file"
  if ! parse_app_server_observation "$response_file" "$canonical_home/.codex"; then
    rm -f -- "$response_file"
    execution_error="config/read summary could not be derived"
    return
  fi
  rm -f -- "$response_file"
}

if [[ -n "$fixture_case" ]]; then
  if ! load_fixture_observation; then
    execution_error="fixture observation could not be loaded"
  fi
else
  if ! load_live_observation; then
    execution_error="live observation could not be completed"
  fi
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

add_check "native_macos" \
  "$(is_native_macos_14_or_newer && print true || print false)" \
  "MACOS_NATIVE_HOST_REQUIRED"
add_check "native_arch" \
  "$([[ "$observed_arch" == "$expected_arch" ]] && print true || print false)" \
  "MACOS_ARCH_MISMATCH"
add_check "executable_present" \
  "$executable_present" \
  "CODEX_EXECUTABLE_MISSING" \
  "blocked"
add_check "version_exact" \
  "$([[ "$version" == "$expected_version" ]] && print true || print false)" \
  "CODEX_VERSION_MISMATCH"
add_check "binary_digest" \
  "$(is_sha256 "$binary_sha256" && print true || print false)" \
  "CODEX_BINARY_DIGEST_INVALID"
add_check "schema_digest" \
  "$(is_sha256 "$schema_sha256" && print true || print false)" \
  "CODEX_SCHEMA_DIGEST_INVALID"
add_check "app_server_protocol" \
  "$([[ "$protocol_initialize" == true &&
        "$protocol_initialized" == true &&
        "$protocol_config_read" == true &&
        "$protocol_include_layers" == true ]] && print true || print false)" \
  "CODEX_APP_SERVER_PROTOCOL_INCOMPLETE"
add_check "default_config_root" \
  "$([[ "$config_root_category" == "default_user" ]] && print true || print false)" \
  "CODEX_CONFIG_ROOT_NOT_DEFAULT"
add_check "canary_model" \
  "$canary_model_match" \
  "CODEX_CANARY_MODEL_MISMATCH"
add_check "canary_provider" \
  "$canary_provider_match" \
  "CODEX_CANARY_PROVIDER_MISMATCH"
add_check "summary_digests" \
  "$(is_sha256 "$model_sha256" &&
      is_sha256 "$provider_sha256" &&
      is_sha256 "$origin_sha256" &&
      print true || print false)" \
  "CODEX_SUMMARY_DIGEST_INVALID"
add_check "credential_carrier" \
  "$([[ "$carrier_env_key" == false &&
        "$carrier_direct_bearer" == true &&
        "$carrier_missing" == false ]] && print true || print false)" \
  "CODEX_CREDENTIAL_CARRIER_MISMATCH"
add_check "shared_user_layer" \
  "$shared_user_layer" \
  "CODEX_SHARED_USER_LAYER_MISSING"

if [[ -n "$execution_error" ]]; then
  add_check "probe_execution" false "CODEX_PROBE_UNAVAILABLE" "blocked"
fi

typeset exit_code="$EXIT_COMPLETED"
typeset outcome="passed"
if (( ${blocking_reasons[(I)CODEX_EXECUTABLE_MISSING]} > 0 )) ||
   (( ${blocking_reasons[(I)CODEX_PROBE_UNAVAILABLE]} > 0 )); then
  exit_code="$EXIT_STRICT_PREREQUISITE_BLOCKED"
  outcome="blocked"
elif (( ${#blocking_reasons} > 0 )); then
  typeset security_failure=false
  typeset code=""
  for code in "${blocking_reasons[@]}"; do
    case "$code" in
      MACOS_NATIVE_HOST_REQUIRED|MACOS_ARCH_MISMATCH|CODEX_CONFIG_ROOT_NOT_DEFAULT|CODEX_CANARY_MODEL_MISMATCH|CODEX_CANARY_PROVIDER_MISMATCH|CODEX_CREDENTIAL_CARRIER_MISMATCH|CODEX_SHARED_USER_LAYER_MISSING)
        security_failure=true
        ;;
    esac
  done
  if [[ "$security_failure" == true ]]; then
    exit_code="$EXIT_SECURITY_BOUNDARY_FAILED"
  else
    exit_code="$EXIT_ASSERTION_FAILED"
  fi
  outcome="failed"
fi

typeset strict_gate_eligible=false
if [[ "$exit_code" -eq 0 && "$test_only" == false ]]; then
  strict_gate_eligible=true
fi

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

if [[ "$output_format" == "tsv" ]]; then
  print -r -- \
    "${outcome}"$'\t'"${exit_code}"$'\t'"${strict_gate_eligible}"$'\t'"${test_only}"$'\t'"${version}"$'\t'"${binary_sha256}"$'\t'"${schema_sha256}"$'\t'"${config_root_category}"$'\t'"${protocol_initialize}"$'\t'"${protocol_initialized}"$'\t'"${protocol_config_read}"$'\t'"${protocol_include_layers}"$'\t'"${model_sha256}"$'\t'"${provider_sha256}"$'\t'"${origin_sha256}"$'\t'"${carrier_env_key}"$'\t'"${carrier_direct_bearer}"$'\t'"${carrier_missing}"$'\t'"${shared_user_layer}"
else
  print -r -- "{\"schema_version\":1,\"probe\":\"codex-app-server-config-read\",\"role\":$(json_quote "$role"),\"outcome\":$(json_quote "$outcome"),\"exit_code\":${exit_code},\"strict_gate_eligible\":${strict_gate_eligible},\"test_only\":${test_only},\"expected_version\":$(json_quote "$expected_version"),\"version\":$(json_nullable_string "$version"),\"binary_sha256\":$(json_nullable_string "$binary_sha256"),\"schema_sha256\":$(json_nullable_string "$schema_sha256"),\"config_root_category\":$(json_quote "$config_root_category"),\"protocol\":{\"initialize\":${protocol_initialize},\"initialized\":${protocol_initialized},\"config_read\":${protocol_config_read},\"include_layers\":${protocol_include_layers}},\"model_sha256\":$(json_nullable_string "$model_sha256"),\"provider_sha256\":$(json_nullable_string "$provider_sha256"),\"origin_sha256\":$(json_nullable_string "$origin_sha256"),\"credential_carrier\":{\"env_key\":${carrier_env_key},\"direct_bearer\":${carrier_direct_bearer},\"missing\":${carrier_missing}},\"shared_user_layer\":${shared_user_layer},\"checks\":[${checks_json}],\"blocking_reasons\":[${reasons_json}]}"
fi

exit "$exit_code"
