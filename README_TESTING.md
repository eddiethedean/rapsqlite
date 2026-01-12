# Local Testing Guide

To test PyPI builds locally without waiting for GitHub Actions:

## Quick Check (No Build Required)

If you have artifacts from a failed GitHub Actions run:

```bash
# Download artifacts from a failed run
gh run download <RUN_ID> -D test_artifacts

# Copy to dist/ and check metadata
mkdir -p dist
find test_artifacts -name "*.whl" -exec cp {} dist/ \;
find test_artifacts -name "*.tar.gz" -exec cp {} dist/ \;

# Check metadata for issues
python3 check_metadata.py
```

## Full Local Build (Requires Rust/Cargo)

If you have Rust installed:

```bash
# Install maturin if needed
python3 -m pip install maturin

# Build and check
./test_pypi_build.sh
```

## Validate with Twine

```bash
# Install twine
python3 -m pip install twine

# Check distributions (validates metadata)
twine check dist/*

# Test upload to TestPyPI (optional, requires TestPyPI account)
twine upload --repository testpypi dist/*
```

## Common Issues

### License-File Field

If you see `License-File: LICENSE` in the metadata, maturin is auto-detecting the LICENSE file. The workflow now temporarily renames it during the sdist build to prevent this.

### Missing Rust

If you don't have Rust installed, you can:
1. Install Rust: `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`
2. Or just download artifacts from GitHub Actions and check them with `check_metadata.py`
