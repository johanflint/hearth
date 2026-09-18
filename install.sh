#!/usr/bin/env sh
set -eu

REPOSITORY="johanflint/hearth"
APP_DIR="${HEARTH_HOME:-$HOME/.hearth}"
VERSION="${HEARTH_VERSION:-}"

log() {
  printf  '%s\n' "$*";
}

die() {
  printf '❌ %s\n' "$*" >&2;
  exit 1;
}

### Detect OS and arch, map to archive_os naming used by release-assets.yml
os="$(uname -s)"
arch="$(uname -m)"

case "$os" in
  Linux) case "$arch" in
    x86_64) archive_os="linux-x86_64" ;;
    aarch64) archive_os="linux-aarch64" ;;
    *) die "Unsupported Linux architecture '$arch'" ;;
  esac ;;
  Darwin) case "$arch" in
    x86_64) archive_os="macos-x86_64" ;;
    arm64) archive_os="macos-aarch64" ;;
    *) die "Unsupported macOS architecture '$arch'" ;;
  esac ;;
  *) die "Unsupported OS '$os', only Linux and macOS are supported"
esac

### Resolve version
if [ -z "$VERSION" ]; then
  log "👀 Looking up the latest release..."
  VERSION="$(curl -fsSL "https://api.github.com/repos/${REPOSITORY}/releases/latest" \
      | grep '"tag_name"' | head -n1 | sed -E 's/.*"tag_name": *"([^"]+)".*/\1/')"
  [ -n "$VERSION" ] || die "Could not determine the latest release, the GitHub API may be unreachable or rate-limited. Pass HEARTH_VERSION=<tag> to pin a version"
fi
log "📦 Installing hearth ${VERSION} (${archive_os})..."

archive="hearth-${VERSION}-${archive_os}.tar.gz"
base_url="https://github.com/${REPOSITORY}/releases/download/${VERSION}"

### Download to a scratch dir, always cleaned up
tmp_dir=
install_tmp=
install_complete=false
cleanup() {
  status=$?
  [ -z "$install_tmp" ] || rm -f "$install_tmp"
  [ -z "$tmp_dir" ] || rm -rf "$tmp_dir"
  if [ "$install_complete" != true ] && [ "$status" -eq 0 ]; then
    die "Installed input ended before completion"
  fi
  exit "$status"
}
trap cleanup EXIT
tmp_dir="$(mktemp -d)" || die "Could not create a temporary directory"

log "⏳ Downloading '${archive}'..."
curl -fsSL -o "${tmp_dir}/${archive}" "${base_url}/${archive}" \
  || die "Failed to download '${archive}', does release ${VERSION} exist?"

curl -fsSL -o "${tmp_dir}/SHA256SUMS" "${base_url}/SHA256SUMS" \
  || die "Failed to download 'SHA256SUMS' for ${VERSION}"

### Verify the checksum before touching the archive
log "🔍 Verifying checksum..."
expected="$(awk -v archive="$archive" '
  $2 == archive { matches++; sum = $1 }
  END { if (matches != 1 || length(sum) != 64) exit 1; print sum }
' "${tmp_dir}/SHA256SUMS")" || die "Expected exactly one checksum entry for '${archive}'"

if command -v sha256sum >/dev/null 2>&1; then
  actual="$(sha256sum "${tmp_dir}/${archive}" | awk '{print $1}')"
elif command -v shasum >/dev/null 2>&1; then
  actual="$(shasum -a 256 "${tmp_dir}/${archive}" | awk '{print $1}')"
else
  die "A SHA-256 tool is required: install sha256sum or shasum"
fi
[ "$expected" = "$actual" ] || die "Checksum mismatch for ${archive}: expected ${expected}, got ${actual}"
log "✅ Verifying checksum... OK"

### Extract now that the archive is verified
tar -xzf "${tmp_dir}/${archive}" -C "$tmp_dir"
extracted_dir="${tmp_dir}/hearth-${VERSION}-${archive_os}"
[ -x "${extracted_dir}/hearth" ] || die "Extracted archive did not contain an executable"

### Install
mkdir -p "$APP_DIR" || die "Could not create '${APP_DIR}'"
install_tmp="$(mktemp "${APP_DIR}/.hearth.XXXXXX")" || die "Could not stage binary"
cp "${extracted_dir}/hearth" "$install_tmp" || die "Could not stage binary, unable to copy"
chmod +x "$install_tmp" || die "Could not mark hearth as executable"
mv -f "$install_tmp" "${APP_DIR}/hearth" || die "Could not install hearth"
install_tmp=
install_complete=true

if [ ! -f "${APP_DIR}/config.local.json5" ]; then
  log "⚠️ No config.local.json5 found in ${APP_DIR} - hearth will run with default settings"
fi

log "✅ Installed hearth ${VERSION} to ${APP_DIR}"
log "Restart any already-running process to use this version"
log ""
log "Run it with:"
log " cd ${APP_DIR} && ./hearth"
