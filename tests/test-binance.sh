#!/bin/bash

echo "Testing Binance WebSocket connection..."
echo "This will connect to the orderbook stream for 5 seconds"
echo "----------------------------------------"

# Test WebSocket connection with wscat (if installed) or curl
# Using Binance US endpoint due to geo-restrictions
if command -v wscat &> /dev/null; then
    timeout 5 wscat -c "wss://stream.binance.us:9443/ws/btcusd@depth20@100ms"
elif command -v websocat &> /dev/null; then
    timeout 5 websocat "wss://stream.binance.us:9443/ws/btcusd@depth20@100ms"
else
    echo "Testing with curl (will show connection headers only):"
    curl -i -N \
        -H "Connection: Upgrade" \
        -H "Upgrade: websocket" \
        -H "Sec-WebSocket-Version: 13" \
        -H "Sec-WebSocket-Key: $(openssl rand -base64 16)" \
        "https://stream.binance.us:9443/ws/btcusd@depth20@100ms"
fi

echo ""
echo "----------------------------------------"
echo "To install a WebSocket client for better testing:"
echo "  npm install -g wscat"
echo "  # or"
echo "  cargo install websocat"