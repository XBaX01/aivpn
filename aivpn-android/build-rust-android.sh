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
RELEASES_DIR="${REPO_ROOT}/releases"
APK_DST="${RELEASES_DIR}/aivpn-client.apk"

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
)

# Ensure stale x86_64 output from previous builds is not packaged into APK.
rm -rf "${JNI_LIBS_DIR}/x86_64"

for entry in "${TARGETS[@]}"; do
    ABI="${entry%%:*}"
    TARGET="${entry##*:}"
    echo "--> [${ABI}]  cargo ndk -t ${ABI}"

    (
        cd "${REPO_ROOT}"
        cargo ndk \
            -t "${ABI}" \
            -o "${JNI_LIBS_DIR}" \
            -- build -p aivpn-android-core \
            ${RELEASE_FLAG}
    )

    echo "    Written to ${JNI_LIBS_DIR}/${ABI}/libaivpn_core.so"
done

echo ""
echo "==> Done.  .so files in ${JNI_LIBS_DIR}:"
find "${JNI_LIBS_DIR}" -name "libaivpn_core.so" -exec ls -lh {} \;

echo ""
echo "==> Building Android APK and publishing to releases/..."
mkdir -p "${RELEASES_DIR}"

if [[ ! -x "${SCRIPT_DIR}/gradlew" ]]; then
    chmod +x "${SCRIPT_DIR}/gradlew" || true
fi

if [[ "${BUILD_TYPE}" == "release" ]]; then
    (
        cd "${SCRIPT_DIR}"
        ./gradlew app:assembleRelease
    )

    RELEASE_APK_SIGNED="${SCRIPT_DIR}/app/build/outputs/apk/release/app-universal-release.apk"
    RELEASE_APK_UNSIGNED="${SCRIPT_DIR}/app/build/outputs/apk/release/app-universal-release-unsigned.apk"
    RELEASE_APK_SIGNED_LOCAL="${SCRIPT_DIR}/app/build/outputs/apk/release/app-universal-release-signed.apk"

    if [[ -f "${RELEASE_APK_SIGNED}" ]]; then
        cp -f "${RELEASE_APK_SIGNED}" "${APK_DST}"
        echo "  Copied signed release APK -> ${APK_DST}"
    elif [[ -f "${RELEASE_APK_UNSIGNED}" ]]; then
        echo "  release APK is unsigned. Attempting to sign with debug keystore..."
        if command -v jarsigner >/dev/null 2>&1 && [[ -f "${HOME}/.android/debug.keystore" ]]; then
            rm -f "${RELEASE_APK_SIGNED_LOCAL}"
            jarsigner \
                -keystore "${HOME}/.android/debug.keystore" \
                -storepass android \
                -keypass android \
                -signedjar "${RELEASE_APK_SIGNED_LOCAL}" \
                "${RELEASE_APK_UNSIGNED}" \
                androiddebugkey >/dev/null

            if jarsigner -verify -certs "${RELEASE_APK_SIGNED_LOCAL}" >/dev/null 2>&1; then
                cp -f "${RELEASE_APK_SIGNED_LOCAL}" "${APK_DST}"
                echo "  Signed release APK with debug.keystore -> ${APK_DST}"
            else
                echo "  WARNING: release signing verification failed. Falling back to debug APK..."
            fi
        else
            echo "  WARNING: jarsigner/debug.keystore unavailable. Falling back to debug APK..."
        fi

        if [[ ! -f "${APK_DST}" || "$(shasum -a 256 "${APK_DST}" | awk '{print $1}')" == "$(shasum -a 256 "${RELEASE_APK_UNSIGNED}" | awk '{print $1}')" ]]; then
            echo "  Building signed debug APK as installable fallback..."
            (
                cd "${SCRIPT_DIR}"
                ./gradlew app:assembleDebug
            )
            DEBUG_APK="${SCRIPT_DIR}/app/build/outputs/apk/debug/app-universal-debug.apk"
            if [[ -f "${DEBUG_APK}" ]]; then
                cp -f "${DEBUG_APK}" "${APK_DST}"
                echo "  Copied debug APK fallback -> ${APK_DST}"
            else
                echo "ERROR: debug fallback APK not found: ${DEBUG_APK}"
                exit 1
            fi
        fi
    else
        echo "ERROR: release APK not found in expected output paths."
        exit 1
    fi
else
    (
        cd "${SCRIPT_DIR}"
        ./gradlew app:assembleDebug
    )
    DEBUG_APK="${SCRIPT_DIR}/app/build/outputs/apk/debug/app-universal-debug.apk"
    if [[ -f "${DEBUG_APK}" ]]; then
        cp -f "${DEBUG_APK}" "${APK_DST}"
        echo "  Copied debug APK -> ${APK_DST}"
    else
        echo "ERROR: debug APK not found: ${DEBUG_APK}"
        exit 1
    fi
fi

echo "==> Final artifact: ${APK_DST}"
ls -lh "${APK_DST}"
