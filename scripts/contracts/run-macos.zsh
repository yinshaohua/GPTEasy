#!/bin/zsh
set -euo pipefail

readonly EXIT_COMPLETED=0
readonly EXIT_STRICT_PREREQUISITE_BLOCKED=3
readonly EXIT_SECURITY_BOUNDARY_FAILED=5
readonly EXPECTED_REPOSITORY="yinshaohua/GPTEasy"
readonly EXPECTED_BUNDLE_IDENTIFIER="com.gpteasy.desktop"
readonly EXPECTED_INSTALL_ROOT_KIND="HOME_APPLICATIONS"

fixture_path=""
fixture_case="positive"
app_path=""
artifact_path=""
install_evidence_path=""
lifecycle_evidence_path=""
expected_arch="arm64"

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

function sha256_file() {
  /usr/bin/shasum -a 256 "$1" | /usr/bin/awk '{print $1}'
}

function read_json_value() {
  local path="$1"
  local key="$2"
  /usr/bin/plutil -extract "$key" raw -o - "$path" 2>/dev/null
}

function emit_unavailable() {
  local fixture_mode="$1"
  local reason="$2"
  local outcome="blocked"
  local exit_code="$EXIT_STRICT_PREREQUISITE_BLOCKED"
  if [[ "$fixture_mode" == true ]]; then
    outcome="failed"
    exit_code="$EXIT_SECURITY_BOUNDARY_FAILED"
  fi
  print -r -- "{\"schema_version\":1,\"probe\":\"macos-package-contract\",\"outcome\":$(json_quote "$outcome"),\"exit_code\":${exit_code},\"strict_gate_eligible\":false,\"test_only\":${fixture_mode},\"blocking_reasons\":[$(json_quote "$reason")]}"
  exit "$exit_code"
}

