#!/usr/bin/env python3
"""Run aiosqlite test suite against rapsqlite.

This script:
1. Clones/downloads aiosqlite test suite
2. Patches imports to use rapsqlite
3. Runs tests and documents results
"""

import sys
import subprocess
import tempfile
import shutil
import re
from pathlib import Path
from datetime import datetime
from typing import List, Tuple

# Colors for output
GREEN = "\033[32m"
RED = "\033[31m"
YELLOW = "\033[33m"
BLUE = "\033[34m"
RESET = "\033[0m"


def print_status(msg: str, color: str = ""):
    """Print status message with color."""
    if color:
        print(f"{color}{msg}{RESET}")
    else:
        print(msg)


def clone_aiosqlite(temp_dir: Path) -> Path:
    """Clone aiosqlite repository to temp directory."""
    aiosqlite_dir = temp_dir / "aiosqlite"

    if aiosqlite_dir.exists():
        print_status(f"⚠️  Removing existing {aiosqlite_dir}", YELLOW)
        shutil.rmtree(aiosqlite_dir)

    print_status("📥 Cloning aiosqlite repository...", BLUE)
    try:
        subprocess.run(
            [
                "git",
                "clone",
                "--depth",
                "1",
                "https://github.com/omnilib/aiosqlite.git",
                str(aiosqlite_dir),
            ],
            check=True,
            capture_output=True,
        )
        print_status("✅ Cloned successfully", GREEN)
    except subprocess.CalledProcessError as e:
        print_status(f"❌ Failed to clone: {e.stderr.decode()}", RED)
        sys.exit(1)
    except FileNotFoundError:
        print_status("❌ git not found. Please install git.", RED)
        sys.exit(1)

    return aiosqlite_dir


def patch_imports(content: str) -> str:
    """Patch aiosqlite imports to use rapsqlite."""
    # Pattern 1: import aiosqlite (standalone)
    content = re.sub(
        r"^import aiosqlite\s*$",
        "import rapsqlite as aiosqlite",
        content,
        flags=re.MULTILINE,
    )

    # Pattern 2: from aiosqlite import ... (need to handle this carefully)
    # We'll replace with import rapsqlite as aiosqlite, then the import should work
    lines = content.split("\n")
    patched_lines = []
    i = 0
    while i < len(lines):
        line = lines[i]
        # Handle: from aiosqlite import X, Y, Z
        if re.match(r"^from aiosqlite import", line):
            # Replace with import rapsqlite as aiosqlite
            patched_lines.append("import rapsqlite as aiosqlite")
            # Keep the original import line - it should work with the alias
            patched_lines.append(
                line.replace("from aiosqlite import", "from aiosqlite import")
            )
        # Handle: from aiosqlite.something import ...
        elif re.match(r"^from aiosqlite\.", line):
            patched_lines.append(line.replace("from aiosqlite.", "from rapsqlite."))
        else:
            patched_lines.append(line)
        i += 1

    return "\n".join(patched_lines)


def patch_test_files(aiosqlite_dir: Path, patched_dir: Path):
    """Copy and patch test files."""
    test_dir = aiosqlite_dir / "aiosqlite" / "tests"

    if not test_dir.exists():
        print_status(f"❌ Test directory not found: {test_dir}", RED)
        sys.exit(1)

    print_status("🔧 Patching test files...", BLUE)

    # Copy and patch all Python files
    for py_file in test_dir.rglob("*.py"):
        rel_path = py_file.relative_to(test_dir)
        target_file = patched_dir / rel_path
        target_file.parent.mkdir(parents=True, exist_ok=True)

        # Read and patch
        content = py_file.read_text(encoding="utf-8")
        patched_content = patch_imports(content)
        target_file.write_text(patched_content, encoding="utf-8")

        print_status(f"   ✓ Patched: {rel_path}", GREEN)

    # Create __init__.py if needed
    (patched_dir / "__init__.py").touch()


def run_tests(
    patched_dir: Path, project_root: Path
) -> Tuple[List[str], List[str], List[str]]:
    """Run tests and collect results."""
    print_status("\n🧪 Running tests...", BLUE)
    print_status("=" * 60, BLUE)

    # Check if rapsqlite is already installed
    try:
        import rapsqlite

        print_status(
            f"✅ rapsqlite already installed (version: {getattr(rapsqlite, '__version__', 'unknown')})",
            GREEN,
        )
    except ImportError:
        # Install rapsqlite
        print_status("📦 Installing rapsqlite...", BLUE)
        try:
            # Try using maturin develop if available
            if shutil.which("maturin"):
                print_status("   Using maturin develop...", BLUE)
                subprocess.run(
                    ["maturin", "develop"],
                    cwd=project_root,
                    check=True,
                    capture_output=True,
                )
            else:
                # Fall back to pip install
                subprocess.run(
                    [sys.executable, "-m", "pip", "install", "-e", str(project_root)],
                    check=True,
                    capture_output=True,
                )
            print_status("✅ rapsqlite installed", GREEN)
        except (subprocess.CalledProcessError, FileNotFoundError):
            # Try without -e
            try:
                subprocess.run(
                    [sys.executable, "-m", "pip", "install", str(project_root)],
                    check=True,
                    capture_output=True,
                )
                print_status("✅ rapsqlite installed", GREEN)
            except subprocess.CalledProcessError:
                print_status("⚠️  Could not install rapsqlite", YELLOW)
                print_status("   Make sure rapsqlite is built: maturin develop", YELLOW)
                print_status("   Continuing anyway...", YELLOW)

    # Find test files
    test_files = list(patched_dir.rglob("test_*.py"))
    if not test_files:
        test_files = list(patched_dir.rglob("*.py"))
        test_files = [f for f in test_files if f.name != "__init__.py"]

    passed = []
    failed = []
    skipped = []

    # Run each test file
    for test_file in test_files:
        rel_path = test_file.relative_to(patched_dir)
        print_status(f"\n📝 Running: {rel_path}", BLUE)
        print_status("-" * 60, BLUE)

        try:
            result = subprocess.run(
                [sys.executable, "-m", "pytest", str(test_file), "-v", "--tb=short"],
                cwd=patched_dir,
                capture_output=True,
                text=True,
            )

            if result.returncode == 0:
                passed.append(str(rel_path))
                print_status(f"✅ PASSED: {rel_path}", GREEN)
            elif result.returncode == 5:  # No tests collected
                skipped.append(str(rel_path))
                print_status(f"⏭️  SKIPPED: {rel_path} (no tests)", YELLOW)
            else:
                failed.append(str(rel_path))
                print_status(f"❌ FAILED: {rel_path}", RED)
                # Print key error information
                output = result.stdout + result.stderr
                # Look for AttributeError, TypeError, etc.
                error_lines = [
                    line
                    for line in output.split("\n")
                    if any(
                        keyword in line
                        for keyword in [
                            "AttributeError",
                            "TypeError",
                            "NotImplementedError",
                            "FAILED",
                            "Error:",
                            "assert",
                        ]
                    )
                ][:15]
                for line in error_lines:
                    if line.strip() and not line.strip().startswith("="):
                        print(f"   {line[:120]}")
        except Exception as e:
            failed.append(str(rel_path))
            print_status(f"❌ ERROR running {rel_path}: {e}", RED)

    return passed, failed, skipped


