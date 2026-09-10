#!/bin/bash
set -e

# ============================================================
# Claudia — Build Only Script (macOS, no sign / no notarize)
# ============================================================
# Usage:
#   ./scripts/mac/build-only.sh              # default: universal
#   ./scripts/mac/build-only.sh arm
#   ./scripts/mac/build-only.sh amd64
#   ./scripts/mac/build-only.sh universal
#   ./scripts/mac/build-only.sh --devtools   # enable DevTools (Cmd+Option+I)
# ============================================================

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "${SCRIPT_DIR}/../.." && pwd)"
CARGO_TOML="${PROJECT_DIR}/src-tauri/Cargo.toml"

info()  { echo "[INFO]  $*"; }
error() { echo "[ERROR] $*" >&2; exit 1; }

export TAURI_SIGNING_PRIVATE_KEY="dW50cnVzdGVkIGNvbW1lbnQ6IHJzaWduIGVuY3J5cHRlZCBzZWNyZXQga2V5ClJXUlRZMEl5Ymxvazh2a3I2QmxsNCtFOWRvaThWRU9ZS3pUVUU3TG4wWDFZWmN1bkoxZ0FBQkFBQUFBQUFBQUFBQUlBQUFBQUEyNTl5WXRxcmkwZGtZQ0ZPTS90ZjV2TUlURkxYUm5zcHVEV0dWNTA5cXcyK3FoeUZEQjlsWFl0T0U4Q1JGNHY4RlZOT0NnQnZPOXh2d0tWZHVxK2FFVHBWTm1FTktYaS9sRVZ3dWIwRE8ySzlCR1dEK3RtOC9JVHBPenFUTnJJVVNRc2VrQ05Eb0E9Cg=="
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="claudia"

# ---- Parse arguments ----
DEVTOOLS=0
ARCH="${ARCH:-universal}"
for arg in "$@"; do
    case "$arg" in
        --devtools) DEVTOOLS=1 ;;
        *)          ARCH="$arg" ;;
    esac
done

# ---- Patch / restore Cargo.toml devtools feature ----
patch_devtools() {
    sed -i '' 's/\(tauri = {[^}]*features = \["\)/\1devtools", "/' "${CARGO_TOML}"
    info "devtools feature added to Cargo.toml"
}

restore_devtools() {
    sed -i '' 's/"devtools", //' "${CARGO_TOML}"
    info "devtools feature removed from Cargo.toml"
}

case "${ARCH}" in
    arm|arm64|aarch64)
        TARGET="aarch64-apple-darwin"
        ;;
    amd64|x86_64|x64|intel)
        TARGET="x86_64-apple-darwin"
        ;;
    universal)
        TARGET="universal-apple-darwin"
        ;;
    *)
        error "Unknown architecture: ${ARCH}. Use: arm, amd64, or universal"
        ;;
esac

BUNDLE_DIR="${PROJECT_DIR}/target/${TARGET}/release/bundle"

clean_bundle() {
    info "Cleaning old bundle cache for ${TARGET}..."
    rm -rf "${PROJECT_DIR}/target/${TARGET}/release/bundle"
    info "Bundle cache cleaned."
}

build() {
    info "Building claudia-proxy for ${TARGET}..."
    "${SCRIPT_DIR}/build-proxy.sh" "${TARGET}"

    info "Building Claudia with Tauri (signing disabled) ..."
    cd "${PROJECT_DIR}"
    CLAUDIA_SKIP_PROXY_BUILD=1 npm run tauri build -- --target "${TARGET}"
    info "Build completed."
}

find_dmg() {
    local dmg_path
    dmg_path="$(ls "${BUNDLE_DIR}"/dmg/Claudia_*.dmg 2>/dev/null | head -1)"
    if [ -z "$dmg_path" ]; then
        error "DMG not found in ${BUNDLE_DIR}/dmg/"
    fi
    info "DMG: ${dmg_path}"
    ls -lh "${dmg_path}"
}

main() {
    info "=== Claudia Build Only (no sign, no notarize) ==="
    info "Target: ${TARGET}"
    [ "$DEVTOOLS" = "1" ] && info "DevTools: ENABLED"

    # Unset signing identity so Tauri skips code signing
    unset APPLE_SIGNING_IDENTITY

    [ "$DEVTOOLS" = "1" ] && patch_devtools
    trap '[ "$DEVTOOLS" = "1" ] && restore_devtools' EXIT

    clean_bundle
    build
    find_dmg

    info "=== Done! ==="
}

main
