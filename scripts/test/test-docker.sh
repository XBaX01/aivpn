#!/bin/bash
# AIVPN Docker Test Script

set -e

echo "=== AIVPN Docker Test ==="
echo ""

cd /Users/oleg/Documents/aivpn

# Step 1: Build Docker image
echo "📦 Building Docker image..."
docker build -t aivpn-server:latest .

if [ $? -ne 0 ]; then
    echo "❌ Docker build failed!"
    exit 1
fi

echo "✅ Docker image built successfully"
echo ""

# Step 2: Show image info
echo "📊 Image information:"
docker images aivpn-server:latest
echo ""

# Step 3: Run container (test mode)
echo "🚀 Starting container in test mode..."

# Stop existing container if running
docker stop aivpn-test 2>/dev/null || true
docker rm aivpn-test 2>/dev/null || true

# Run container
docker run -d \
    --name aivpn-test \
    --network host \
    --cap-add NET_ADMIN \
    --cap-add NET_RAW \
    --sysctl net.core.rmem_max=25000000 \
    --sysctl net.core.wmem_max=25000000 \
    --sysctl net.ipv4.ip_forward=1 \
    --tmpfs /run:mode=1777,size=64M \
    --tmpfs /tmp:mode=1777,size=128M \
    --memory 800m \
    --memory-swap 800m \
    -e RUST_LOG=info \
    aivpn-server:latest \
    --listen 0.0.0.0:8443

echo "⏳ Waiting for server to start..."
sleep 3

# Step 4: Check container status
echo ""
echo "📋 Container status:"
docker ps -a --filter "name=aivpn-test" --format "table {{.Names}}\t{{.Status}}\t{{.Ports}}"
echo ""

# Step 5: Check logs
echo "📝 Server logs:"
docker logs aivpn-test --tail 20
echo ""

# Step 6: Health check
echo "🏥 Health check:"
if docker exec aivpn-test pgrep -x aivpn-server > /dev/null 2>&1; then
    echo "✅ Server is running!"
else
    echo "❌ Server is not running!"
    docker logs aivpn-test --tail 50
    exit 1
fi

# Step 7: Check port binding
echo ""
echo "🔌 Port binding check:"
if netstat -tuln 2>/dev/null | grep -q ":8443"; then
    echo "✅ Port 8443 is bound"
    netstat -tuln | grep ":8443"
else
    echo "⚠️  Port 8443 check skipped (netstat not available)"
fi

echo ""
echo "=== Test Complete ==="
echo ""
echo "Useful commands:"
echo "  View logs:     docker logs aivpn-test -f"
echo "  Stop server:   docker stop aivpn-test"
echo "  Remove:        docker rm aivpn-test"
echo "  Exec shell:    docker exec -it aivpn-test bash"
echo ""