def generate_report(
    passed: List[str],
    failed: List[str],
    skipped: List[str],
    project_root: Path,
    rapsqlite_version: str,
):
    """Generate test results report."""
    report_file = project_root / "docs" / "AIOSQLITE_TEST_RESULTS.md"
    report_file.parent.mkdir(exist_ok=True)

    total = len(passed) + len(failed) + len(skipped)

    content = f"""# aiosqlite Test Suite Results

This document contains the results of running the aiosqlite test suite against rapsqlite.

**Date**: {datetime.now().strftime("%Y-%m-%d %H:%M:%S")}
**rapsqlite Version**: {rapsqlite_version}
**Python Version**: {sys.version.split()[0]}

## Summary

- **Total Test Files**: {total}
- **✅ Passed**: {len(passed)}
- **❌ Failed**: {len(failed)}
- **⏭️  Skipped**: {len(skipped)}

## Passed Tests

"""

    if passed:
        for test in sorted(passed):
            content += f"- `{test}`\n"
    else:
        content += "*No tests passed*\n"

    if failed:
        content += """
## Failed Tests

"""
        for test in sorted(failed):
            content += f"- `{test}`\n"

        content += """
### Failure Analysis

These tests failed due to compatibility differences between aiosqlite and rapsqlite.
See [MIGRATION.md](MIGRATION.md) for details on known differences.

**Common failure reasons:**
- API differences (intentional or unintentional)
- Different error message formats
- Behavioral differences in edge cases
- Missing features in rapsqlite

**Next steps:**
1. Review failed tests to identify compatibility gaps
2. Fix compatibility issues where possible
3. Document intentional differences in MIGRATION.md
"""

    if skipped:
        content += """
## Skipped Tests

"""
        for test in sorted(skipped):
            content += f"- `{test}`\n"

    content += """
## Notes

- Tests were run by patching aiosqlite imports to use rapsqlite
- Some failures may be due to intentional differences (see [MIGRATION.md](MIGRATION.md))
- Some failures may indicate areas for improvement in rapsqlite compatibility
- This is a compatibility validation exercise, not a requirement for 100% pass rate
"""

    report_file.write_text(content, encoding="utf-8")
    print_status(f"\n📝 Report saved to {report_file}", GREEN)

    return report_file


def main():
    """Main entry point."""
    project_root = Path(__file__).parent.parent.resolve()

    # Get rapsqlite version
    try:
        import rapsqlite

        rapsqlite_version = rapsqlite.__version__
    except (ImportError, AttributeError):
        rapsqlite_version = "unknown"

    print_status("🔍 aiosqlite Test Suite Adapter for rapsqlite", BLUE)
    print_status("=" * 60, BLUE)

    # Create temp directory
    with tempfile.TemporaryDirectory(prefix="rapsqlite_aiosqlite_") as temp_dir:
        temp_path = Path(temp_dir)
        patched_dir = temp_path / "patched_tests"
        patched_dir.mkdir()

        # Clone aiosqlite
        aiosqlite_dir = clone_aiosqlite(temp_path)

        # Patch test files
        patch_test_files(aiosqlite_dir, patched_dir)

        # Run tests
        passed, failed, skipped = run_tests(patched_dir, project_root)

        # Generate report
        report_file = generate_report(
            passed, failed, skipped, project_root, rapsqlite_version
        )

        # Print summary
        print_status("\n" + "=" * 60, BLUE)
        print_status("📊 Test Results Summary", BLUE)
        print_status("=" * 60, BLUE)
        print_status(f"✅ Passed: {len(passed)}", GREEN)
        print_status(f"❌ Failed: {len(failed)}", RED if failed else GREEN)
        print_status(f"⏭️  Skipped: {len(skipped)}", YELLOW)
        print_status(f"\n📝 Detailed report: {report_file}", BLUE)

        if failed:
            print_status("\n⚠️  Some tests failed. See report for details.", YELLOW)
            return 1
        else:
            print_status("\n✅ All tests passed!", GREEN)
            return 0


if __name__ == "__main__":
    sys.exit(main())
