#!/usr/bin/env bash
set -euo pipefail

: "${GITEE_TOKEN:?GITEE_TOKEN is required}"
: "${GITEE_REPOSITORY:?GITEE_REPOSITORY is required}"
: "${GITEE_DEFAULT_BRANCH:?GITEE_DEFAULT_BRANCH is required}"
: "${GITEE_API_BASE_URL:=https://api.gitee.com/api/v5}"

release_id="${1:-}"
tag="${2:-}"
[[ "$release_id" =~ ^[0-9]+$ && "$release_id" -gt 0 ]] || { printf 'usage: cleanup-gitee-release.sh <numeric-release-id> <exact-tag>\n' >&2; exit 2; }
[[ "$tag" =~ ^smoke-[0-9]+-[0-9]+$ ]] || { printf 'refusing to delete a non-smoke tag\n' >&2; exit 2; }

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
json_tool="$script_dir/gitee-smoke-json.mjs"
contract_path="$script_dir/gitee-distribution.json"
smoke_manifest_prefix=$(node "$json_tool" config "$contract_path" smokeManifestPrefix)
manifest_path="${smoke_manifest_prefix}${tag}.md"
encoded_branch=$(node "$json_tool" urlencode "$GITEE_DEFAULT_BRANCH")
release_url="$GITEE_API_BASE_URL/repos/$GITEE_REPOSITORY/releases/$release_id"
manifest_url="$GITEE_API_BASE_URL/repos/$GITEE_REPOSITORY/contents/$manifest_path"
response_file=$(mktemp)
trap 'rm -f "$response_file"' EXIT
status=$(curl --silent --show-error --location --request GET \
  --header "Authorization: Bearer $GITEE_TOKEN" --output "$response_file" --write-out '%{http_code}' "$release_url")
[[ "$status" == 200 ]] || { printf 'release lookup failed: HTTP %s\n' "$status" >&2; exit 1; }
actual_tag=$(node -e 'const fs = require("fs"); const value = JSON.parse(fs.readFileSync(process.argv[1], "utf8")); process.stdout.write(typeof value.tag_name === "string" ? value.tag_name : "")' "$response_file")
[[ "$actual_tag" == "$tag" ]] || { printf 'release ID %s belongs to tag %s, refusing to delete\n' "$release_id" "${actual_tag:-<missing>}" >&2; exit 1; }

status=$(curl --silent --show-error --location --request GET \
  --header "Authorization: Bearer $GITEE_TOKEN" --output "$response_file" --write-out '%{http_code}' \
  "$manifest_url?ref=$encoded_branch")
if [[ "$status" == 200 ]]; then
  manifest_sha=$(node "$json_tool" cleanup-metadata "$response_file" "$tag")
elif [[ "$status" == 404 ]]; then
  manifest_sha=""
else
  printf 'smoke manifest lookup failed: HTTP %s\n' "$status" >&2
  exit 1
fi

printf 'About to delete Gitee smoke manifest %s and release %s (ID %s).\n' "$manifest_path" "$tag" "$release_id"
read -r -p 'Type the exact tag to confirm: ' confirmation
[[ "$confirmation" == "$tag" ]] || { printf 'confirmation did not match; nothing deleted\n' >&2; exit 1; }

if [[ -n "$manifest_sha" ]]; then
  status=$(curl --silent --show-error --location --request DELETE \
    --header "Authorization: Bearer $GITEE_TOKEN" --output /dev/null --write-out '%{http_code}' \
    --form-string "branch=$GITEE_DEFAULT_BRANCH" \
    --form-string "message=test: remove $tag distribution smoke" \
    --form-string "sha=$manifest_sha" \
    "$manifest_url")
  [[ "$status" =~ ^2[0-9][0-9]$ ]] || { printf 'smoke manifest deletion failed: HTTP %s\n' "$status" >&2; exit 1; }
fi

status=$(curl --silent --show-error --location --request DELETE \
  --header "Authorization: Bearer $GITEE_TOKEN" --output /dev/null --write-out '%{http_code}' "$release_url")
[[ "$status" =~ ^2[0-9][0-9]$ ]] || { printf 'release deletion failed: HTTP %s\n' "$status" >&2; exit 1; }
printf 'Deleted Gitee smoke manifest and release %s (ID %s).\n' "$tag" "$release_id"
