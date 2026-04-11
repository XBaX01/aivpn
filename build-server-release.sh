#!/bin/bash

set -euo pipefail

IMAGE_TAG="aivpn-server-builder:release"
CONTAINER_NAME="aivpn-server-release-$RANDOM-$RANDOM"
ARTIFACT_PATH="releases/aivpn-server-linux-x86_64"

cleanup() {
    docker rm -f "$CONTAINER_NAME" >/dev/null 2>&1 || true
}

trap cleanup EXIT

echo "=== Building Linux server release artifact ==="
echo ""
echo "Building Docker builder image..."
docker build --target builder -t "$IMAGE_TAG" -f Dockerfile .

echo ""
echo "Extracting aivpn-server binary..."
mkdir -p releases
docker create --name "$CONTAINER_NAME" "$IMAGE_TAG" >/dev/null
docker cp "$CONTAINER_NAME:/app/target/release/aivpn-server" "$ARTIFACT_PATH"
chmod +x "$ARTIFACT_PATH"

echo ""
echo "=== Artifact Ready ==="
ls -lh "$ARTIFACT_PATH"
file "$ARTIFACT_PATH" || true
echo ""
echo "Fast Docker install command:"
echo "  AIVPN_SERVER_DOCKERFILE=Dockerfile.prebuilt docker compose up -d aivpn-server"