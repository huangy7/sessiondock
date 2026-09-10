#!/bin/bash
set -e

# ============================================================
# Claudia — Build, Sign & Notarize Script (macOS)
# ============================================================
# Usage:
#   ./scripts/mac/build-and-notarize.sh              # default: universal
#   ./scripts/mac/build-and-notarize.sh arm
#   ./scripts/mac/build-and-notarize.sh amd64
#   ./scripts/mac/build-and-notarize.sh universal
#
# Prerequisites:
#   1. Apple Developer ID Application certificate in Keychain
#   2. Environment variables (or modify the defaults below):
#      - APPLE_SIGNING_IDENTITY
#      - APPLE_ID
#      - APPLE_APP_PASSWORD  (app-specific password)
#      - APPLE_TEAM_ID
# ============================================================

export APPLE_SIGNING_IDENTITY="${APPLE_SIGNING_IDENTITY:-}"
APPLE_ID="${APPLE_ID:-}"
APPLE_APP_PASSWORD="${APPLE_APP_PASSWORD:-}"
APPLE_TEAM_ID="${APPLE_TEAM_ID:-}"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "${SCRIPT_DIR}/../.." && pwd)"
DMG_PATH=""  # will be detected
export TAURI_SIGNING_PRIVATE_KEY="${TAURI_SIGNING_PRIVATE_KEY:-}"
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}"

# ---- Parse arch argument ----
ARCH="${1:-${ARCH:-universal}}"
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
        echo "[ERROR] Unknown architecture: ${ARCH}. Use: arm, amd64, or universal" >&2
        exit 1
        ;;
esac

BUNDLE_DIR="${PROJECT_DIR}/target/${TARGET}/release/bundle"

# ---- Helpers ----
info()  { echo "[INFO]  $*"; }
error() { echo "[ERROR] $*" >&2; exit 1; }

# ---- Step 0: Clean old bundle cache to ensure version update ----
clean_bundle() {
    info "Cleaning old bundle cache for ${TARGET}..."
    rm -rf "${PROJECT_DIR}/target/${TARGET}/release/bundle"
    info "Bundle cache cleaned."
}

# ---- Step 1: Build (Tauri auto-signs .app via APPLE_SIGNING_IDENTITY) ----
build() {
    info "Building claudia-proxy for ${TARGET}..."
    "${SCRIPT_DIR}/build-proxy.sh" "${TARGET}"

    info "Building Claudia with Tauri (auto-sign enabled) ..."
    cd "${PROJECT_DIR}"
    CLAUDIA_SKIP_PROXY_BUILD=1 npm run tauri build -- --target "${TARGET}"
    info "Build completed."
}

# ---- Step 2: Find DMG ----
find_dmg() {
    DMG_PATH="$(ls "${BUNDLE_DIR}"/dmg/Claudia_*.dmg 2>/dev/null | head -1)"
    if [ -z "$DMG_PATH" ]; then
        error "DMG not found in ${BUNDLE_DIR}/dmg/"
    fi
    info "Found DMG: ${DMG_PATH}"
}

# ---- Step 3: Sign DMG ----
sign_dmg() {
    info "Signing DMG: ${DMG_PATH} ..."
    codesign --timestamp --options=runtime --force \
        -s "${APPLE_SIGNING_IDENTITY}" \
        "${DMG_PATH}"
    info "DMG signed."
}

# ---- Step 4: Notarize ----
notarize() {
    info "Submitting DMG for notarization..."
    local result
    result="$(xcrun notarytool submit \
        --apple-id "${APPLE_ID}" \
        --password "${APPLE_APP_PASSWORD}" \
        --team-id "${APPLE_TEAM_ID}" \
        "${DMG_PATH}" \
        --wait \
        --output-format json)"

    echo "${result}"

    local status
    status="$(echo "${result}" | python3 -c "import sys,json; print(json.load(sys.stdin).get('status',''))" 2>/dev/null || true)"

    if [ "${status}" != "Accepted" ]; then
        error "Notarization failed! Status: ${status}"
    fi
    info "Notarization accepted."
}

# ---- Step 5: Staple ----
staple() {
    info "Stapling notarization ticket to DMG..."
    local retries=0
    while ! xcrun stapler staple "${DMG_PATH}"; do
        retries=$((retries + 1))
        if [ $retries -ge 6 ]; then
            error "Stapling failed after ${retries} attempts."
        fi
        info "Staple not ready, retrying in 10s... (attempt ${retries}/6)"
        sleep 10
    done
    info "DMG stapled successfully."
}

# ---- Step 6: Verify ----
verify() {
    info "Verifying notarization..."
    spctl --assess --type open --context context:primary-signature -v "${DMG_PATH}" 2>&1 || true
    info "Final DMG: ${DMG_PATH}"
    ls -lh "${DMG_PATH}"
}

# ---- Main ----
main() {
    info "=== Claudia Build + Sign + Notarize ==="
    info "Target: ${TARGET}"
    info "Signing Identity: ${APPLE_SIGNING_IDENTITY}"
    info "Apple ID: ${APPLE_ID}"
    info "Team ID: ${APPLE_TEAM_ID}"
    echo ""

    clean_bundle
    build
    find_dmg
    sign_dmg
    notarize
    staple
    verify

    echo ""
    info "=== Done! ==="
    info "Distributable DMG: ${DMG_PATH}"
}

main
