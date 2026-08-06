#!/bin/zsh
set -euo pipefail

readonly EXIT_COMPLETED=0
readonly EXIT_STRICT_PREREQUISITE_BLOCKED=3
readonly EXIT_SECURITY_BOUNDARY_FAILED=5
readonly EXPECTED_REPOSITORY="yinshaohua/GPTEasy"

action="initialize"
state_path=""
evidence_path=""
baseline_root=""
standard_output_path=""
standard_error_path=""
launch_session=false
typeset -a command_arguments
command_arguments=()

function json_quote() {
  local value="$1"
  value="${value//\\/\\\\}"
  value="${value//\"/\\\"}"
  value="${value//$'\n'/\\n}"
  value="${value//$'\r'/\\r}"
  value="${value//$'\t'/\\t}"
  print -nr -- "\"$value\""
}

function sha256_string() {
  print -nr -- "$1" |
    /usr/bin/shasum -a 256 |
    /usr/bin/awk '{print $1}'
}

function valid_hex_string() {
  local value="$1"
  local expected_length="$2"
  local remainder="${value//[0-9a-f]/}"
  [[ ${#value} -eq "$expected_length" && -z "$remainder" ]]
}

function valid_decimal_string() {
  local value="$1"
  local remainder="${value//[0-9]/}"
  [[ -n "$value" && -z "$remainder" ]]
}

function restore_caller_ownership() {
  local path="$1"
  local caller_uid="${SUDO_UID:-}"
  local caller_gid="${SUDO_GID:-}"
  [[ -e "$path" ]] || return 0
  valid_decimal_string "$caller_uid" || return 0
  valid_decimal_string "$caller_gid" || return 0
  [[ "$caller_uid" -ne 0 ]] || return 0
  /usr/sbin/chown "${caller_uid}:${caller_gid}" "$path"
}

function required_environment() {
  local name="$1"
  local value=""
  if (( ${+parameters[$name]} )); then
    value="${(P)name}"
  fi
  [[ -n "$value" ]] || return 1
  print -nr -- "$value"
}

function optional_environment() {
  local name="$1"
  local fallback="$2"
  local value=""
  if (( ${+parameters[$name]} )); then
    value="${(P)name}"
  fi
  if [[ -n "$value" ]]; then
    print -nr -- "$value"
  else
    print -nr -- "$fallback"
  fi
}

function ensure_private_parent() {
  local path="$1"
  local parent="${path:h}"
  /bin/mkdir -p "$parent"
  /bin/chmod 700 "$parent"
}

function write_private_file() {
  local path="$1"
  local content="$2"
  ensure_private_parent "$path"
  local temporary_path="${path}.tmp.$$"
  print -nr -- "$content" > "$temporary_path"
  /bin/chmod 600 "$temporary_path"
  /bin/mv -f "$temporary_path" "$path"
  restore_caller_ownership "$path"
}

function read_json_value() {
  local path="$1"
  local key="$2"
  /usr/bin/plutil -extract "$key" raw -o - "$path" 2>/dev/null
}

function tree_sha256() {
  local root="$1"
  [[ -d "$root" ]] || return 1
  (
    builtin cd "$root"
    while IFS= read -r relative_path; do
      [[ -f "$relative_path" ]] || continue
      local digest
      digest=$(/usr/bin/shasum -a 256 "$relative_path" | /usr/bin/awk '{print $1}')
      print -r -- "${relative_path}\t${digest}"
    done < <(/usr/bin/find . -type f -print | LC_ALL=C /usr/bin/sort)
  ) | /usr/bin/shasum -a 256 | /usr/bin/awk '{print $1}'
}

function emit_blocked() {
  local reason="$1"
  print -r -- "{\"schema_version\":1,\"probe\":\"macos-job-lifecycle\",\"action\":$(json_quote "$action"),\"outcome\":\"blocked\",\"exit_code\":${EXIT_STRICT_PREREQUISITE_BLOCKED},\"strict_gate_eligible\":false,\"blocking_reasons\":[$(json_quote "$reason")]}"
  exit "$EXIT_STRICT_PREREQUISITE_BLOCKED"
}

function require_root() {
  [[ "$EUID" -eq 0 ]] || emit_blocked "MACOS_LIFECYCLE_REQUIRES_ROOT"
}

function load_runner_identity() {
  github_run_id="$(required_environment GITHUB_RUN_ID)" ||
    emit_blocked "MACOS_GITHUB_IDENTITY_MISSING"
  github_run_attempt="$(required_environment GITHUB_RUN_ATTEMPT)" ||
    emit_blocked "MACOS_GITHUB_IDENTITY_MISSING"
  github_job="$(required_environment GITHUB_JOB)" ||
    emit_blocked "MACOS_GITHUB_IDENTITY_MISSING"
  github_sha="$(required_environment GITHUB_SHA)" ||
    emit_blocked "MACOS_GITHUB_IDENTITY_MISSING"
  github_repository="$(required_environment GITHUB_REPOSITORY)" ||
    emit_blocked "MACOS_GITHUB_IDENTITY_MISSING"
  [[ "$github_repository" == "$EXPECTED_REPOSITORY" ]] ||
    emit_blocked "MACOS_GITHUB_IDENTITY_MISMATCH"

  runner_name="$(required_environment RUNNER_NAME)" ||
    emit_blocked "MACOS_RUNNER_IDENTITY_MISSING"
  runner_tracking_id="$(required_environment RUNNER_TRACKING_ID)" ||
    emit_blocked "MACOS_RUNNER_IDENTITY_MISSING"
  runner_arch_reported="$(required_environment RUNNER_ARCH)" ||
    emit_blocked "MACOS_RUNNER_IDENTITY_MISSING"
  runner_architecture="$(/usr/bin/uname -m)"
  local normalized_reported_architecture=""
  case "${runner_arch_reported:l}" in
    arm64)
      normalized_reported_architecture="arm64"
      ;;
    x64|x86_64)
      normalized_reported_architecture="x86_64"
      ;;
    *)
      emit_blocked "MACOS_RUNNER_ARCHITECTURE_INVALID"
      ;;
  esac
  [[ "$normalized_reported_architecture" == "$runner_architecture" ]] ||
    emit_blocked "MACOS_RUNNER_ARCHITECTURE_MISMATCH"
  runner_image="$(optional_environment ImageOS "$(optional_environment ImageVersion self-hosted)")"
  runner_environment="$(required_environment RUNNER_ENVIRONMENT)" ||
    emit_blocked "MACOS_RUNNER_IDENTITY_MISSING"
  runner_ephemeral=false
  if [[ "$runner_environment" == "github-hosted" ||
        "$(optional_environment RUNNER_EPHEMERAL false)" == true ]]; then
    runner_ephemeral=true
  fi
  runner_dedicated=false
  [[ -n "$github_job" ]] && runner_dedicated=true
  runner_name_sha256="$(sha256_string "$runner_name")"
  runner_tracking_sha256="$(sha256_string "$runner_tracking_id")"
}

