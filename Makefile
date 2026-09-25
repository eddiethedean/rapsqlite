# rapsqlite Makefile
# Pattern from https://github.com/eddiethedean/robin-sparkless/blob/main/Makefile

.PHONY: build build-release test test-rust test-python check check-full fmt fmt-check clippy audit deny outdated lint-python typecheck clean

# Use stable toolchain when no default is configured (override with RUSTUP_TOOLCHAIN=nightly etc.)
export RUSTUP_TOOLCHAIN ?= stable
# Use real Cargo home to avoid sandbox cache corruption (e.g. Cursor IDE)
export CARGO_HOME := $(HOME)/.cargo

# Build
build:
	cargo build

build-release:
	cargo build --release

# Run Rust tests (uses --no-default-features + lib path so test binary can link libpython)
test-rust:
	./scripts/run_rust_tests.sh

# Run Python tests (creates .venv, installs extension + test deps including alembic/sqlalchemy, runs pytest)
test-python:
	@if [ ! -d .venv ]; then python3 -m venv .venv; fi
	. .venv/bin/activate && pip install -q maturin && pip install -q -r requirements-test.txt && pip install -q alembic sqlalchemy greenlet fastapi httpx aiohttp
	. .venv/bin/activate && maturin develop
	. .venv/bin/activate && pytest tests/ -v

# Run all tests
test: test-rust test-python

# Run Rust checks (format, clippy, audit). Use test-rust separately if you have libpython set up.
check: fmt clippy audit
	@echo "Rust checks passed"

# Full check: Rust checks + Python lint + Python tests
check-full: check lint-python test-python
	@echo "All checks including Python passed"

# Format code
fmt:
	cargo fmt
	@echo "Formatted"

# Check format without modifying
fmt-check:
	cargo fmt --check

# Lint with Clippy
clippy:
	cargo clippy -- -D warnings

# Security: scan for known vulnerabilities
audit:
	cargo audit

# Optional: advisory/bans/sources (requires cargo-deny). Uncomment and add deny.toml to use.
# deny:
# 	cargo deny check advisories bans sources

# List outdated dependencies
outdated:
	cargo outdated

# Python lint and type checks. Uses the same .venv as test-python.
lint-python:
	@if [ ! -d .venv ]; then python3 -m venv .venv; fi
	. .venv/bin/activate && pip install -q -r requirements-ci.txt -r requirements-test.txt && pip install -q alembic aiohttp fastapi greenlet httpx redis sqlalchemy
	. .venv/bin/activate && ./scripts/check_python_types.sh

# Run only the strict Pyright checks in the active environment.
typecheck:
	pyright
	pyright --verifytypes rapsqlite --ignoreexternal

# Clean
clean:
	cargo clean
	rm -rf target/
