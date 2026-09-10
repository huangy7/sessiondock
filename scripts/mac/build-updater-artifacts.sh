#!/bin/bash
# build-updater-artifacts.sh
# Build macOS Tauri updater artifacts for both architectures and validate outputs
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

# Check required env vars
export TAURI_SIGNING_PRIVATE_KEY="${TAURI_SIGNING_PRIVATE_KEY:-}"
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}"

# Parse arguments
OUTPUT_DIR="${OUTPUT_DIR:-$PROJECT_ROOT/release-artifacts}"
BUILDER_SCRIPT="$SCRIPT_DIR/build-only.sh"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --sign)
      BUILDER_SCRIPT="$SCRIPT_DIR/build-and-notarize.sh"
      shift
      ;;
    --output-dir|-o)
      OUTPUT_DIR="$2"
      shift 2
      ;;
    *)
      shift
      ;;
  esac
done

# Clean and prepare output directory
echo "[INFO] Target output directory: $OUTPUT_DIR"
echo "[INFO] Cleaning output directory before build..."
rm -rf "$OUTPUT_DIR"
mkdir -p "$OUTPUT_DIR"

if [[ "$BUILDER_SCRIPT" == *"build-only.sh"* ]]; then
  echo "[INFO] Build mode: Ad-hoc / No Apple Sign (build-only.sh)"
  echo "[INFO] Skipping Apple Developer ID certificate & Apple notarization."
else
  echo "[INFO] Build mode: Apple Developer ID sign & Notarize (build-and-notarize.sh)"
fi

echo "[INFO] Building macOS arm64 (aarch64-apple-darwin)..."
bash "$BUILDER_SCRIPT" arm

echo "[INFO] Building macOS x64 (x86_64-apple-darwin)..."
bash "$BUILDER_SCRIPT" amd64

# Read version
VERSION=$(node -p "require('$PROJECT_ROOT/package.json').version")
echo "[INFO] Version: $VERSION"

# Validate artifacts using glob discovery (Tauri produces names without version/arch)
ARM64_BUNDLE_DIR="$PROJECT_ROOT/target/aarch64-apple-darwin/release/bundle/macos"
X64_BUNDLE_DIR="$PROJECT_ROOT/target/x86_64-apple-darwin/release/bundle/macos"

# Rename to include version and arch (since Tauri defaults to just AppName.app.tar.gz)
rename_artifact() {
  local dir="$1"
  local arch="$2"
  local old_gz
  old_gz=$(ls "$dir"/*.app.tar.gz 2>/dev/null | grep -v "${VERSION}" | head -1 || true)
  if [[ -n "$old_gz" ]]; then
    local new_gz="$dir/SessionDock_${VERSION}_${arch}.app.tar.gz"
    mv "$old_gz" "$new_gz"
    if [[ -f "${old_gz}.sig" ]]; then
      mv "${old_gz}.sig" "${new_gz}.sig"
    fi
  fi
}
rename_artifact "$ARM64_BUNDLE_DIR" "aarch64"
rename_artifact "$X64_BUNDLE_DIR" "x64"

ARM64_GZ=$(ls "$ARM64_BUNDLE_DIR"/*${VERSION}*aarch64.app.tar.gz 2>/dev/null | head -1 || true)
ARM64_SIG=$(ls "$ARM64_BUNDLE_DIR"/*${VERSION}*aarch64.app.tar.gz.sig 2>/dev/null | head -1 || true)
X64_GZ=$(ls "$X64_BUNDLE_DIR"/*${VERSION}*x64.app.tar.gz 2>/dev/null | head -1 || true)
X64_SIG=$(ls "$X64_BUNDLE_DIR"/*${VERSION}*x64.app.tar.gz.sig 2>/dev/null | head -1 || true)

for f in "$ARM64_GZ" "$ARM64_SIG" "$X64_GZ" "$X64_SIG"; do
  if [[ -z "$f" || ! -f "$f" ]]; then
    echo "[ERROR] Missing artifact: $f" >&2
    exit 1
  fi
  echo "[OK] $f"
done

ARM64_DMG_DIR="$PROJECT_ROOT/target/aarch64-apple-darwin/release/bundle/dmg"
X64_DMG_DIR="$PROJECT_ROOT/target/x86_64-apple-darwin/release/bundle/dmg"
ARM64_DMG=$(ls "$ARM64_DMG_DIR"/*${VERSION}*.dmg 2>/dev/null | head -1 || true)
X64_DMG=$(ls "$X64_DMG_DIR"/*${VERSION}*.dmg 2>/dev/null | head -1 || true)

echo ""
echo "[INFO] Copying artifacts to $OUTPUT_DIR ..."
cp "$ARM64_GZ" "$OUTPUT_DIR/"
cp "$ARM64_SIG" "$OUTPUT_DIR/"
cp "$X64_GZ" "$OUTPUT_DIR/"
cp "$X64_SIG" "$OUTPUT_DIR/"
if [[ -n "$ARM64_DMG" && -f "$ARM64_DMG" ]]; then
  cp "$ARM64_DMG" "$OUTPUT_DIR/"
fi
if [[ -n "$X64_DMG" && -f "$X64_DMG" ]]; then
  cp "$X64_DMG" "$OUTPUT_DIR/"
fi

echo ""
echo "============================================================"
echo "[SUCCESS] All release artifacts collected in: $OUTPUT_DIR"
echo "============================================================"
ls -lh "$OUTPUT_DIR"
echo ""
echo "Next step: Upload all files from $OUTPUT_DIR to GitHub Releases."


