#!/usr/bin/env bash
# Build multi-arch aivpn-mikrotik images and push a Docker Hub manifest.
# Usage: ./aivpn-mikrotik/build-mikrotik.sh [registry/image:tag]
#   Default image: infosave2007/aivpn-mikrotik:latest

set -euo pipefail

IMAGE="${1:-infosave2007/aivpn-mikrotik:latest}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR/.."

require_command() { command -v "$1" >/dev/null 2>&1 || { echo "Error: '$1' not installed" >&2; exit 1; }; }
require_command docker

echo "=== Building aivpn-mikrotik images ==="
echo "Target: ${IMAGE}"
echo ""

TAGS_FILE="$(mktemp)"

build_arch() {
    local ARCH="$1" MUSL_TAG="$2" TARGET="$3"
    local TAG="${IMAGE%:*}:${IMAGE##*:}-${ARCH}"
    echo "--- Building ${ARCH} (${TARGET}) ---"
    docker build \
        --platform linux/amd64 \
        --build-arg MUSL_IMAGE_TAG="${MUSL_TAG}" \
        --build-arg TARGET_TRIPLE="${TARGET}" \
        -t "${TAG}" \
        -f aivpn-mikrotik/Dockerfile \
        .
    docker push "${TAG}"
    echo "${TAG}" >> "$TAGS_FILE"
}

build_arch arm64  aarch64-musl          aarch64-unknown-linux-musl
build_arch armv7  armv7-musleabihf      armv7-unknown-linux-musleabihf
build_arch amd64  x86_64-musl           x86_64-unknown-linux-musl

echo ""
echo "--- Creating multi-arch manifest: ${IMAGE} ---"
docker manifest rm "${IMAGE}" 2>/dev/null || true

mapfile -t ALL_TAGS < "$TAGS_FILE"
rm -f "$TAGS_FILE"

docker manifest create "${IMAGE}" "${ALL_TAGS[@]}"
docker manifest annotate "${IMAGE}" "${ALL_TAGS[0]}" --os linux --arch arm64 --variant v8
docker manifest annotate "${IMAGE}" "${ALL_TAGS[1]}" --os linux --arch arm --variant v7
docker manifest annotate "${IMAGE}" "${ALL_TAGS[2]}" --os linux --arch amd64

docker manifest push "${IMAGE}"

echo ""
echo "=== Done: ${IMAGE} published ==="
