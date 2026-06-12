#!/bin/bash
# sg32 — Oracle Cloud free tier setup script
# Run on a fresh Ubuntu 22.04/24.04 ARM instance

set -e

echo "=== sg32 Oracle setup ==="

# Dependencies
sudo apt-get update -qq
sudo apt-get install -y -qq curl build-essential pkg-config libssl-dev git

# Rust
if ! command -v cargo &>/dev/null; then
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    . "$HOME/.cargo/env"
fi

# Clone and build
if [ ! -d sg32 ]; then
    git clone https://github.com/victorchukwuemeka/sg32.git
fi
cd sg32
cargo build --release

# Open firewall ports
sudo ufw allow 8899/tcp 2>/dev/null || true
sudo ufw allow 8001/udp 2>/dev/null || true
sudo ufw allow 8003/udp 2>/dev/null || true

echo ""
echo "=== Done ==="
echo "Run: cd sg32 && ./target/release/dc-tvu"
echo "Then open http://$(curl -s ifconfig.me):8899"
