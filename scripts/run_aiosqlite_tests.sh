#!/bin/bash
# Script to run aiosqlite test suite with rapsqlite
# Downloads aiosqlite, patches imports, runs tests

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
TEMP_DIR="${TEMP_DIR:-/tmp/rapsqlite_aiosqlite_tests}"
AIOSQLITE_REPO="https://github.com/omnilib/aiosqlite.git"
AIOSQLITE_DIR="$TEMP_DIR/aiosqlite"

echo "🔍 Setting up aiosqlite test suite for rapsqlite compatibility testing..."

# Clean up previous run
if [ -d "$TEMP_DIR" ]; then
    echo "🧹 Cleaning up previous test run..."
    rm -rf "$TEMP_DIR"
fi

mkdir -p "$TEMP_DIR"
cd "$TEMP_DIR"

# Clone aiosqlite repository
echo "📥 Cloning aiosqlite repository..."
if ! git clone --depth 1 "$AIOSQLITE_REPO" "$AIOSQLITE_DIR" 2>/dev/null; then
    echo "❌ Failed to clone aiosqlite repository"
    echo "   Make sure you have git installed and internet access"
    exit 1
fi

cd "$AIOSQLITE_DIR"

# Find test files
TEST_FILES=$(find . -name "test_*.py" -type f | grep -E "(tests?|test)" | head -20)
if [ -z "$TEST_FILES" ]; then
    # Try alternative locations
    TEST_FILES=$(find . -path "*/test*.py" -type f | head -20)
fi

if [ -z "$TEST_FILES" ]; then
    echo "⚠️  No test files found in aiosqlite repository"
    echo "   Repository structure may have changed"
    echo "   Listing repository contents:"
    find . -type f -name "*.py" | head -20
    exit 1
fi

echo "📋 Found test files:"
echo "$TEST_FILES" | head -10
echo ""

# Create patched test directory
PATCHED_DIR="$TEMP_DIR/patched_tests"
mkdir -p "$PATCHED_DIR"

# Copy and patch test files
echo "🔧 Patching test files to use rapsqlite..."
for test_file in $TEST_FILES; do
    if [ -f "$test_file" ]; then
        rel_path="${test_file#./}"
        target_dir="$PATCHED_DIR/$(dirname "$rel_path")"
        mkdir -p "$target_dir"
        
        # Patch imports: replace aiosqlite with rapsqlite
        sed 's/^import aiosqlite$/import rapsqlite as aiosqlite/g' "$test_file" | \
        sed 's/^from aiosqlite/from rapsqlite as aiosqlite/g' | \
        sed 's/from aiosqlite import/import rapsqlite as aiosqlite; from aiosqlite import/g' > \
        "$PATCHED_DIR/$rel_path"
        
        echo "   ✓ Patched: $rel_path"
    fi
done

# Create __init__.py if needed
touch "$PATCHED_DIR/__init__.py"

# Install rapsqlite in test environment
echo ""
echo "📦 Installing rapsqlite..."
cd "$PROJECT_ROOT"

# Build rapsqlite if needed
if [ ! -f "target/wheels/rapsqlite-"*.whl ] && [ ! -d "rapsqlite/_rapsqlite.abi3.so" ] && [ ! -d "rapsqlite/_rapsqlite.so" ]; then
    echo "🔨 Building rapsqlite..."
    if command -v maturin &> /dev/null; then
        maturin develop
    else
        echo "⚠️  maturin not found, trying pip install -e ."
        pip install -e . > /dev/null 2>&1 || {
            echo "❌ Failed to install rapsqlite"
            exit 1
        }
    fi
else
    pip install -e . > /dev/null 2>&1 || {
        echo "⚠️  Could not install rapsqlite with -e, trying direct install"
        pip install . > /dev/null 2>&1 || {
            echo "❌ Failed to install rapsqlite"
            exit 1
        }
    }
fi

# Run tests
echo ""
echo "🧪 Running aiosqlite tests with rapsqlite..."
echo "=========================================="
cd "$PATCHED_DIR"

# Find and run test files
FAILED_TESTS=()
PASSED_TESTS=()
SKIPPED_TESTS=()

