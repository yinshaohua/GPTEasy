#!/usr/bin/env bash

set -euo pipefail

: "${GITCODE_TOKEN:?GITCODE_TOKEN is required}"
: "${GITCODE_REPOSITORY:?GITCODE_REPOSITORY is required}"
: "${GITCODE_DEFAULT_BRANCH:?GITCODE_DEFAULT_BRANCH is required}"
: "${SMOKE_RUN_ID:?SMOKE_RUN_ID is required}"

for command in curl node sha256sum; do
  command -v "$command" >/dev/null 2>&1 || {
    printf 'missing required command: %s\n' "$command" >&2
    exit 2
  }
done

if [[ ! "$GITCODE_REPOSITORY" =~ ^[^/[:space:]]+/[^/[:space:]]+$ ]]; then
  printf 'GITCODE_REPOSITORY must use owner/repo form\n' >&2
  exit 2
fi
if [[ ! "$SMOKE_RUN_ID" =~ ^[0-9]+-[0-9]+$ ]]; then
  printf 'SMOKE_RUN_ID must use the GitHub run-attempt form\n' >&2
  exit 2
fi

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
JSON_TOOL="$SCRIPT_DIR/gitcode-smoke-json.mjs"
CONTRACT_PATH="$SCRIPT_DIR/gitcode-distribution.json"
CONTRACT_API_BASE=$(node "$JSON_TOOL" config "$CONTRACT_PATH" apiBaseUrl)
CONTRACT_RAW_BASE=$(node "$JSON_TOOL" config "$CONTRACT_PATH" rawBaseUrl)
FORMAL_MANIFEST_PATH=$(node "$JSON_TOOL" config "$CONTRACT_PATH" formalManifestPath)
SMOKE_MANIFEST_PREFIX=$(node "$JSON_TOOL" config "$CONTRACT_PATH" smokeManifestPrefix)
API_BASE="${GITCODE_API_BASE_URL:-$CONTRACT_API_BASE}"
RAW_BASE="${GITCODE_RAW_BASE_URL:-${CONTRACT_RAW_BASE}/${GITCODE_REPOSITORY}/raw/${GITCODE_DEFAULT_BRANCH}}"
if [[ "${GITCODE_SMOKE_TEST_MODE:-0}" != "1" ]]; then
  [[ "$API_BASE" == "$CONTRACT_API_BASE" ]] || { printf 'custom API base is test-only\n' >&2; exit 2; }
  [[ "$RAW_BASE" == "${CONTRACT_RAW_BASE}/${GITCODE_REPOSITORY}/raw/${GITCODE_DEFAULT_BRANCH}" ]] || {
    printf 'custom Raw base is test-only\n' >&2
    exit 2
  }
fi
SMOKE_TAG="smoke-${SMOKE_RUN_ID}"
ASSET_NAME="gpteasy-${SMOKE_TAG}.txt"
MANIFEST_PATH="${SMOKE_MANIFEST_PREFIX}${SMOKE_TAG}.md"
[[ "$MANIFEST_PATH" != "$FORMAL_MANIFEST_PATH" ]] || { printf 'smoke manifest overlaps formal manifest\n' >&2; exit 2; }
WORK_DIR=$(mktemp -d)
trap 'rm -rf -- "$WORK_DIR"' EXIT

ASSET_PATH="$WORK_DIR/$ASSET_NAME"
printf 'GPTEasy GitCode distribution smoke %s\n' "$SMOKE_TAG" > "$ASSET_PATH"
ASSET_SHA256=$(sha256sum "$ASSET_PATH" | cut -d ' ' -f 1)

request_json() {
  local method="$1" url="$2" body="${3:-}" output="$4" status
  local arguments=(
    --silent --show-error --location
    --request "$method"
    --header "Authorization: Bearer $GITCODE_TOKEN"
    --header 'Accept: application/json'
    --output "$output"
    --write-out '%{http_code}'
  )
  if [[ -n "$body" ]]; then
    arguments+=(--header 'Content-Type: application/json' --data "$body")
  fi
  status=$(curl "${arguments[@]}" "$url")
  if [[ ! "$status" =~ ^2[0-9][0-9]$ ]]; then
    printf 'GitCode API request failed: %s %s returned %s\n' "$method" "$url" "$status" >&2
    node "$JSON_TOOL" api-error "$output" >&2 2>/dev/null || true
    exit 1
  fi
}

