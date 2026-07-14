#!/usr/bin/env bash
# tests: all mirror download endpoints - verifies ZIP magic bytes (first 4 bytes)
# rust source: src/download/size_fetcher.rs (probe_mirror), osu-downloader/src/mirrors/mod.rs
# mirrors tested:
#   - nerinyan                        https://api.nerinyan.moe/d/{id}
#   - osu.direct download template    https://osu.direct/d/{id}
#   - osu.direct MIRROR_CHECK_URLS    https://osu.direct/api/d/{id}  (different path!)
#   - sayobot                         https://dl.sayobot.cn/beatmaps/download/full/{id}
#   - nekoha                          https://mirror.nekoha.moe/api/download/{id}
#   - beatconnect (anon)             https://beatconnect.io/b/{id}/
#   - hinamizawa (cascade)           https://mirror.hinamizawa.ai/api/v1/hinai/d/{id}

set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/common.sh"

ZIP_MAGIC="504b0304"

probe_zip() {
    local label="$1" url="$2"
    echo "  probing: $url"

    local tmpfile
    tmpfile=$(mktemp)
    local http_status
    http_status=$(curl -s -L \
        -r 0-3 \
        -H "User-Agent: $UA" \
        -o "$tmpfile" \
        -w "%{http_code}" \
        --max-time 20 \
        "$url" 2>/dev/null) || true

    if [[ -z "$http_status" || "$http_status" == "000" ]]; then
        rm -f "$tmpfile"
        echo "  SKIP: $label: cannot connect (network unreachable)"
        return
    fi

    if [[ "$http_status" == "429" ]]; then
        rm -f "$tmpfile"
        echo "  SKIP: $label: rate limited (HTTP 429)"
        return
    fi

    if [[ "$http_status" == "502" || "$http_status" == "503" || "$http_status" == "504" ]]; then
        rm -f "$tmpfile"
        echo "  SKIP: $label: mirror server error (HTTP $http_status) - upstream issue"
        return
    fi

    if [[ "$http_status" -ge 400 ]]; then
        rm -f "$tmpfile"
        fail "$label: HTTP $http_status"
        return
    fi

    local magic
    magic=$(xxd -p "$tmpfile" 2>/dev/null | tr -d '[:space:]' | head -c 8 || true)
    rm -f "$tmpfile"

    if [[ "${magic:0:8}" == "$ZIP_MAGIC" ]]; then
        pass "$label: ZIP magic ok (HTTP $http_status)"
    else
        fail "$label: bad magic bytes (got: '${magic:-empty}', HTTP $http_status)"
    fi
}

echo "=== mirror download endpoints (beatmapset $BEATMAPSET_ID) ==="
echo ""

echo "--- download templates (used for actual downloads in osu-downloader) ---"
probe_zip "nerinyan" "https://api.nerinyan.moe/d/$BEATMAPSET_ID"
probe_zip "osu.direct (download template: /d/)" "https://osu.direct/d/$BEATMAPSET_ID"
probe_zip "sayobot" "https://dl.sayobot.cn/beatmaps/download/full/$BEATMAPSET_ID"
probe_zip "nekoha" "https://mirror.nekoha.moe/api/download/$BEATMAPSET_ID"
probe_zip "beatconnect (anon /b/)" "https://beatconnect.io/b/$BEATMAPSET_ID/"
probe_zip "hinamizawa (cascade /api/v1/hinai/d/)" "https://mirror.hinamizawa.ai/api/v1/hinai/d/$BEATMAPSET_ID"

echo ""
echo "--- MIRROR_CHECK_URLS constants (src/config/constants.rs) ---"
probe_zip "osu.direct (check URL: /api/d/)" "https://osu.direct/api/d/$BEATMAPSET_ID"

echo ""
if [[ $FAILURES -eq 0 ]]; then
    echo "result: PASS"
else
    echo "result: FAIL ($FAILURES failure(s))"
    exit 1
fi
