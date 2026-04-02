#!/usr/bin/env bash
# build-rust-android.sh — cross-compile aivpn-android-core for all Android ABI targets
# and copy the resulting .so files into the app's jniLibs directory.
#
# Prerequisites:
#   cargo install cargo-ndk
#   rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android
#
# Usage:
#   cd aivpn-android
#   ./build-rust-android.sh            # debug build (default)
#   ./build-rust-android.sh release    # release build

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
CRATE_DIR="${REPO_ROOT}/aivpn-android-core"
JNI_LIBS_DIR="${SCRIPT_DIR}/app/src/main/jniLibs"

BUILD_TYPE="${1:-debug}"

echo "==> Building aivpn-android-core [${BUILD_TYPE}]"

# Require Android NDK
if [[ -z "${ANDROID_NDK_HOME:-}" ]]; then
    # Common locations for NDK installed via Android Studio or command-line tools
    for candidate in \
        "${HOME}/Library/Android/sdk/ndk/latest" \
        "${HOME}/Library/Android/sdk/ndk/$(ls "${HOME}/Library/Android/sdk/ndk/" 2>/dev/null | sort -V | tail -1)" \
        "/usr/local/share/android-commandlinetools/ndk/latest" \
        "/opt/android-ndk"; do
        if [[ -d "${candidate}" ]]; then
            export ANDROID_NDK_HOME="${candidate}"
            break
        fi
    done
fi

if [[ -z "${ANDROID_NDK_HOME:-}" ]]; then
    echo "ERROR: ANDROID_NDK_HOME is not set and could not be auto-detected."
    echo "       Install the Android NDK and export ANDROID_NDK_HOME."
    exit 1
fi
echo "     NDK: ${ANDROID_NDK_HOME}"

# Confirm cargo-ndk is installed
if ! command -v cargo-ndk &>/dev/null; then
    echo "ERROR: cargo-ndk not found.  Run: cargo install cargo-ndk"
    exit 1
fi

RELEASE_FLAG=""
if [[ "${BUILD_TYPE}" == "release" ]]; then
    RELEASE_FLAG="--release"
fi

TARGETS=(
    "arm64-v8a:aarch64-linux-android"
    "armeabi-v7a:armv7-linux-androideabi"
    "x86_64:x86_64-linux-android"
)

for entry in "${TARGETS[@]}"; do
    ABI="${entry%%:*}"
    TARGET="${entry##*:}"
    echo "--> [${ABI}]  cargo ndk -t ${ABI}"

    (
        cd "${REPO_ROOT}"
        cargo ndk \
            -t "${ABI}" \
            -o "${JNI_LIBS_DIR}" \
            -- build ${RELEASE_FLAG} -p aivpn-android-core
    )

    echo "    Written to ${JNI_LIBS_DIR}/${ABI}/libaivpn_core.so"
done

echo ""
echo "==> Done.  .so files in ${JNI_LIBS_DIR}:"
find "${JNI_LIBS_DIR}" -name "libaivpn_core.so" -exec ls -lh {} \;
