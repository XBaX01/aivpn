#!/bin/bash
# Test AIVPN client connection
export RUST_LOG=debug
exec ./target/release/aivpn-client \
  --server YOUR_SERVER_IP:443 \
  --server-key 'YOUR_SERVER_PUBLIC_KEY_BASE64' \
  2>&1 | tee /tmp/aivpn-client.log
