#!/bin/bash
# Script to reproduce CI failure in Docker (Ubuntu + Python 3.11)

set -e

echo "Building Docker image..."
docker build -f Dockerfile.test -t rapsqlite-test:ubuntu-3.11 . || {
    echo "Docker build failed. Trying alternative approach..."
    echo "Creating a simpler test environment..."
    
    # Alternative: Use a Python 3.11 container directly
    docker run --rm -it \
        -v "$(pwd):/workspace" \
        -w /workspace \
        python:3.11-slim \
        bash -c "
            apt-get update && \
            apt-get install -y build-essential pkg-config libsqlite3-dev curl && \
            curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y && \
            export PATH=\"/root/.cargo/bin:\$PATH\" && \
            python3.11 -m pip install maturin && \
            python3.11 -m maturin build --release && \
            python3.11 -m pip install target/wheels/rapsqlite-*.whl --force-reinstall && \
            python3.11 -m pip install pytest pytest-asyncio pytest-cov pytest-xdist hypothesis && \
            cd /tmp && \
            python3.11 -m pytest /workspace/tests/ -v --cov=rapsqlite --cov-report=term-missing --cov-report=xml -n auto
        "
}
