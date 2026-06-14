# syntax=docker/dockerfile:1
# aivpn-mikrotik: runs aivpn-client inside a MikroTik RouterOS container
# Build: docker build --build-arg MUSL_IMAGE_TAG=aarch64-musl --build-arg TARGET_TRIPLE=aarch64-unknown-linux-musl -t aivpn-mikrotik .
# See build-mikrotik.sh for multi-arch manifest push

ARG MUSL_IMAGE_TAG=aarch64-musl
ARG TARGET_TRIPLE=aarch64-unknown-linux-musl

# Stage 1: cross-compile aivpn-client as a static musl binary
FROM --platform=linux/amd64 messense/rust-musl-cross:${MUSL_IMAGE_TAG} AS builder

ARG TARGET_TRIPLE

WORKDIR /app
COPY Cargo.toml ./
COPY aivpn-common aivpn-common/
COPY aivpn-client aivpn-client/
COPY aivpn-server aivpn-server/
COPY aivpn-android-core aivpn-android-core/
COPY aivpn-ios-core aivpn-ios-core/
COPY aivpn-windows aivpn-windows/
COPY aivpn-linux aivpn-linux/
COPY aivpn-common/mask-assets mask-assets/

RUN cargo build --release --target "${TARGET_TRIPLE}" -p aivpn-client --bin aivpn-client \
    && cp "target/${TARGET_TRIPLE}/release/aivpn-client" /aivpn-client \
    && strip /aivpn-client

# Stage 2: minimal runtime image with iproute2 for ip commands
FROM alpine:3.19
RUN apk add --no-cache iproute2 ca-certificates \
    && rm -rf /var/cache/apk/*

COPY --from=builder /aivpn-client /usr/local/bin/aivpn-client
COPY entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh /usr/local/bin/aivpn-client

# Required at runtime: /dev/net/tun must be mounted from RouterOS host
# Required capability: NET_ADMIN (set by RouterOS container runtime)
ENTRYPOINT ["/entrypoint.sh"]