function assert_state_identity() {
  load_runner_identity
  [[ "$(read_json_value "$state_path" identity.github.repository)" == "$github_repository" &&
     "$(read_json_value "$state_path" identity.github.run_id)" == "$github_run_id" &&
     "$(read_json_value "$state_path" identity.github.run_attempt)" == "$github_run_attempt" &&
     "$(read_json_value "$state_path" identity.github.job)" == "$github_job" &&
     "$(read_json_value "$state_path" identity.github.commit)" == "$github_sha" &&
     "$(read_json_value "$state_path" identity.runner.name_sha256)" == "$runner_name_sha256" &&
     "$(read_json_value "$state_path" identity.runner.tracking_id_sha256)" == "$runner_tracking_sha256" &&
     "$(read_json_value "$state_path" identity.runner.architecture)" == "$runner_architecture" ]] ||
    emit_blocked "MACOS_LIFECYCLE_IDENTITY_MISMATCH"
}

function valid_disposable_identity() {
  local account_name="$1"
  local account_home="$2"
  local suffix="${account_name#gpteasyjob}"
  [[ "$account_name" == "gpteasyjob${suffix}" &&
     "$account_home" == "/Users/$account_name" ]] &&
    valid_hex_string "$suffix" 10
}

function rollback_disposable_account() {
  local account_name="$1"
  local account_home="$2"
  valid_disposable_identity "$account_name" "$account_home" || return 1
  /usr/sbin/sysadminctl -deleteUser "$account_name" -secure >/dev/null 2>&1 || true
  if /usr/bin/id -u "$account_name" >/dev/null 2>&1; then
    /usr/bin/dscl . -delete "/Users/$account_name" >/dev/null 2>&1 || true
  fi
  [[ ! -e "$account_home" ]] || /bin/rm -rf "$account_home"
}