function valid_sha256() {
  local value="$1"
  local remainder="${value//[0-9a-f]/}"
  [[ ${#value} -eq 64 && -z "$remainder" ]]
}

function valid_decimal() {
  local value="$1"
  local remainder="${value//[0-9]/}"
  [[ -n "$value" && -z "$remainder" ]]
}

function valid_job_name() {
  local value="$1"
  local remainder="${value//[A-Za-z0-9_-]/}"
  [[ -n "$value" && -z "$remainder" ]]
}

function version_major_at_least_14() {
  local version="$1"
  local major="${version%%.*}"
  valid_decimal "$major" && [[ "$major" -ge 14 ]]
}

typeset host_os_name=""
typeset host_os_major="0"
typeset host_architecture=""
typeset package_exists=false
typeset artifact_correlated=false
typeset package_artifact_sha256=""
typeset package_executable_sha256=""
typeset bundle_identifier=""
typeset minimum_system_version=""
typeset executable_architecture=""
typeset install_scope=""
typeset install_root_kind=""
typeset install_profile_sha256=""
typeset install_absolute_path_redacted=false
typeset install_gatekeeper_accepted=false
typeset codesign_verified=false
typeset codesign_deep=false
typeset codesign_strict=false
typeset codesign_developer_id=false
typeset codesign_team_sha256=""
typeset notary_stapled=false
typeset notary_validated=false
typeset gatekeeper_accepted=false
typeset path_smoke_outcome=""
typeset path_smoke_root_kind=""
typeset path_smoke_reopened=false
typeset path_smoke_absolute_path_redacted=false
typeset marker_correlated=false
typeset lifecycle_runner_ephemeral=false
typeset lifecycle_runner_dedicated=false
typeset lifecycle_uid_sha256=""
typeset lifecycle_profile_sha256=""
typeset lifecycle_created_for_job=false
typeset lifecycle_profile_created_for_job=false
typeset lifecycle_cleanup_attempted=false
typeset lifecycle_cleanup_attested=false
typeset lifecycle_cleanup_succeeded=false
typeset lifecycle_account_absent=false
typeset lifecycle_profile_absent=false
typeset lifecycle_baseline_restored=false
typeset lifecycle_repository=""
typeset lifecycle_run_id=""
typeset lifecycle_run_attempt="0"
typeset lifecycle_job=""
typeset lifecycle_commit=""
typeset lifecycle_runner_name_sha256=""
typeset lifecycle_runner_image=""
typeset lifecycle_runner_tracking_sha256=""
typeset lifecycle_runner_architecture=""

function load_fixture() {
  [[ -f "$fixture_path" ]] || return 1
  [[ "$(read_json_value "$fixture_path" fixture_mode)" == true ]] || return 1
  [[ "$(read_json_value "$fixture_path" schema_version)" == 1 ]] || return 1

  host_os_name="$(read_json_value "$fixture_path" host.os_name)" || return 1
  host_os_major="$(read_json_value "$fixture_path" host.os_major)" || return 1
  host_architecture="$(read_json_value "$fixture_path" host.architecture)" || return 1
  package_exists="$(read_json_value "$fixture_path" package.exists)" || return 1
  artifact_correlated="$(read_json_value "$fixture_path" package.artifact_correlated)" || return 1
  package_artifact_sha256="$(read_json_value "$fixture_path" package.artifact_sha256)" || return 1
  package_executable_sha256="$(read_json_value "$fixture_path" package.executable_sha256)" || return 1
  bundle_identifier="$(read_json_value "$fixture_path" package.bundle.identifier)" || return 1
  minimum_system_version="$(read_json_value "$fixture_path" package.bundle.minimum_system_version)" || return 1
  executable_architecture="$(read_json_value "$fixture_path" package.bundle.executable_architecture)" || return 1
  install_scope="$(read_json_value "$fixture_path" package.install.scope)" || return 1
  install_root_kind="$(read_json_value "$fixture_path" package.install.root_kind)" || return 1
  install_profile_sha256="$(read_json_value "$fixture_path" package.install.profile_id_sha256)" || return 1
  install_absolute_path_redacted="$(read_json_value "$fixture_path" package.install.absolute_path_redacted)" || return 1
  install_gatekeeper_accepted="$(read_json_value "$fixture_path" package.install.gatekeeper_accepted)" || return 1
  codesign_verified="$(read_json_value "$fixture_path" package.codesign.verified)" || return 1
  codesign_deep="$(read_json_value "$fixture_path" package.codesign.deep)" || return 1
  codesign_strict="$(read_json_value "$fixture_path" package.codesign.strict)" || return 1
  codesign_developer_id="$(read_json_value "$fixture_path" package.codesign.developer_id)" || return 1
  codesign_team_sha256="$(read_json_value "$fixture_path" package.codesign.team_id_sha256)" || return 1
  notary_stapled="$(read_json_value "$fixture_path" package.notarization.stapled)" || return 1
  notary_validated="$(read_json_value "$fixture_path" package.notarization.validated)" || return 1
  gatekeeper_accepted="$(read_json_value "$fixture_path" package.gatekeeper.accepted)" || return 1
  path_smoke_outcome="$(read_json_value "$fixture_path" package.path_smoke.outcome)" || return 1
  path_smoke_root_kind="$(read_json_value "$fixture_path" package.path_smoke.root_kind)" || return 1
  path_smoke_reopened="$(read_json_value "$fixture_path" package.path_smoke.reopened)" || return 1
  path_smoke_absolute_path_redacted="$(read_json_value "$fixture_path" package.path_smoke.absolute_path_redacted)" || return 1
  marker_correlated="$(read_json_value "$fixture_path" package.marker_correlated)" || return 1

  lifecycle_runner_ephemeral="$(read_json_value "$fixture_path" lifecycle.runner_lifecycle.ephemeral)" || return 1
  lifecycle_runner_dedicated="$(read_json_value "$fixture_path" lifecycle.runner_lifecycle.dedicated_job)" || return 1
  lifecycle_uid_sha256="$(read_json_value "$fixture_path" lifecycle.account_lifecycle.uid_sha256)" || return 1
  lifecycle_profile_sha256="$(read_json_value "$fixture_path" lifecycle.account_lifecycle.profile_id_sha256)" || return 1
  lifecycle_created_for_job="$(read_json_value "$fixture_path" lifecycle.account_lifecycle.created_for_job)" || return 1
  lifecycle_profile_created_for_job="$(read_json_value "$fixture_path" lifecycle.account_lifecycle.profile_created_for_job)" || return 1
  lifecycle_cleanup_attempted="$(read_json_value "$fixture_path" lifecycle.account_lifecycle.cleanup_attempted)" || return 1
  lifecycle_cleanup_attested="$(read_json_value "$fixture_path" lifecycle.account_lifecycle.cleanup_attested)" || return 1
  lifecycle_cleanup_succeeded="$(read_json_value "$fixture_path" lifecycle.account_lifecycle.cleanup_succeeded)" || return 1
  lifecycle_account_absent="$(read_json_value "$fixture_path" lifecycle.account_lifecycle.account_absent_after_cleanup)" || return 1
  lifecycle_profile_absent="$(read_json_value "$fixture_path" lifecycle.account_lifecycle.profile_absent_after_cleanup)" || return 1
  lifecycle_baseline_restored="$(read_json_value "$fixture_path" lifecycle.account_lifecycle.baseline_restored)" || return 1
  lifecycle_repository="$(read_json_value "$fixture_path" lifecycle.github.repository)" || return 1
  lifecycle_run_id="$(read_json_value "$fixture_path" lifecycle.github.run_id)" || return 1
  lifecycle_run_attempt="$(read_json_value "$fixture_path" lifecycle.github.run_attempt)" || return 1
  lifecycle_job="$(read_json_value "$fixture_path" lifecycle.github.job)" || return 1
  lifecycle_commit="$(read_json_value "$fixture_path" lifecycle.github.commit)" || return 1
  lifecycle_runner_name_sha256="$(read_json_value "$fixture_path" lifecycle.runner.name_sha256)" || return 1
  lifecycle_runner_image="$(read_json_value "$fixture_path" lifecycle.runner.image)" || return 1
  lifecycle_runner_tracking_sha256="$(read_json_value "$fixture_path" lifecycle.runner.tracking_id_sha256)" || return 1
  lifecycle_runner_architecture="$(read_json_value "$fixture_path" lifecycle.runner.architecture)" || return 1

  case "$fixture_case" in
    positive)
      ;;
    non-darwin)
      host_os_name="Linux"
      ;;
    wrong-arch)
      executable_architecture="x86_64"
      ;;
    artifact-mismatch)
      artifact_correlated=false
      ;;
    missing-codesign)
      codesign_verified=false
      codesign_deep=false
      codesign_strict=false
      codesign_developer_id=false
      ;;
    missing-notary)
      notary_stapled=false
      notary_validated=false
      ;;
    missing-gatekeeper)
      gatekeeper_accepted=false
      install_gatekeeper_accepted=false
      ;;
    system-install)
      install_scope="system"
      install_root_kind="SYSTEM_APPLICATIONS"
      ;;
    marker-only)
      lifecycle_created_for_job=false
      lifecycle_profile_created_for_job=false
      lifecycle_cleanup_attested=false
      lifecycle_cleanup_succeeded=false
      lifecycle_account_absent=false
      lifecycle_profile_absent=false
      marker_correlated=true
      ;;
    cleanup-missing)
      lifecycle_cleanup_attempted=false
      lifecycle_cleanup_attested=false
      lifecycle_cleanup_succeeded=false
      lifecycle_account_absent=false
      lifecycle_profile_absent=false
      ;;
    *)
      return 1
      ;;
  esac
}

