#!/usr/bin/env bash
# Build and run all tests (Rust unit tests + Python pytest) using a single Python interpreter (Python 3.10+ required).
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
echo "Running Rust unit tests..."
case "$(uname -s)" in
    Darwin*|Linux*)
        if [[ -x "$SCRIPT_DIR/run_rust_tests.sh" ]]; then
            "$SCRIPT_DIR/run_rust_tests.sh" || { echo "Rust tests failed."; exit 1; }
        else
            cargo test --no-default-features --lib || { echo "Rust tests failed."; exit 1; }
        fi
        ;;
    *)
        # Windows and other: run cargo test with env so test binary can find libpython if needed
        cargo test --no-default-features --lib || { echo "Rust tests failed."; exit 1; }
        ;;
esac

echo ""
echo "Running Python tests..."
"$PYTHON" -m pytest tests/ -v "$@"
