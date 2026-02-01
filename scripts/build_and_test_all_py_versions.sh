#!/usr/bin/env bash
# Create venvs for Python 3.10–3.14, build rapsqlite for each, and run tests.
# Uses pyenv-managed Pythons when available; skips versions that are not installed.

set -e
set -o pipefail
# Use user's Cargo home so builds work outside sandbox (avoid cursor-sandbox-cache)
export CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
VENVS_DIR="${PROJECT_ROOT}/.venvs"

# Python versions to try (in order)
VERSIONS=(3.10 3.11 3.12 3.13 3.14)

# Resolve python executable for a version (e.g. 3.10 -> python3.10 or pyenv 3.10.18 path)
get_python() {
  local v="$1"
  local p
  # Prefer pyenv versioned path so we get exact interpreter (3.10 -> 3.10.18)
  local pyenv_root="${PYENV_ROOT:-$HOME/.pyenv}"
  if [ -d "${pyenv_root}/versions" ]; then
    for p in "${pyenv_root}"/versions/"${v}"*/bin/python; do
      [ -x "$p" ] && echo "$p" && return
    done
  fi
  if command -v "python${v}" &>/dev/null; then
    command -v "python${v}"
  else
    return 1
  fi
}

run_for_version() {
  local ver="$1"
  local venv_dir="${VENVS_DIR}/py${ver}"
  local python_exe
  python_exe="$(get_python "$ver" 2>/dev/null)" || {
    echo "[Python ${ver}] not installed, skipping"
    return 0
  }

  echo "=============================================="
  echo "Python ${ver} -> ${python_exe}"
  echo "=============================================="

  # Create venv
  rm -rf "${venv_dir}"
  "${python_exe}" -m venv "${venv_dir}"
  local pip="${venv_dir}/bin/pip"
  local py="${venv_dir}/bin/python"

  # Upgrade pip and install maturin + test deps (no extra quotes around sqlalchemy spec)
  "${pip}" install -q --upgrade pip
  "${pip}" install -q maturin
  "${pip}" install -q pytest pytest-asyncio pytest-xdist pytest-timeout hypothesis fastapi "sqlalchemy>=2.0" greenlet httpx aiohttp

  # Build rapsqlite into this venv: activate so maturin finds this interpreter.
  (cd "${PROJECT_ROOT}" && . "${venv_dir}/bin/activate" && python -m maturin develop --release)

  # Run tests (pipefail so pipeline exit code is pytest's)
  (cd "${PROJECT_ROOT}" && "${py}" -m pytest tests/ -v --tb=short -q 2>&1) | tail -40
  local status=$?
  if [ $status -eq 0 ]; then
    echo "[Python ${ver}] PASSED"
  else
    echo "[Python ${ver}] FAILED (exit $status)"
  fi
  return $status
}

mkdir -p "${VENVS_DIR}"
cd "${PROJECT_ROOT}"

FAILED=()
for ver in "${VERSIONS[@]}"; do
  if run_for_version "$ver"; then
    :
  else
    FAILED+=("$ver")
  fi
  echo ""
done

if [ ${#FAILED[@]} -eq 0 ]; then
  echo "All versions passed."
  exit 0
else
  echo "Failed versions: ${FAILED[*]}"
  exit 1
fi
