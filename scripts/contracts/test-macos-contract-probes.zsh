#!/bin/zsh
set -euo pipefail

script_dir="${0:A:h}"
codex_probe="$script_dir/probe-codex-macos.zsh"
host_probe="$script_dir/probe-macos-host.zsh"
self_script="$script_dir/test-macos-contract-probes.zsh"
expected_arch="arm64"

function fail_test() {
  print -u2 -- "macOS contract probe self-test failed; native host evidence was not claimed."
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

function invoke_capture() {
  local script="$1"
  shift
  set +e
  REPLY="$(zsh "$script" "$@" 2>/dev/null)"
  REPLY_STATUS="$?"
  set -e
}

zsh -n "$script_dir/probe-codex-macos.zsh"
zsh -n "$script_dir/probe-macos-host.zsh"
zsh -n "$script_dir/test-macos-contract-probes.zsh"

typeset -a all_outputs
all_outputs=()

invoke_capture \
  "$codex_probe" \
  --role official_cli \
  --disposable-home /fixture/home \
  --working-directory /fixture/work \
  --fixture-case positive \
  --expected-version 0.146.1 \
  --expected-arch "$expected_arch"
[[ "$REPLY_STATUS" -eq 0 ]] || fail_test
assert_contains "$REPLY" '"probe":"codex-app-server-config-read"'
assert_contains "$REPLY" '"role":"official_cli"'
assert_contains "$REPLY" '"outcome":"passed"'
assert_contains "$REPLY" '"strict_gate_eligible":false'
assert_contains "$REPLY" '"test_only":true'
assert_contains "$REPLY" '"shared_user_layer":true'
all_outputs+=("$REPLY")

invoke_capture \
  "$host_probe" \
  --fixture-case positive \
  --expected-version 0.146.1 \
  --expected-arch "$expected_arch"
[[ "$REPLY_STATUS" -eq 0 ]] || fail_test
assert_contains "$REPLY" '"probe":"macos-host-codex-parity"'
assert_contains "$REPLY" '"outcome":"passed"'
assert_contains "$REPLY" '"strict_gate_eligible":false'
assert_contains "$REPLY" '"test_only":true'
assert_contains "$REPLY" '"all":true'
assert_contains "$REPLY" '"shared_user_layer":true'
all_outputs+=("$REPLY")

invoke_capture \
  "$host_probe" \
  --fixture-case host_missing \
  --expected-version 0.146.1 \
  --expected-arch "$expected_arch"
[[ "$REPLY_STATUS" -eq 3 ]] || fail_test
assert_contains "$REPLY" '"outcome":"blocked"'
assert_contains "$REPLY" '"HOST_CODEX_MISSING"'
all_outputs+=("$REPLY")

typeset fixture_case=""
for fixture_case in \
  wrong_arch \
  root_mismatch \
  origin_mismatch \
  provider_mismatch \
  carrier_mismatch
do
  invoke_capture \
    "$host_probe" \
    --fixture-case "$fixture_case" \
    --expected-version 0.146.1 \
    --expected-arch "$expected_arch"
  [[ "$REPLY_STATUS" -eq 5 ]] || fail_test
  assert_contains "$REPLY" '"outcome":"failed"'
  assert_contains "$REPLY" '"strict_gate_eligible":false'
  assert_contains "$REPLY" '"all":false'
  all_outputs+=("$REPLY")
done

typeset combined_output="${(j:\n:)all_outputs}"
for forbidden in \
  "GPTEASY-CONTRACT-CANARY-NONSECRET-01-12" \
  "experimental_bearer_token" \
  "Authorization:" \
  '"config":' \
  "/Users/" \
  "/fixture/home" \
  "/fixture/work"
do
  assert_not_contains "$combined_output" "$forbidden"
done

print -r -- "macOS contract probe self-test passed: zsh syntax, native/arch checks, shared user-layer parity, and 6 fail-closed cases"
