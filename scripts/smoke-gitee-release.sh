#!/usr/bin/env bash

set -euo pipefail

: "${GITEE_TOKEN:?GITEE_TOKEN is required}"
: "${GITEE_REPOSITORY:?GITEE_REPOSITORY is required}"
: "${GITEE_DEFAULT_BRANCH:?GITEE_DEFAULT_BRANCH is required}"
: "${SMOKE_RUN_ID:?SMOKE_RUN_ID is required}"

for command in curl node sha256sum; do
  command -v "$command" >/dev/null 2>&1 || {
    printf 'missing required command: %s\n' "$command" >&2
    exit 2
  }
done

if [[ ! "$GITEE_REPOSITORY" =~ ^[^/[:space:]]+/[^/[:space:]]+$ ]]; then
  printf 'GITEE_REPOSITORY must use owner/repo form\n' >&2
  exit 2
fi
if [[ ! "$SMOKE_RUN_ID" =~ ^[0-9]+-[0-9]+$ ]]; then
  printf 'SMOKE_RUN_ID must use the GitHub run-attempt form\n' >&2
  exit 2
fi

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
JSON_TOOL="$SCRIPT_DIR/gitee-smoke-json.mjs"
CONTRACT_PATH="$SCRIPT_DIR/gitee-distribution.json"
CONTRACT_API_BASE=$(node "$JSON_TOOL" config "$CONTRACT_PATH" apiBaseUrl)
CONTRACT_RAW_BASE=$(node "$JSON_TOOL" config "$CONTRACT_PATH" rawBaseUrl)
FORMAL_MANIFEST_PATH=$(node "$JSON_TOOL" config "$CONTRACT_PATH" formalManifestPath)
SMOKE_MANIFEST_PREFIX=$(node "$JSON_TOOL" config "$CONTRACT_PATH" smokeManifestPrefix)
API_BASE="${GITEE_API_BASE_URL:-$CONTRACT_API_BASE}"
RAW_BASE="${GITEE_RAW_BASE_URL:-${CONTRACT_RAW_BASE}/${GITEE_REPOSITORY}/raw/${GITEE_DEFAULT_BRANCH}}"
if [[ "${GITEE_SMOKE_TEST_MODE:-0}" != "1" ]]; then
  [[ "$API_BASE" == "$CONTRACT_API_BASE" ]] || { printf 'custom API base is test-only\n' >&2; exit 2; }
  [[ "$RAW_BASE" == "${CONTRACT_RAW_BASE}/${GITEE_REPOSITORY}/raw/${GITEE_DEFAULT_BRANCH}" ]] || {
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
if [[ -n "${SMOKE_ASSET_PATH:-}" && -f "$SMOKE_ASSET_PATH" ]]; then
  cp "$SMOKE_ASSET_PATH" "$ASSET_PATH"
else
  # Keep the default fixture close to the current installer size without shipping binaries.
  dd if=/dev/zero of="$ASSET_PATH" bs=1M count="${SMOKE_ASSET_SIZE_MB:-4}" status=none
fi
ASSET_SHA256=$(sha256sum "$ASSET_PATH" | cut -d ' ' -f 1)
ASSET_SIZE=$(wc -c < "$ASSET_PATH")
ASSET_SIZE=$((ASSET_SIZE))
CURL_ASSET_PATH="$ASSET_PATH"
if command -v cygpath >/dev/null 2>&1; then
  CURL_ASSET_PATH=$(cygpath -w "$ASSET_PATH")
fi

request_form() {
  local method="$1" url="$2" output="$3"; shift 3
  local arguments=(--silent --show-error --location --request "$method"
    --header "Authorization: Bearer $GITEE_TOKEN" --header 'Accept: application/json'
    --output "$output" --write-out '%{http_code}')
  for field in "$@"; do arguments+=(--form-string "$field"); done
  local status; status=$(curl "${arguments[@]}" "$url")
  if [[ ! "$status" =~ ^2[0-9][0-9]$ ]]; then
    printf 'Gitee API request failed: %s %s returned %s\n' "$method" "$url" "$status" >&2
    node "$JSON_TOOL" api-error "$output" >&2 2>/dev/null || true
    exit 1
  fi
}

ANONYMOUS_MAX_ATTEMPTS=40
ANONYMOUS_RETRY_DELAY_SECONDS=5
if [[ "${GITEE_SMOKE_TEST_MODE:-0}" == "1" ]]; then
  ANONYMOUS_RETRY_DELAY_SECONDS="${GITEE_SMOKE_RETRY_DELAY_SECONDS:-0}"
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
request_form POST "$API_BASE/repos/$GITEE_REPOSITORY/releases" "$RELEASE_RESPONSE" \
  "tag_name=$SMOKE_TAG" "name=GPTEasy 分发冒烟 $SMOKE_TAG" \
  'body=非正式 API 冒烟资源，不是 GPTEasy 正式版本。' "prerelease=true" \
  "target_commitish=$GITEE_DEFAULT_BRANCH"
RELEASE_ID=$(node -e 'const r=require(process.argv[1]); const id=Number(r.id); if(!Number.isSafeInteger(id)||id<=0) process.exit(1); process.stdout.write(String(id))' "$RELEASE_RESPONSE")

UPLOAD_RESPONSE="$WORK_DIR/upload.json"
UPLOAD_STATUS=$(curl --silent --show-error --location --request POST \
  --header "Authorization: Bearer $GITEE_TOKEN" --header 'Accept: application/json' \
  --form "file=@$CURL_ASSET_PATH;filename=$ASSET_NAME" \
  --output "$UPLOAD_RESPONSE" --write-out '%{http_code}' \
  "$API_BASE/repos/$GITEE_REPOSITORY/releases/$RELEASE_ID/attach_files")
[[ "$UPLOAD_STATUS" =~ ^2[0-9][0-9]$ ]] || { printf 'Gitee attachment upload failed: HTTP %s\n' "$UPLOAD_STATUS" >&2; exit 1; }
ATTACHMENT_ID=$(node -e 'const r=require(process.argv[1]); const id=Number(r.id); if(!Number.isSafeInteger(id)||id<=0) process.exit(1); process.stdout.write(String(id))' "$UPLOAD_RESPONSE")

DOWNLOAD_PATH="$WORK_DIR/downloaded.txt"
DOWNLOAD_URL="https://gitee.com/$GITEE_REPOSITORY/releases/download/$SMOKE_TAG/$ASSET_NAME"
if [[ "${GITEE_SMOKE_TEST_MODE:-0}" == "1" ]]; then DOWNLOAD_URL="$API_BASE/repos/$GITEE_REPOSITORY/releases/$RELEASE_ID/attach_files/$ATTACHMENT_ID/download"; fi
RANGE_PATH="$WORK_DIR/range.bin"
RANGE_HEADERS="$WORK_DIR/range-headers.txt"
RANGE_RESULT=$(curl --silent --show-error --location --range 0-0 \
  --dump-header "$RANGE_HEADERS" --output "$RANGE_PATH" --write-out '%{http_code} %{size_download}' "$DOWNLOAD_URL")
RANGE_STATUS=${RANGE_RESULT%% *}
RANGE_BYTES=${RANGE_RESULT#* }
RANGE_BYTES=${RANGE_BYTES%%.*}
RANGE_CONTENT=$(tr -d '\r' < "$RANGE_HEADERS" | awk 'tolower($1) == "content-range:" { value=$2 " " $3 } END { print value }')
if [[ "$RANGE_STATUS" == 206 ]]; then
  [[ "$RANGE_BYTES" == 1 && $(wc -c < "$RANGE_PATH") -eq 1 ]] || { printf 'anonymous attachment range download did not return exactly one byte\n' >&2; exit 1; }
  [[ "$RANGE_CONTENT" == "bytes 0-0/$ASSET_SIZE" ]] || { printf 'anonymous attachment range response was %s\n' "${RANGE_CONTENT:-<missing>}" >&2; exit 1; }
elif [[ "$RANGE_STATUS" == 200 ]]; then
  RANGE_SHA256=$(sha256sum "$RANGE_PATH" | cut -d ' ' -f 1)
  [[ "$RANGE_BYTES" == "$ASSET_SIZE" && $(wc -c < "$RANGE_PATH") -eq "$ASSET_SIZE" ]] || { printf 'anonymous Range fallback did not return the complete attachment\n' >&2; exit 1; }
  [[ "$RANGE_SHA256" == "$ASSET_SHA256" ]] || { printf 'anonymous Range fallback SHA-256 does not match uploaded content\n' >&2; exit 1; }
else
  printf 'anonymous attachment range download failed: expected HTTP 206 or full HTTP 200, got %s\n' "$RANGE_STATUS" >&2
  exit 1
fi
download_anonymously "$DOWNLOAD_URL" "$DOWNLOAD_PATH" 'anonymous attachment download'
DOWNLOADED_SHA256=$(sha256sum "$DOWNLOAD_PATH" | cut -d ' ' -f 1)
DOWNLOADED_SIZE=$(wc -c < "$DOWNLOAD_PATH")
DOWNLOADED_SIZE=$((DOWNLOADED_SIZE))
if [[ "$DOWNLOADED_SIZE" -ne "$ASSET_SIZE" ]]; then
  printf 'anonymous attachment size does not match uploaded content\n' >&2
  exit 1
fi
if [[ "$DOWNLOADED_SHA256" != "$ASSET_SHA256" ]]; then
  printf 'anonymous attachment SHA-256 does not match uploaded content\n' >&2
  exit 1
fi

TEST_MANIFEST_PATH="$WORK_DIR/test-manifest.json"
node "$JSON_TOOL" manifest "$SMOKE_TAG" "$ASSET_NAME" "$ASSET_SHA256" > "$TEST_MANIFEST_PATH"
CONTENT_RESPONSE="$WORK_DIR/content.json"
CONTENT_BASE64=$(node -e 'const fs=require("node:fs"); process.stdout.write(fs.readFileSync(process.argv[1]).toString("base64"))' "$TEST_MANIFEST_PATH")
request_form POST "$API_BASE/repos/$GITEE_REPOSITORY/contents/$MANIFEST_PATH" "$CONTENT_RESPONSE" \
  "branch=$GITEE_DEFAULT_BRANCH" "message=test: record $SMOKE_TAG distribution smoke" "content=$CONTENT_BASE64"

CONTENT_READ_RESPONSE="$WORK_DIR/content-read.json"
ENCODED_BRANCH=$(node "$JSON_TOOL" urlencode "$GITEE_DEFAULT_BRANCH")
download_anonymously \
  "$API_BASE/repos/$GITEE_REPOSITORY/contents/$MANIFEST_PATH?ref=$ENCODED_BRANCH" \
  "$CONTENT_READ_RESPONSE" \
  'anonymous manifest metadata download'
RAW_MANIFEST_URL=$(node "$JSON_TOOL" download-url "$CONTENT_READ_RESPONSE" "$RAW_BASE")
RAW_RESULT="$WORK_DIR/raw.json"
download_anonymously "$RAW_MANIFEST_URL" "$RAW_RESULT" 'anonymous Raw manifest download'
node "$JSON_TOOL" verify-manifest "$RAW_RESULT" "$SMOKE_TAG" "$ASSET_SHA256"
node "$JSON_TOOL" report "$SMOKE_TAG" "$RELEASE_ID" "$DOWNLOAD_URL" "$RAW_MANIFEST_URL" \
  "$RANGE_STATUS" "$RANGE_BYTES" "$RANGE_CONTENT" "$DOWNLOADED_SIZE" "$DOWNLOADED_SHA256"
