#!/usr/bin/env bash
# tests: Nekoha API - GET /api/beatmapset/{id}
# rust source: src/download/size_fetcher.rs
# used to fetch file_size for download progress estimation
# model: BeatmapsetResponse { file_size: Option<u64> } (may be string or number per deserializer)

set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/common.sh"

BASE="https://mirror.nekoha.moe/api"
URL="$BASE/beatmapset/$BEATMAPSET_ID"

echo "=== Nekoha size API: GET $URL ==="

resp=$(curl -s -w "\n%{http_code}" -H "User-Agent: $UA" "$URL")
status=$(echo "$resp" | tail -1)
body=$(echo "$resp" | head -n -1)

check_http_status "$status" 200 "Nekoha beatmapset fetch" || { echo "$body" | head -5; exit 1; }

file_size_raw=$(echo "$body" | jq -r '.file_size // "__missing__"' 2>/dev/null)
if [[ "$file_size_raw" == "__missing__" ]]; then
    file_size_raw=$(echo "$body" | jq -r '.fileSize // "__missing__"' 2>/dev/null)
    if [[ "$file_size_raw" != "__missing__" ]]; then
        fail "Nekoha beatmapset: field name is 'fileSize' but rust expects 'file_size'"
    else
        fail "Nekoha beatmapset: no file_size field in response"
        echo "response keys: $(echo "$body" | jq 'keys' 2>/dev/null || echo "$body" | head -3)"
        echo ""
        echo "result: FAIL ($FAILURES failure(s))"
        exit 1
    fi
else
    if echo "$file_size_raw" | grep -qE '^[0-9]+$'; then
        pass "file_size is numeric: $file_size_raw bytes"
    elif [[ "$file_size_raw" == "null" ]]; then
        pass "file_size is null (beatmapset may be missing) - rust handles Option<u64>"
    else
        pass "file_size is string: '$file_size_raw' (rust deserializer handles string->u64)"
    fi
fi

echo ""
if [[ $FAILURES -eq 0 ]]; then
    echo "result: PASS"
else
    echo "result: FAIL ($FAILURES failure(s))"
    exit 1
fi
