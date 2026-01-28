#!/bin/bash
# Reproduce CI failure in Docker (Ubuntu + Python 3.11)

set -e

echo "=== Reproducing CI failure in Docker (Ubuntu + Python 3.11) ==="
echo ""

docker run --rm \
    -v "$(pwd):/workspace" \
    -w /workspace \
    python:3.11-slim \
    bash -c "
        set -e
        echo 'Installing system dependencies...'
        apt-get update -qq && \
        apt-get install -y -qq build-essential pkg-config libsqlite3-dev curl > /dev/null 2>&1
        
        echo 'Installing Rust...'
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y > /dev/null 2>&1
        export PATH=\"/root/.cargo/bin:\$PATH\"
        
        echo 'Installing maturin...'
        python3.11 -m pip install -q maturin
        
        echo 'Building package...'
        python3.11 -m maturin build --release
        
        echo 'Installing package...'
        python3.11 -m pip install target/wheels/rapsqlite-*.whl --force-reinstall
        
        echo 'Installing test dependencies...'
        python3.11 -m pip install -q pytest pytest-asyncio pytest-cov pytest-xdist hypothesis
        
        echo 'Running tests from /tmp...'
        cd /tmp
        python3.11 -m pytest /workspace/tests/ -v --cov=rapsqlite --cov-report=term-missing --cov-report=xml -n auto
    "