ANONYMOUS_MAX_ATTEMPTS=40
ANONYMOUS_RETRY_DELAY_SECONDS=5
if [[ "${GITCODE_SMOKE_TEST_MODE:-0}" == "1" ]]; then
  ANONYMOUS_RETRY_DELAY_SECONDS="${GITCODE_SMOKE_RETRY_DELAY_SECONDS:-0}"
fi

download_anonymously() {
  local url="$1" output="$2" description="$3" status attempt
  for attempt in $(seq 1 "$ANONYMOUS_MAX_ATTEMPTS"); do
    status=$(curl \
      --silent --show-error --location \
      --output "$output" \
      --write-out '%{http_code}' \
      "$url") || status=000
    if [[ "$status" =~ ^2[0-9][0-9]$ ]]; then
      return
    fi
    if (( attempt == ANONYMOUS_MAX_ATTEMPTS )) || [[ ! "$status" =~ ^(000|403|404|418|429|5[0-9][0-9])$ ]]; then
      printf '%s failed after %s attempt(s): HTTP %s\n' "$description" "$attempt" "$status" >&2
      exit 1
    fi
    sleep "$ANONYMOUS_RETRY_DELAY_SECONDS"
  done
}

RELEASE_RESPONSE="$WORK_DIR/release.json"
RELEASE_BODY=$(node "$JSON_TOOL" release-body "$SMOKE_TAG")
request_json POST "$API_BASE/repos/$GITCODE_REPOSITORY/releases" "$RELEASE_BODY" "$RELEASE_RESPONSE"

UPLOAD_RESPONSE="$WORK_DIR/upload-url.json"
ENCODED_ASSET_NAME=$(node "$JSON_TOOL" urlencode "$ASSET_NAME")
request_json GET "$API_BASE/repos/$GITCODE_REPOSITORY/releases/$SMOKE_TAG/upload_url?file_name=$ENCODED_ASSET_NAME" '' "$UPLOAD_RESPONSE"
node "$SCRIPT_DIR/gitcode-upload.mjs" \
  "$UPLOAD_RESPONSE" \
  "$ASSET_PATH" \
  "${GITCODE_SMOKE_TEST_MODE:-0}" >/dev/null

DOWNLOAD_PATH="$WORK_DIR/downloaded.txt"
DOWNLOAD_URL="$API_BASE/repos/$GITCODE_REPOSITORY/releases/$SMOKE_TAG/attach_files/$ASSET_NAME/download"
download_anonymously "$DOWNLOAD_URL" "$DOWNLOAD_PATH" 'anonymous attachment download'
DOWNLOADED_SHA256=$(sha256sum "$DOWNLOAD_PATH" | cut -d ' ' -f 1)
if [[ "$DOWNLOADED_SHA256" != "$ASSET_SHA256" ]]; then
  printf 'anonymous attachment SHA-256 does not match uploaded content\n' >&2
  exit 1
fi

TEST_MANIFEST_PATH="$WORK_DIR/test-manifest.json"
node "$JSON_TOOL" manifest "$SMOKE_TAG" "$ASSET_NAME" "$ASSET_SHA256" > "$TEST_MANIFEST_PATH"
CONTENT_BODY=$(node "$JSON_TOOL" content-body \
  "$GITCODE_DEFAULT_BRANCH" \
  "test: record $SMOKE_TAG distribution smoke" \
  "$TEST_MANIFEST_PATH")
CONTENT_RESPONSE="$WORK_DIR/content.json"
request_json POST "$API_BASE/repos/$GITCODE_REPOSITORY/contents/$MANIFEST_PATH" "$CONTENT_BODY" "$CONTENT_RESPONSE"

RAW_RESULT="$WORK_DIR/raw.json"
download_anonymously "$RAW_BASE/$MANIFEST_PATH" "$RAW_RESULT" 'anonymous Raw manifest download'
node "$JSON_TOOL" verify-manifest "$RAW_RESULT" "$SMOKE_TAG" "$ASSET_SHA256"
node "$JSON_TOOL" report "$SMOKE_TAG" "$DOWNLOAD_URL" "$RAW_BASE/$MANIFEST_PATH"
