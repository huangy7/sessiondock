#!/bin/bash
set -e

# Build sessiondock-proxy as a universal macOS binary and place it
# where Tauri's externalBin expects it.

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "${SCRIPT_DIR}/../.." && pwd)"
BINARIES_DIR="${PROJECT_DIR}/src-tauri/binaries"

mkdir -p "${BINARIES_DIR}"

TARGET="${1:-}"

if [ "${TARGET}" = "universal-apple-darwin" ]; then
    echo "[proxy] Building sessiondock-proxy for x86_64-apple-darwin..."
    cargo build -p sessiondock-proxy --release --target x86_64-apple-darwin

    echo "[proxy] Building sessiondock-proxy for aarch64-apple-darwin..."
    cargo build -p sessiondock-proxy --release --target aarch64-apple-darwin

    echo "[proxy] Copying per-arch binaries for Tauri..."
    cp "${PROJECT_DIR}/target/x86_64-apple-darwin/release/sessiondock-proxy" \
       "${BINARIES_DIR}/sessiondock-proxy-x86_64-apple-darwin"
    cp "${PROJECT_DIR}/target/aarch64-apple-darwin/release/sessiondock-proxy" \
       "${BINARIES_DIR}/sessiondock-proxy-aarch64-apple-darwin"

    echo "[proxy] Creating universal binary with lipo..."
    lipo -create \
        "${PROJECT_DIR}/target/x86_64-apple-darwin/release/sessiondock-proxy" \
        "${PROJECT_DIR}/target/aarch64-apple-darwin/release/sessiondock-proxy" \
        -output "${BINARIES_DIR}/sessiondock-proxy-universal-apple-darwin"

    echo "[proxy] Binaries created at ${BINARIES_DIR}/"
elif [ -n "${TARGET}" ]; then
    echo "[proxy] Building sessiondock-proxy for ${TARGET}..."
    cargo build -p sessiondock-proxy --release --target "${TARGET}"
    cp "${PROJECT_DIR}/target/${TARGET}/release/sessiondock-proxy" \
       "${BINARIES_DIR}/sessiondock-proxy-${TARGET}"
    echo "[proxy] Binary created at ${BINARIES_DIR}/sessiondock-proxy-${TARGET}"
else
    # Default: build for current platform (dev mode)
    echo "[proxy] Building sessiondock-proxy (current platform)..."
    cargo build -p sessiondock-proxy --release
    TRIPLE="$(rustc --print host-tuple 2>/dev/null || rustc -vV | sed -n 's/host: //p')"
    cp "${PROJECT_DIR}/target/release/sessiondock-proxy" \
       "${BINARIES_DIR}/sessiondock-proxy-${TRIPLE}"
    echo "[proxy] Binary created at ${BINARIES_DIR}/sessiondock-proxy-${TRIPLE}"
fi
