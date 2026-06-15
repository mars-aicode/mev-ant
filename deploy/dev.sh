#!/bin/bash
set -euo pipefail

echo "=== Starting mev-ant dev environment ==="

# Start Rust API backend
echo "Starting API server on port 6080..."
RUST_LOG=info cargo run -- serve --config serve.toml &
API_PID=$!
echo "  API PID: $API_PID"

# Wait for API to be ready
sleep 1

# Start dashboard dev server
echo "Starting dashboard dev server..."
cd dashboard
npm run dev &
DASH_PID=$!
echo "  Dashboard PID: $DASH_PID"

echo ""
echo "API:      http://localhost:6080"
echo "Dashboard: http://localhost:8000"
echo ""
echo "Press Ctrl+C to stop both services."

# Cleanup on exit
trap "echo 'Stopping...'; kill $API_PID $DASH_PID 2>/dev/null; exit" INT TERM
wait
