#!/usr/bin/env bash
# install.sh — installs or updates osu-collect from the latest GitHub release
# supports: linux x64, macOS arm64 (Apple Silicon)
set -euo pipefail

# ── constants ────────────────────────────────────────────────────────────────

readonly REPO="uwuclxdy/osu-collect"
readonly API_URL="https://api.github.com/repos/${REPO}/releases/latest"
readonly INSTALL_DIR="${XDG_BIN_HOME:-$HOME/.local/bin}"
readonly BINARY_NAME="osu-collect"

# ── helpers ──────────────────────────────────────────────────────────────────

info()  { printf '==> %s\n' "$*"; }
error() { printf 'error: %s\n' "$*" >&2; }

cleanup() {
  local tmp="${1:-}"
  [[ -n "$tmp" && -d "$tmp" ]] && rm -rf -- "$tmp"
}

require_cmd() {
  command -v "$1" &>/dev/null || { error "required command not found: $1"; exit 1; }
}

# parse_field <json_string> <field>
# minimal jq-free JSON field extractor for simple string values
parse_field() {
  printf '%s' "$1" | grep -o "\"$2\":[[:space:]]*\"[^\"]*\"" | head -1 \
    | sed 's/.*":[[:space:]]*"\(.*\)"/\1/'
}

# ── os / arch detection ──────────────────────────────────────────────────────

detect_asset() {
  local os
  os="$(uname -s)"
  local arch
  arch="$(uname -m)"

  case "$os" in
    Linux)
      if [[ "$arch" != "x86_64" ]]; then
        error "unsupported architecture: $arch (only x64 is supported)"
        exit 1
      fi
      printf 'osu-collect-linux-x64'
      ;;
    Darwin)
      case "$arch" in
        arm64 | aarch64)
          printf 'osu-collect-macos-arm64'
          ;;
        *)
          error "unsupported macOS architecture: $arch (only Apple Silicon / arm64 is published)"
          info  "Intel Macs need an x64 build, which is not published yet"
          exit 1
          ;;
      esac
      ;;
    *)
      error "unsupported OS: $os"
      exit 1
      ;;
  esac
}

# ── fetch release metadata ───────────────────────────────────────────────────

fetch_latest_release() {
  require_cmd curl
  curl -fsSL --retry 3 "$API_URL"
}

# Emits three lines on stdout: tag, download URL, sha256 hex.
# Returns values via stdout rather than globals — a command-substitution
# subshell cannot write back to the caller's variables.
parse_release() {
  local json="$1"
  local asset_name="$2"

  local tag download_url digest
  if command -v jq &>/dev/null; then
    tag="$(printf '%s' "$json" | jq -r '.tag_name')"
    download_url="$(printf '%s' "$json" \
      | jq -r --arg n "$asset_name" '.assets[] | select(.name == $n) | .browser_download_url')"
    digest="$(printf '%s' "$json" \
      | jq -r --arg n "$asset_name" '.assets[] | select(.name == $n) | .digest // empty')"
  else
    tag="$(parse_field "$json" "tag_name")"
    # GitHub's per-asset block spans ~30 lines and carries both the download URL
    # and the `digest` GitHub computed ("sha256:<hex>"); -A5 is too short.
    download_url="$(printf '%s' "$json" \
      | grep -A30 "\"name\":.*\"${asset_name}\"" \
      | grep "browser_download_url" | head -1 \
      | sed 's/.*"browser_download_url":[[:space:]]*"\([^"]*\)".*/\1/')"
    digest="$(printf '%s' "$json" \
      | grep -A30 "\"name\":.*\"${asset_name}\"" \
      | grep '"digest"' | head -1 \
      | sed 's/.*"digest":[[:space:]]*"\([^"]*\)".*/\1/')"
  fi

  [[ -n "$tag" ]]          || { error "could not parse tag_name from release JSON"; exit 1; }
  [[ -n "$download_url" ]] || { error "asset '${asset_name}' not found in release ${tag}"; exit 1; }
  [[ -n "$digest" ]]       || { error "asset '${asset_name}' has no digest in release ${tag}"; exit 1; }

  # GitHub only emits sha256 today; refuse anything else rather than trust a
  # digest algorithm we don't verify against.
  case "$digest" in
    sha256:*) digest="${digest#sha256:}" ;;
    *) error "unexpected digest for '${asset_name}': ${digest}"; exit 1 ;;
  esac

  # the hash is compared as a plain hex string; make sure the jq-free parse
  # didn't hand us a stray JSON fragment instead of a real digest.
  [[ "$digest" =~ ^[0-9a-fA-F]{64}$ ]] \
    || { error "malformed digest for '${asset_name}': ${digest}"; exit 1; }

  printf '%s\n%s\n%s\n' "$tag" "$download_url" "$digest"
}