function initialize_lifecycle() {
  require_root
  [[ -n "$state_path" ]] || emit_blocked "MACOS_LIFECYCLE_STATE_PATH_REQUIRED"
  [[ "$(uname -s)" == Darwin ]] || emit_blocked "MACOS_NATIVE_HOST_REQUIRED"
  load_runner_identity

  local suffix
  suffix="$(/usr/bin/uuidgen | /usr/bin/tr '[:upper:]' '[:lower:]' | /usr/bin/tr -d '-' | /usr/bin/cut -c1-10)"
  local account_name="gpteasyjob${suffix}"
  local account_home="/Users/${account_name}"
  valid_disposable_identity "$account_name" "$account_home" ||
    emit_blocked "MACOS_DISPOSABLE_ACCOUNT_IDENTITY_INVALID"
  if /usr/bin/id -u "$account_name" >/dev/null 2>&1 || [[ -e "$account_home" ]]; then
    emit_blocked "MACOS_DISPOSABLE_ACCOUNT_ALREADY_EXISTS"
  fi

  local password
  password="Aa1!$(/usr/bin/openssl rand -base64 32 | /usr/bin/tr -d '\r\n')"
  local baseline_before=""
  if [[ -n "$baseline_root" ]]; then
    baseline_root="${baseline_root:A}"
    baseline_before="$(tree_sha256 "$baseline_root")" ||
      emit_blocked "MACOS_BASELINE_UNAVAILABLE"
  fi

  if ! /usr/sbin/sysadminctl \
    -addUser "$account_name" \
    -fullName "GPTEasy disposable contract account" \
    -home "$account_home" \
    -shell /bin/zsh \
    -password "$password" \
    >/dev/null 2>&1; then
    if /usr/bin/id -u "$account_name" >/dev/null 2>&1 ||
       [[ -e "$account_home" ]]; then
      rollback_disposable_account "$account_name" "$account_home" || true
    fi
    emit_blocked "MACOS_ACCOUNT_CREATION_FAILED"
  fi
  password=""

  /usr/sbin/createhomedir -c -u "$account_name" >/dev/null 2>&1 || true
  if [[ ! -d "$account_home" ]]; then
    rollback_disposable_account "$account_name" "$account_home" || true
    emit_blocked "MACOS_PROFILE_CREATION_FAILED"
  fi

  local uid
  if ! uid="$(/usr/bin/id -u "$account_name" 2>/dev/null)"; then
    rollback_disposable_account "$account_name" "$account_home" || true
    emit_blocked "MACOS_ACCOUNT_CREATION_FAILED"
  fi

  local state
  state="{\"schema_version\":1,\"account_name\":$(json_quote "$account_name"),\"account_uid\":$(json_quote "$uid"),\"profile_path\":$(json_quote "$account_home"),\"created_for_job\":true,\"profile_created_for_job\":true,\"baseline_root\":$(json_quote "$baseline_root"),\"baseline_before_sha256\":$(json_quote "$baseline_before"),\"created_utc\":$(json_quote "$(/bin/date -u +'%Y-%m-%dT%H:%M:%SZ')"),\"identity\":{\"github\":{\"repository\":$(json_quote "$github_repository"),\"run_id\":$(json_quote "$github_run_id"),\"run_attempt\":${github_run_attempt},\"job\":$(json_quote "$github_job"),\"commit\":$(json_quote "$github_sha")},\"runner\":{\"name_sha256\":$(json_quote "$runner_name_sha256"),\"image\":$(json_quote "$runner_image"),\"tracking_id_sha256\":$(json_quote "$runner_tracking_sha256"),\"reported_architecture\":$(json_quote "$runner_arch_reported"),\"architecture\":$(json_quote "$runner_architecture"),\"ephemeral\":${runner_ephemeral},\"dedicated_job\":${runner_dedicated}}}}"
  if ! write_private_file "$state_path" "$state"; then
    rollback_disposable_account "$account_name" "$account_home" || true
    /bin/rm -f "$state_path" "${state_path}.tmp.$$"
    emit_blocked "MACOS_LIFECYCLE_STATE_WRITE_FAILED"
  fi

  print -r -- "{\"schema_version\":1,\"probe\":\"macos-job-lifecycle\",\"action\":\"initialize\",\"outcome\":\"passed\",\"exit_code\":0,\"strict_gate_eligible\":false,\"account_created_for_job\":true,\"profile_created_for_job\":true}"
}

