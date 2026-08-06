#!/bin/zsh
set -euo pipefail

script_dir="${0:A:h}"
repository_root="${script_dir:h:h}"
lifecycle_guard="$script_dir/assert-macos-job-lifecycle.zsh"
package_verifier="$script_dir/run-macos.zsh"
self_script="$script_dir/test-macos-package-verifier.zsh"
positive_fixture="$repository_root/tests/fixtures/contracts/packaging/macos-positive-control.json"
expected_arch="arm64"

function fail_test() {
  print -u2 -- "macOS package verifier self-test failed; native package evidence was not claimed."
  exit 1
}

function assert_contains() {
  local value="$1"
  local expected="$2"
  [[ "$value" == *"$expected"* ]] || fail_test
}

function assert_not_contains() {
  local value="$1"
  local forbidden="$2"
  [[ "$value" != *"$forbidden"* ]] || fail_test
}

function invoke_case() {
  local fixture_case="$1"
  local expected_status="$2"
  local expected_reason="${3:-}"

  set +e
  REPLY="$(
    zsh "$package_verifier" \
      --fixture-path "$positive_fixture" \
      --fixture-case "$fixture_case" \
      --expected-arch "$expected_arch" \
      2>/dev/null
  )"
  REPLY_STATUS="$?"
  set -e

  [[ "$REPLY_STATUS" -eq "$expected_status" ]] || fail_test
  if [[ "$expected_status" -eq 0 ]]; then
    assert_contains "$REPLY" '"probe":"macos-package-contract"'
    assert_contains "$REPLY" '"outcome":"passed"'
    assert_contains "$REPLY" '"strict_gate_eligible":false'
    assert_contains "$REPLY" '"test_only":true'
  else
    assert_contains "$REPLY" '"outcome":"failed"'
    assert_contains "$REPLY" '"strict_gate_eligible":false'
    assert_contains "$REPLY" '"test_only":true'
    assert_contains "$REPLY" "$expected_reason"
  fi
  all_outputs+=("$REPLY")
}

for syntax_target in \
  "$lifecycle_guard" \
  "$package_verifier" \
  "$self_script"
do
  zsh -n "$syntax_target"
done

[[ -f "$positive_fixture" ]] || fail_test

typeset -a all_outputs
all_outputs=()

invoke_case positive 0
invoke_case non-darwin 5 MACOS_NATIVE_HOST_REQUIRED
invoke_case wrong-arch 5 MACOS_PACKAGE_ARCH_MISMATCH
invoke_case missing-codesign 5 MACOS_CODESIGN_INVALID
invoke_case missing-notary 5 MACOS_NOTARIZATION_INVALID
invoke_case missing-gatekeeper 5 MACOS_GATEKEEPER_REJECTED
invoke_case system-install 5 MACOS_PACKAGE_NOT_CURRENT_USER
invoke_case marker-only 5 MACOS_LIFECYCLE_NOT_ATTESTED
invoke_case cleanup-missing 5 MACOS_LIFECYCLE_CLEANUP_MISSING

typeset combined_output="${(j:\n:)all_outputs}"
for forbidden in \
  "APPLE_CERTIFICATE" \
  "APPLE_CERTIFICATE_PASSWORD" \
  "APPLE_API_KEY" \
  "APPLE_API_ISSUER" \
  "APPLE_API_KEY_ID" \
  "/Users/" \
  "$positive_fixture"
do
  assert_not_contains "$combined_output" "$forbidden"
done

print -r -- "macOS package verifier self-test passed: syntax, positive control, and 8 fail-closed cases"