for test_file in $(find . -name "test_*.py" -type f); do
    echo ""
    echo "Running: $test_file"
    echo "----------------------------------------"
    
    if python -m pytest "$test_file" -v --tb=short 2>&1 | tee /tmp/test_output.log; then
        PASSED_TESTS+=("$test_file")
        echo "✅ PASSED: $test_file"
    else
        EXIT_CODE=${PIPESTATUS[0]}
        if [ "$EXIT_CODE" -eq 5 ]; then
            SKIPPED_TESTS+=("$test_file")
            echo "⏭️  SKIPPED: $test_file"
        else
            FAILED_TESTS+=("$test_file")
            echo "❌ FAILED: $test_file"
        fi
    fi
done

# Generate report
echo ""
echo "=========================================="
echo "📊 Test Results Summary"
echo "=========================================="
echo "✅ Passed: ${#PASSED_TESTS[@]}"
echo "❌ Failed: ${#FAILED_TESTS[@]}"
echo "⏭️  Skipped: ${#SKIPPED_TESTS[@]}"
echo ""

if [ ${#FAILED_TESTS[@]} -gt 0 ]; then
    echo "❌ Failed Tests:"
    for test in "${FAILED_TESTS[@]}"; do
        echo "   - $test"
    done
    echo ""
fi

# Save detailed report
REPORT_FILE="$PROJECT_ROOT/docs/AIOSQLITE_TEST_RESULTS.md"
echo "📝 Saving detailed report to $REPORT_FILE..."

cat > "$REPORT_FILE" << EOF
# aiosqlite Test Suite Results

This document contains the results of running the aiosqlite test suite against rapsqlite.

**Date**: $(date)
**rapsqlite Version**: $(python -c "import rapsqlite; print(rapsqlite.__version__)" 2>/dev/null || echo "unknown")
**Python Version**: $(python --version)

## Summary

- **Total Tests**: $((${#PASSED_TESTS[@]} + ${#FAILED_TESTS[@]} + ${#SKIPPED_TESTS[@]}))
- **Passed**: ${#PASSED_TESTS[@]}
- **Failed**: ${#FAILED_TESTS[@]}
- **Skipped**: ${#SKIPPED_TESTS[@]}

## Passed Tests

EOF

for test in "${PASSED_TESTS[@]}"; do
    echo "- \`$test\`" >> "$REPORT_FILE"
done

if [ ${#FAILED_TESTS[@]} -gt 0 ]; then
    cat >> "$REPORT_FILE" << EOF

## Failed Tests

EOF
    for test in "${FAILED_TESTS[@]}"; do
        echo "- \`$test\`" >> "$REPORT_FILE"
    done
    
    cat >> "$REPORT_FILE" << EOF

### Failure Analysis

These tests failed due to compatibility differences between aiosqlite and rapsqlite.
See [MIGRATION.md](MIGRATION.md) for details on known differences.

EOF
fi

if [ ${#SKIPPED_TESTS[@]} -gt 0 ]; then
    cat >> "$REPORT_FILE" << EOF

## Skipped Tests

EOF
    for test in "${SKIPPED_TESTS[@]}"; do
        echo "- \`$test\`" >> "$REPORT_FILE"
    done
fi

cat >> "$REPORT_FILE" << EOF

## Notes

- Tests were run by patching aiosqlite imports to use rapsqlite
- Some failures may be due to intentional differences (see [MIGRATION.md](MIGRATION.md))
- Some failures may indicate areas for improvement in rapsqlite compatibility

EOF

echo "✅ Report saved to $REPORT_FILE"

# Cleanup
if [ "${KEEP_TEMP:-0}" != "1" ]; then
    echo ""
    echo "🧹 Cleaning up temporary files..."
    rm -rf "$TEMP_DIR"
fi

# Exit with appropriate code
if [ ${#FAILED_TESTS[@]} -gt 0 ]; then
    echo ""
    echo "⚠️  Some tests failed. See $REPORT_FILE for details."
    exit 1
else
    echo ""
    echo "✅ All tests passed!"
    exit 0
fi