# ── sha256 verification ──────────────────────────────────────────────────────

verify_sha256() {
  local file="$1"
  local expected="$2"

  local actual
  if command -v sha256sum &>/dev/null; then
    actual="$(sha256sum "$file" | awk '{print $1}')"
  elif command -v shasum &>/dev/null; then
    actual="$(shasum -a 256 "$file" | awk '{print $1}')"
  else
    error "no sha256sum or shasum found — cannot verify download"
    exit 1
  fi

  # GitHub's digest is lowercase hex; normalize both sides before comparing
  expected="$(printf '%s' "$expected" | tr '[:upper:]' '[:lower:]')"
  actual="$(printf '%s' "$actual" | tr '[:upper:]' '[:lower:]')"

  if [[ "$actual" != "$expected" ]]; then
    error "sha256 mismatch"
    error "  expected: $expected"
    error "  actual:   $actual"
    return 1
  fi
}

# ── path advice ──────────────────────────────────────────────────────────────

check_path() {
  local dir="$1"
  case ":${PATH}:" in
    *":${dir}:"*) ;;
    *)
      info "${dir} is not in \$PATH"
      info "add this to your shell rc (~/.bashrc, ~/.zshrc, etc.):"
      # shellcheck disable=SC2016
      printf '    export PATH="%s:$PATH"\n' "$dir"
      ;;
  esac
}

# ── current install state ────────────────────────────────────────────────────

installed_hash() {
  local bin="${INSTALL_DIR}/${BINARY_NAME}"
  [[ -f "$bin" ]] || { printf ''; return; }
  if command -v sha256sum &>/dev/null; then
    sha256sum "$bin" | awk '{print $1}'
  elif command -v shasum &>/dev/null; then
    shasum -a 256 "$bin" | awk '{print $1}'
  else
    printf ''
  fi
}

# ── main ─────────────────────────────────────────────────────────────────────

main() {
  local asset_name
  asset_name="$(detect_asset)"

  info "fetching latest release info..."
  local json
  json="$(fetch_latest_release)"

  # parse_release prints tag / download URL / sha256 hex on three lines and
  # exits non-zero on failure; command substitution propagates that exit
  # under `set -e`, so a parse failure still aborts the installer.
  local release_info
  release_info="$(parse_release "$json" "$asset_name")"

  local tag DOWNLOAD_URL REMOTE_HASH
  { read -r tag; read -r DOWNLOAD_URL; read -r REMOTE_HASH; } <<< "$release_info"

  info "latest release: $tag"

  # download to a temp dir so we can verify before replacing
  local tmpdir
  tmpdir="$(mktemp -d)"
  trap 'cleanup "$tmpdir"' EXIT

  local tmp_bin="${tmpdir}/${asset_name}"

  # idempotency check: same hash already installed? GitHub's digest is lowercase,
  # as is installed_hash, so compare directly.
  local current_hash
  current_hash="$(installed_hash)"

  if [[ -n "$current_hash" && "$current_hash" == "$REMOTE_HASH" ]]; then
    info "already up to date ($tag)"
    exit 0
  fi

  info "downloading osu-collect $tag..."
  curl -fsSL --retry 3 -o "$tmp_bin" "$DOWNLOAD_URL"

  info "verifying checksum..."
  if ! verify_sha256 "$tmp_bin" "$REMOTE_HASH"; then
    rm -f -- "$tmp_bin"
    error "download aborted due to checksum failure"
    exit 1
  fi

  mkdir -p -- "$INSTALL_DIR"
  install -m 755 -- "$tmp_bin" "${INSTALL_DIR}/${BINARY_NAME}"
  info "installed to ${INSTALL_DIR}/${BINARY_NAME}"

  check_path "$INSTALL_DIR"

  info "done — run 'osu-collect' to start"
}

main "$@"