function load_lifecycle_evidence() {
  local path="$1"
  [[ -f "$path" ]] || return 1
  lifecycle_runner_ephemeral="$(read_json_value "$path" runner_lifecycle.ephemeral)" || return 1
  lifecycle_runner_dedicated="$(read_json_value "$path" runner_lifecycle.dedicated_job)" || return 1
  lifecycle_uid_sha256="$(read_json_value "$path" account_lifecycle.uid_sha256)" || return 1
  lifecycle_profile_sha256="$(read_json_value "$path" account_lifecycle.profile_id_sha256)" || return 1
  lifecycle_created_for_job="$(read_json_value "$path" account_lifecycle.created_for_job)" || return 1
  lifecycle_profile_created_for_job="$(read_json_value "$path" account_lifecycle.profile_created_for_job)" || return 1
  lifecycle_cleanup_attempted="$(read_json_value "$path" account_lifecycle.cleanup_attempted)" || return 1
  lifecycle_cleanup_attested="$(read_json_value "$path" account_lifecycle.cleanup_attested)" || return 1
  lifecycle_cleanup_succeeded="$(read_json_value "$path" account_lifecycle.cleanup_succeeded)" || return 1
  lifecycle_account_absent="$(read_json_value "$path" account_lifecycle.account_absent_after_cleanup)" || return 1
  lifecycle_profile_absent="$(read_json_value "$path" account_lifecycle.profile_absent_after_cleanup)" || return 1
  lifecycle_baseline_restored="$(read_json_value "$path" account_lifecycle.baseline_restored)" || return 1
  lifecycle_repository="$(read_json_value "$path" github.repository)" || return 1
  lifecycle_run_id="$(read_json_value "$path" github.run_id)" || return 1
  lifecycle_run_attempt="$(read_json_value "$path" github.run_attempt)" || return 1
  lifecycle_job="$(read_json_value "$path" github.job)" || return 1
  lifecycle_commit="$(read_json_value "$path" github.commit)" || return 1
  lifecycle_runner_name_sha256="$(read_json_value "$path" runner.name_sha256)" || return 1
  lifecycle_runner_image="$(read_json_value "$path" runner.image)" || return 1
  lifecycle_runner_tracking_sha256="$(read_json_value "$path" runner.tracking_id_sha256)" || return 1
  lifecycle_runner_architecture="$(read_json_value "$path" runner.architecture)" || return 1
}

