#!/usr/bin/env bash
# Run the canonical Python formatting, lint, and type checks.
# The same script is used locally and by GitHub Actions.

set -euo pipefail

ruff format --check .
ruff check .
mypy rapsqlite
pyright
pyright --verifytypes rapsqlite --ignoreexternal
