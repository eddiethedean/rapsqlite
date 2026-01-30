#!/usr/bin/env bash
# Build and run tests using a single Python interpreter (Python 3.10+ required).
# Use this to avoid version mismatches between maturin build and pytest run.
#
# Usage: ./scripts/dev_test.sh [pytest options...]
#   PYTHON=python3.12 ./scripts/dev_test.sh -n 4
#   ./scripts/dev_test.sh -m "not slow"
#
# Use a venv or ensure PYTHON's environment is writable so `maturin develop`
# can install the wheel.

set -e

PYTHON="${PYTHON:-python3}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
export PYO3_PYTHON="$("$PYTHON" -c 'import sys; print(sys.executable)')"

echo "Using Python: $PYTHON ($PYO3_PYTHON)"
"$PYTHON" --version
echo ""

cd "$PROJECT_ROOT"

echo "Building (maturin develop)..."
# PYO3_PYTHON forces the build target; maturin installs into that interpreter's env.
"$PYTHON" -m maturin develop

echo ""
echo "Running tests..."
"$PYTHON" -m pytest tests/ -v "$@"