function load_live_facts() {
  [[ "$(uname -s)" == Darwin ]] || return 1
  [[ -d "$app_path" && -f "$artifact_path" &&
     -f "$install_evidence_path" && -f "$lifecycle_evidence_path" ]] ||
    return 1

  host_os_name="Darwin"
  host_os_major="$(/usr/bin/sw_vers -productVersion | /usr/bin/awk -F. '{print $1}')" || return 1
  host_architecture="$(/usr/bin/uname -m)" || return 1
  package_exists=true
  package_artifact_sha256="$(sha256_file "$artifact_path")" || return 1

  local temporary_root
  temporary_root="$(/usr/bin/mktemp -d "${TMPDIR:-/tmp}/gpteasy-macos-package.XXXXXX")" ||
    return 1
  if ! /usr/bin/ditto -x -k "$artifact_path" "$temporary_root" >/dev/null 2>&1; then
    /bin/rm -rf "$temporary_root"
    return 1
  fi
  typeset -a packaged_apps
  packaged_apps=("${(@f)$(/usr/bin/find "$temporary_root" -type d -name 'GPTEasy.app' -prune -print)}")
  if (( ${#packaged_apps} != 1 )); then
    /bin/rm -rf "$temporary_root"
    return 1
  fi
  local packaged_app_path="${packaged_apps[1]}"
  local info_plist="$packaged_app_path/Contents/Info.plist"
  local source_info_plist="$app_path/Contents/Info.plist"
  if [[ ! -f "$info_plist" || ! -f "$source_info_plist" ]]; then
    /bin/rm -rf "$temporary_root"
    return 1
  fi

  bundle_identifier="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$info_plist" 2>/dev/null)" || {
    /bin/rm -rf "$temporary_root"
    return 1
  }
  minimum_system_version="$(/usr/libexec/PlistBuddy -c 'Print :LSMinimumSystemVersion' "$info_plist" 2>/dev/null)" || {
    /bin/rm -rf "$temporary_root"
    return 1
  }
  local executable_name
  executable_name="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "$info_plist" 2>/dev/null)" || {
    /bin/rm -rf "$temporary_root"
    return 1
  }
  local source_executable_name
  source_executable_name="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "$source_info_plist" 2>/dev/null)" || {
    /bin/rm -rf "$temporary_root"
    return 1
  }
  local source_bundle_identifier
  source_bundle_identifier="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$source_info_plist" 2>/dev/null)" || {
    /bin/rm -rf "$temporary_root"
    return 1
  }
  local executable_path="$packaged_app_path/Contents/MacOS/$executable_name"
  local source_executable_path="$app_path/Contents/MacOS/$source_executable_name"
  if [[ ! -f "$executable_path" || ! -f "$source_executable_path" ]]; then
    /bin/rm -rf "$temporary_root"
    return 1
  fi
  package_executable_sha256="$(sha256_file "$executable_path")" || {
    /bin/rm -rf "$temporary_root"
    return 1
  }
  local source_executable_sha256
  source_executable_sha256="$(sha256_file "$source_executable_path")" || {
    /bin/rm -rf "$temporary_root"
    return 1
  }
  artifact_correlated=false
  if [[ "$source_bundle_identifier" == "$bundle_identifier" &&
        "$source_executable_sha256" == "$package_executable_sha256" ]]; then
    artifact_correlated=true
  fi
  local executable_architectures
  executable_architectures="$(/usr/bin/lipo -archs "$executable_path" 2>/dev/null)" || {
    /bin/rm -rf "$temporary_root"
    return 1
  }
  executable_architecture=""
  if [[ " $executable_architectures " == *" $expected_arch "* ]]; then
    executable_architecture="$expected_arch"
  fi

  codesign_verified=false
  codesign_deep=true
  codesign_strict=true
  if /usr/bin/codesign --verify --deep --strict --verbose=2 "$packaged_app_path" >/dev/null 2>&1; then
    codesign_verified=true
  fi
  local codesign_details
  codesign_details="$(/usr/bin/codesign -dv --verbose=4 "$packaged_app_path" 2>&1 || true)"
  codesign_developer_id=false
  if [[ "$codesign_details" == *"Authority=Developer ID Application:"* ]]; then
    codesign_developer_id=true
  fi
  local team_identifier
  team_identifier="$(
    print -r -- "$codesign_details" |
      /usr/bin/awk -F= '/^TeamIdentifier=/{print $2; exit}'
  )"
  codesign_team_sha256=""
  [[ -n "$team_identifier" ]] &&
    codesign_team_sha256="$(sha256_string "$team_identifier")"
  codesign_details=""
  team_identifier=""

  notary_stapled=false
  notary_validated=false
  if /usr/bin/xcrun stapler validate "$packaged_app_path" >/dev/null 2>&1; then
    notary_stapled=true
    notary_validated=true
  fi
  gatekeeper_accepted=false
  if /usr/sbin/spctl --assess --type execute --verbose=4 "$packaged_app_path" >/dev/null 2>&1; then
    gatekeeper_accepted=true
  fi
  /bin/rm -rf "$temporary_root"

  install_scope="$(read_json_value "$install_evidence_path" install.scope)" || return 1
  install_root_kind="$(read_json_value "$install_evidence_path" install.root_kind)" || return 1
  install_profile_sha256="$(read_json_value "$install_evidence_path" install.profile_id_sha256)" || return 1
  install_absolute_path_redacted="$(read_json_value "$install_evidence_path" install.absolute_path_redacted)" || return 1
  install_gatekeeper_accepted="$(read_json_value "$install_evidence_path" install.gatekeeper_accepted)" || return 1
  path_smoke_outcome="$(read_json_value "$install_evidence_path" path_smoke.outcome)" || return 1
  path_smoke_root_kind="$(read_json_value "$install_evidence_path" path_smoke.root_kind)" || return 1
  path_smoke_reopened="$(read_json_value "$install_evidence_path" path_smoke.reopened)" || return 1
  path_smoke_absolute_path_redacted="$(read_json_value "$install_evidence_path" path_smoke.absolute_path_redacted)" || return 1
  marker_correlated="$(read_json_value "$install_evidence_path" marker_correlated)" || return 1
  load_lifecycle_evidence "$lifecycle_evidence_path" || return 1
}