function invoke_as_disposable_user() {
  require_root
  [[ -f "$state_path" ]] || emit_blocked "MACOS_LIFECYCLE_STATE_MISSING"
  (( ${#command_arguments} > 0 )) ||
    emit_blocked "MACOS_LIFECYCLE_COMMAND_REQUIRED"
  assert_state_identity

  local account_name
  local account_uid
  local account_home
  account_name="$(read_json_value "$state_path" account_name)" ||
    emit_blocked "MACOS_LIFECYCLE_STATE_INVALID"
  account_uid="$(read_json_value "$state_path" account_uid)" ||
    emit_blocked "MACOS_LIFECYCLE_STATE_INVALID"
  account_home="$(read_json_value "$state_path" profile_path)" ||
    emit_blocked "MACOS_LIFECYCLE_STATE_INVALID"
  valid_disposable_identity "$account_name" "$account_home" ||
    emit_blocked "MACOS_DISPOSABLE_ACCOUNT_IDENTITY_INVALID"
  [[ "$(/usr/bin/id -u "$account_name" 2>/dev/null)" == "$account_uid" &&
     -d "$account_home" ]] ||
    emit_blocked "MACOS_DISPOSABLE_ACCOUNT_MISSING"

  if [[ -n "$standard_output_path" ]]; then
    ensure_private_parent "$standard_output_path"
    : > "$standard_output_path"
    /bin/chmod 600 "$standard_output_path"
  fi
  if [[ -n "$standard_error_path" ]]; then
    ensure_private_parent "$standard_error_path"
    : > "$standard_error_path"
    /bin/chmod 600 "$standard_error_path"
  fi

  typeset -a invocation
  invocation=(
    /usr/bin/sudo -n -H -u "$account_name"
    /usr/bin/env
    "HOME=$account_home"
    "USER=$account_name"
    "LOGNAME=$account_name"
    "SHELL=/bin/zsh"
    "PATH=$PATH"
    "${command_arguments[@]}"
  )
  if [[ "$launch_session" == true ]]; then
    invocation=(
      /bin/launchctl asuser "$account_uid"
      "${invocation[@]}"
    )
  fi

  local status=0
  if [[ -n "$standard_output_path" && -n "$standard_error_path" ]]; then
    "${invocation[@]}" >"$standard_output_path" 2>"$standard_error_path" || status="$?"
  elif [[ -n "$standard_output_path" ]]; then
    "${invocation[@]}" >"$standard_output_path" || status="$?"
  elif [[ -n "$standard_error_path" ]]; then
    "${invocation[@]}" 2>"$standard_error_path" || status="$?"
  else
    "${invocation[@]}" || status="$?"
  fi
  [[ -n "$standard_output_path" ]] &&
    restore_caller_ownership "$standard_output_path"
  [[ -n "$standard_error_path" ]] &&
    restore_caller_ownership "$standard_error_path"
  return "$status"
}

function stop_run_scoped_processes() {
  local uid="$1"
  typeset -a process_ids
  process_ids=("${(@f)$(/usr/bin/pgrep -u "$uid" 2>/dev/null || true)}")
  if (( ${#process_ids} == 0 )); then
    return 0
  fi

  /bin/kill -TERM "${process_ids[@]}" >/dev/null 2>&1 || true
  local attempt=0
  for attempt in 1 2 3 4 5; do
    /bin/sleep 1
    process_ids=("${(@f)$(/usr/bin/pgrep -u "$uid" 2>/dev/null || true)}")
    (( ${#process_ids} == 0 )) && return 0
  done
  /bin/kill -KILL "${process_ids[@]}" >/dev/null 2>&1 || true
}

function write_lifecycle_evidence() {
  local account_uid="$1"
  local account_home="$2"
  local account_absent="$3"
  local profile_absent="$4"
  local baseline_restored="$5"
  local cleanup_succeeded="$6"
  local uid_sha256
  local profile_sha256
  uid_sha256="$(sha256_string "$account_uid")"
  profile_sha256="$(sha256_string "$account_home")"
  local evidence
  evidence="{\"schema_version\":1,\"runner_lifecycle\":{\"ephemeral\":$(read_json_value "$state_path" identity.runner.ephemeral),\"dedicated_job\":$(read_json_value "$state_path" identity.runner.dedicated_job)},\"account_lifecycle\":{\"uid_sha256\":$(json_quote "$uid_sha256"),\"profile_id_sha256\":$(json_quote "$profile_sha256"),\"created_for_job\":$(read_json_value "$state_path" created_for_job),\"profile_created_for_job\":$(read_json_value "$state_path" profile_created_for_job),\"cleanup_attempted\":true,\"cleanup_attested\":${cleanup_succeeded},\"cleanup_succeeded\":${cleanup_succeeded},\"account_absent_after_cleanup\":${account_absent},\"profile_absent_after_cleanup\":${profile_absent},\"baseline_restored\":${baseline_restored}},\"github\":{\"repository\":$(json_quote "$(read_json_value "$state_path" identity.github.repository)"),\"run_id\":$(json_quote "$(read_json_value "$state_path" identity.github.run_id)"),\"run_attempt\":$(read_json_value "$state_path" identity.github.run_attempt),\"job\":$(json_quote "$(read_json_value "$state_path" identity.github.job)"),\"commit\":$(json_quote "$(read_json_value "$state_path" identity.github.commit)")},\"runner\":{\"name_sha256\":$(json_quote "$(read_json_value "$state_path" identity.runner.name_sha256)"),\"image\":$(json_quote "$(read_json_value "$state_path" identity.runner.image)"),\"tracking_id_sha256\":$(json_quote "$(read_json_value "$state_path" identity.runner.tracking_id_sha256)"),\"reported_architecture\":$(json_quote "$(read_json_value "$state_path" identity.runner.reported_architecture)"),\"architecture\":$(json_quote "$(read_json_value "$state_path" identity.runner.architecture)"),\"ephemeral\":$(read_json_value "$state_path" identity.runner.ephemeral),\"dedicated_job\":$(read_json_value "$state_path" identity.runner.dedicated_job)}}"
  write_private_file "$evidence_path" "$evidence"
}

function finalize_lifecycle() {
  require_root
  [[ -f "$state_path" ]] || emit_blocked "MACOS_LIFECYCLE_STATE_MISSING"
  [[ -n "$evidence_path" ]] || emit_blocked "MACOS_LIFECYCLE_EVIDENCE_PATH_REQUIRED"
  assert_state_identity

  local account_name
  local account_uid
  local account_home
  account_name="$(read_json_value "$state_path" account_name)" ||
    emit_blocked "MACOS_LIFECYCLE_STATE_INVALID"
  account_uid="$(read_json_value "$state_path" account_uid)" ||
    emit_blocked "MACOS_LIFECYCLE_STATE_INVALID"
  account_home="$(read_json_value "$state_path" profile_path)" ||
    emit_blocked "MACOS_LIFECYCLE_STATE_INVALID"
  valid_disposable_identity "$account_name" "$account_home" ||
    emit_blocked "MACOS_DISPOSABLE_ACCOUNT_IDENTITY_INVALID"

  stop_run_scoped_processes "$account_uid"
  /usr/sbin/sysadminctl -deleteUser "$account_name" -secure >/dev/null 2>&1 || true
  if /usr/bin/id -u "$account_name" >/dev/null 2>&1; then
    /usr/bin/dscl . -delete "/Users/$account_name" >/dev/null 2>&1 || true
  fi
  if [[ -e "$account_home" ]]; then
    valid_disposable_identity "$account_name" "$account_home" ||
      emit_blocked "MACOS_PROFILE_PATH_UNSAFE"
    /bin/rm -rf "$account_home"
  fi

  local account_absent=false
  local profile_absent=false
  if ! /usr/bin/id -u "$account_name" >/dev/null 2>&1 &&
     ! /usr/bin/dscl . -read "/Users/$account_name" >/dev/null 2>&1; then
    account_absent=true
  fi
  [[ ! -e "$account_home" ]] && profile_absent=true

  local baseline_restored=false
  local saved_baseline_root
  local baseline_before
  saved_baseline_root="$(read_json_value "$state_path" baseline_root)"
  baseline_before="$(read_json_value "$state_path" baseline_before_sha256)"
  if [[ -n "$saved_baseline_root" && -n "$baseline_before" &&
        "$runner_environment" == self-hosted &&
        "$(optional_environment RUNNER_EPHEMERAL false)" == true ]]; then
    local baseline_after
    baseline_after="$(tree_sha256 "$saved_baseline_root")" || baseline_after=""
    [[ "$baseline_after" == "$baseline_before" ]] && baseline_restored=true
  fi

  local cleanup_succeeded=false
  if [[ "$runner_ephemeral" == true &&
        "$runner_dedicated" == true &&
        ( "$account_absent" == true && "$profile_absent" == true ||
          "$baseline_restored" == true ) ]]; then
    cleanup_succeeded=true
  fi
  write_lifecycle_evidence \
    "$account_uid" \
    "$account_home" \
    "$account_absent" \
    "$profile_absent" \
    "$baseline_restored" \
    "$cleanup_succeeded"

  if [[ "$cleanup_succeeded" == true ]]; then
    print -r -- "{\"schema_version\":1,\"probe\":\"macos-job-lifecycle\",\"action\":\"finalize\",\"outcome\":\"passed\",\"exit_code\":0,\"strict_gate_eligible\":false,\"cleanup_attested\":true,\"cleanup_succeeded\":true}"
    return 0
  fi

  print -r -- "{\"schema_version\":1,\"probe\":\"macos-job-lifecycle\",\"action\":\"finalize\",\"outcome\":\"failed\",\"exit_code\":${EXIT_SECURITY_BOUNDARY_FAILED},\"strict_gate_eligible\":false,\"cleanup_attested\":false,\"cleanup_succeeded\":false,\"blocking_reasons\":[\"MACOS_LIFECYCLE_CLEANUP_FAILED\"]}"
  return "$EXIT_SECURITY_BOUNDARY_FAILED"
}

while (( $# > 0 )); do
  case "$1" in
    --action)
      [[ $# -ge 2 ]] || emit_blocked "MACOS_LIFECYCLE_ARGUMENT_INVALID"
      action="$2"
      shift 2
      ;;
    --state-path)
      [[ $# -ge 2 ]] || emit_blocked "MACOS_LIFECYCLE_ARGUMENT_INVALID"
      state_path="$2"
      shift 2
      ;;
    --evidence-path)
      [[ $# -ge 2 ]] || emit_blocked "MACOS_LIFECYCLE_ARGUMENT_INVALID"
      evidence_path="$2"
      shift 2
      ;;
    --baseline-root)
      [[ $# -ge 2 ]] || emit_blocked "MACOS_LIFECYCLE_ARGUMENT_INVALID"
      baseline_root="$2"
      shift 2
      ;;
    --stdout-path)
      [[ $# -ge 2 ]] || emit_blocked "MACOS_LIFECYCLE_ARGUMENT_INVALID"
      standard_output_path="$2"
      shift 2
      ;;
    --stderr-path)
      [[ $# -ge 2 ]] || emit_blocked "MACOS_LIFECYCLE_ARGUMENT_INVALID"
      standard_error_path="$2"
      shift 2
      ;;
    --launch-session)
      launch_session=true
      shift
      ;;
    --)
      shift
      command_arguments=("$@")
      break
      ;;
    *)
      emit_blocked "MACOS_LIFECYCLE_ARGUMENT_INVALID"
      ;;
  esac
done

case "$action" in
  initialize)
    initialize_lifecycle
    ;;
  invoke)
    invoke_as_disposable_user
    exit "$?"
    ;;
  finalize)
    finalize_lifecycle
    exit "$?"
    ;;
  *)
    emit_blocked "MACOS_LIFECYCLE_ACTION_INVALID"
    ;;
esac