typeset -a checks
typeset -a blocking_reasons
checks=()
blocking_reasons=()

function add_check() {
  local name="$1"
  local passed="$2"
  local failure_code="$3"
  if [[ "$passed" == true ]]; then
    checks+=("{\"name\":$(json_quote "$name"),\"outcome\":\"passed\",\"code\":null}")
  else
    checks+=("{\"name\":$(json_quote "$name"),\"outcome\":\"failed\",\"code\":$(json_quote "$failure_code")}")
    blocking_reasons+=("$failure_code")
  fi
}

function evaluate_predicate() {
  local fixture_mode="$1"
  local passed=false

  local native_host=false
  if [[ "$host_os_name" == Darwin &&
        "$host_architecture" == "$expected_arch" ]] &&
     valid_decimal "$host_os_major" &&
     [[ "$host_os_major" -ge 14 ]]; then
    native_host=true
  fi
  add_check "native_macos" "$native_host" "MACOS_NATIVE_HOST_REQUIRED"

  local package_architecture_ok=false
  [[ "$executable_architecture" == "$expected_arch" ]] && package_architecture_ok=true
  add_check "package_architecture" "$package_architecture_ok" "MACOS_PACKAGE_ARCH_MISMATCH"

  local package_identity_ok=false
  if [[ "$package_exists" == true &&
        "$bundle_identifier" == "$EXPECTED_BUNDLE_IDENTIFIER" ]] &&
     version_major_at_least_14 "$minimum_system_version" &&
     valid_sha256 "$package_artifact_sha256" &&
     valid_sha256 "$package_executable_sha256"; then
    package_identity_ok=true
  fi
  add_check "package_identity" "$package_identity_ok" "MACOS_PACKAGE_IDENTITY_INVALID"

  local artifact_correlation_ok=false
  [[ "$artifact_correlated" == true ]] && artifact_correlation_ok=true
  add_check "artifact_correlation" "$artifact_correlation_ok" "MACOS_ARTIFACT_APP_MISMATCH"

  local codesign_ok=false
  if [[ "$codesign_verified" == true &&
        "$codesign_deep" == true &&
        "$codesign_strict" == true &&
        "$codesign_developer_id" == true ]] &&
     valid_sha256 "$codesign_team_sha256"; then
    codesign_ok=true
  fi
  add_check "codesign" "$codesign_ok" "MACOS_CODESIGN_INVALID"

  local notarization_ok=false
  [[ "$notary_stapled" == true &&
     "$notary_validated" == true ]] && notarization_ok=true
  add_check "notarization" "$notarization_ok" "MACOS_NOTARIZATION_INVALID"

  local gatekeeper_ok=false
  [[ "$gatekeeper_accepted" == true ]] && gatekeeper_ok=true
  add_check "gatekeeper" "$gatekeeper_ok" "MACOS_GATEKEEPER_REJECTED"

  local current_user_install=false
  if [[ "$install_scope" == currentUser &&
        "$install_root_kind" == "$EXPECTED_INSTALL_ROOT_KIND" &&
        "$install_absolute_path_redacted" == true &&
        "$install_gatekeeper_accepted" == true ]] &&
     valid_sha256 "$install_profile_sha256"; then
    current_user_install=true
  fi
  add_check "current_user_install" "$current_user_install" "MACOS_PACKAGE_NOT_CURRENT_USER"

  local path_smoke_ok=false
  [[ "$path_smoke_outcome" == passed &&
     "$path_smoke_root_kind" == app_local_data_dir &&
     "$path_smoke_reopened" == true &&
     "$path_smoke_absolute_path_redacted" == true ]] && path_smoke_ok=true
  add_check "path_smoke" "$path_smoke_ok" "MACOS_PATH_SMOKE_FAILED"

  local marker_ok=false
  [[ "$marker_correlated" == true ]] && marker_ok=true
  add_check "marker_correlation" "$marker_ok" "MACOS_MARKER_NOT_CORRELATED"

  local lifecycle_scoped=false
  if [[ "$lifecycle_runner_ephemeral" == true &&
        "$lifecycle_runner_dedicated" == true &&
        "$lifecycle_created_for_job" == true &&
        "$lifecycle_profile_created_for_job" == true ]] &&
     valid_sha256 "$lifecycle_uid_sha256" &&
     valid_sha256 "$lifecycle_profile_sha256" &&
     [[ "$install_profile_sha256" == "$lifecycle_profile_sha256" ]]; then
    lifecycle_scoped=true
  fi
  add_check "job_scoped_account" "$lifecycle_scoped" "MACOS_LIFECYCLE_NOT_ATTESTED"

  local cleanup_attempted=false
  [[ "$lifecycle_cleanup_attempted" == true ]] && cleanup_attempted=true
  add_check "cleanup_attempted" "$cleanup_attempted" "MACOS_LIFECYCLE_CLEANUP_MISSING"

  local cleanup_complete=false
  if [[ "$lifecycle_cleanup_attested" == true &&
        "$lifecycle_cleanup_succeeded" == true &&
        ( "$lifecycle_account_absent" == true &&
          "$lifecycle_profile_absent" == true ||
          "$lifecycle_baseline_restored" == true ) ]]; then
    cleanup_complete=true
  fi
  add_check "cleanup_attested" "$cleanup_complete" "MACOS_LIFECYCLE_NOT_ATTESTED"

  local identity_shape=false
  if [[ "$lifecycle_repository" == "$EXPECTED_REPOSITORY" &&
        "$lifecycle_runner_architecture" == "$expected_arch" ]] &&
     valid_decimal "$lifecycle_run_id" &&
     valid_decimal "$lifecycle_run_attempt" &&
     [[ "$lifecycle_run_attempt" -ge 1 ]] &&
     valid_job_name "$lifecycle_job" &&
     [[ ${#lifecycle_commit} -eq 40 ]] &&
     [[ -z "${lifecycle_commit//[0-9a-f]/}" ]] &&
     valid_sha256 "$lifecycle_runner_name_sha256" &&
     valid_sha256 "$lifecycle_runner_tracking_sha256"; then
    identity_shape=true
  fi
  add_check "identity_shape" "$identity_shape" "MACOS_GITHUB_IDENTITY_INVALID"

  local identity_binding=true
  if [[ "$fixture_mode" == false ]]; then
    identity_binding=false
    local current_runner_name_sha256=""
    local current_runner_tracking_sha256=""
    if [[ -n "${RUNNER_NAME:-}" && -n "${RUNNER_TRACKING_ID:-}" ]]; then
      current_runner_name_sha256="$(sha256_string "$RUNNER_NAME")"
      current_runner_tracking_sha256="$(sha256_string "$RUNNER_TRACKING_ID")"
    fi
    if [[ "$lifecycle_repository" == "${GITHUB_REPOSITORY:-}" &&
          "$lifecycle_run_id" == "${GITHUB_RUN_ID:-}" &&
          "$lifecycle_run_attempt" == "${GITHUB_RUN_ATTEMPT:-}" &&
          "$lifecycle_job" == "${GITHUB_JOB:-}" &&
          "$lifecycle_commit" == "${GITHUB_SHA:-}" &&
          "$lifecycle_runner_name_sha256" == "$current_runner_name_sha256" &&
          "$lifecycle_runner_tracking_sha256" == "$current_runner_tracking_sha256" &&
          "$lifecycle_runner_architecture" == "$(/usr/bin/uname -m)" ]]; then
      identity_binding=true
    fi
  fi
  add_check "identity_binding" "$identity_binding" "MACOS_GITHUB_IDENTITY_MISMATCH"

  (( ${#blocking_reasons} == 0 )) && passed=true
  local strict_gate_eligible=false
  [[ "$passed" == true && "$fixture_mode" == false ]] &&
    strict_gate_eligible=true
  local outcome="failed"
  local exit_code="$EXIT_SECURITY_BOUNDARY_FAILED"
  if [[ "$passed" == true ]]; then
    outcome="passed"
    exit_code="$EXIT_COMPLETED"
  fi

  typeset -a unique_reasons
  unique_reasons=("${(@u)blocking_reasons}")
  local checks_json="${(j:,:)checks}"
  local reasons_json=""
  if (( ${#unique_reasons} > 0 )); then
    typeset -a quoted_reasons
    quoted_reasons=()
    local reason
    for reason in "${unique_reasons[@]}"; do
      quoted_reasons+=("$(json_quote "$reason")")
    done
    reasons_json="${(j:,:)quoted_reasons}"
  fi

  print -r -- "{\"schema_version\":1,\"probe\":\"macos-package-contract\",\"outcome\":$(json_quote "$outcome"),\"exit_code\":${exit_code},\"strict_gate_eligible\":${strict_gate_eligible},\"test_only\":${fixture_mode},\"target_architecture\":$(json_quote "$expected_arch"),\"package\":{\"artifact_correlated\":${artifact_correlated},\"artifact_sha256\":$(json_quote "$package_artifact_sha256"),\"executable_sha256\":$(json_quote "$package_executable_sha256"),\"bundle_identifier\":$(json_quote "$bundle_identifier"),\"minimum_system_version\":$(json_quote "$minimum_system_version"),\"install_scope\":$(json_quote "$install_scope"),\"install_root_kind\":$(json_quote "$install_root_kind")},\"checks\":[${checks_json}],\"blocking_reasons\":[${reasons_json}]}"
  return "$exit_code"
}

while (( $# > 0 )); do
  case "$1" in
    --fixture-path)
      [[ $# -ge 2 ]] || emit_unavailable true "MACOS_PACKAGE_ARGUMENT_INVALID"
      fixture_path="$2"
      shift 2
      ;;
    --fixture-case)
      [[ $# -ge 2 ]] || emit_unavailable true "MACOS_PACKAGE_ARGUMENT_INVALID"
      fixture_case="$2"
      shift 2
      ;;
    --app-path)
      [[ $# -ge 2 ]] || emit_unavailable false "MACOS_PACKAGE_ARGUMENT_INVALID"
      app_path="$2"
      shift 2
      ;;
    --artifact-path)
      [[ $# -ge 2 ]] || emit_unavailable false "MACOS_PACKAGE_ARGUMENT_INVALID"
      artifact_path="$2"
      shift 2
      ;;
    --install-evidence-path)
      [[ $# -ge 2 ]] || emit_unavailable false "MACOS_PACKAGE_ARGUMENT_INVALID"
      install_evidence_path="$2"
      shift 2
      ;;
    --lifecycle-evidence-path)
      [[ $# -ge 2 ]] || emit_unavailable false "MACOS_PACKAGE_ARGUMENT_INVALID"
      lifecycle_evidence_path="$2"
      shift 2
      ;;
    --expected-arch)
      [[ $# -ge 2 ]] || emit_unavailable false "MACOS_PACKAGE_ARGUMENT_INVALID"
      expected_arch="$2"
      shift 2
      ;;
    *)
      emit_unavailable "$([[ -n "$fixture_path" ]] && print true || print false)" "MACOS_PACKAGE_ARGUMENT_INVALID"
      ;;
  esac
done

[[ "$expected_arch" == arm64 || "$expected_arch" == x86_64 ]] ||
  emit_unavailable "$([[ -n "$fixture_path" ]] && print true || print false)" "MACOS_PACKAGE_ARCHITECTURE_INVALID"

fixture_mode=false
if [[ -n "$fixture_path" ]]; then
  fixture_mode=true
  load_fixture ||
    emit_unavailable true "MACOS_PACKAGE_FIXTURE_INVALID"
else
  [[ -n "$app_path" &&
     -n "$artifact_path" &&
     -n "$install_evidence_path" &&
     -n "$lifecycle_evidence_path" ]] ||
    emit_unavailable false "MACOS_PACKAGE_LIVE_INPUT_MISSING"
  load_live_facts ||
    emit_unavailable false "MACOS_PACKAGE_VERIFICATION_UNAVAILABLE"
fi

evaluate_predicate "$fixture_mode"
exit "$?"
