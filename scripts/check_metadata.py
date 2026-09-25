#!/usr/bin/env python3
"""
Script to check distribution metadata for PyPI compatibility.
Can be used with pre-built wheels/sdists or will help validate configuration.
"""

import sys
import zipfile
import tarfile
from pathlib import Path


def check_wheel_metadata(wheel_path: Path | str) -> bool:
    """Check wheel metadata for problematic fields."""
    print(f"📦 Checking wheel: {wheel_path}")

    with zipfile.ZipFile(wheel_path, "r") as z:
        metadata_files = [f for f in z.namelist() if f.endswith("METADATA")]
        if not metadata_files:
            print("❌ No METADATA file found in wheel!")
            return False

        metadata_file = metadata_files[0]
        metadata_content = z.read(metadata_file).decode("utf-8")

        print("\n📄 METADATA content:")
        print("=" * 80)
        print(metadata_content)
        print("=" * 80)

        # Check for problematic fields
        issues = []
        # License-File is valid in modern Core Metadata (e.g., Metadata-Version: 2.4).
        if (
            "License-File:" in metadata_content
            or "license-file:" in metadata_content.lower()
        ):
            # Keep as a note, not a failure.
            pass

        if (
            "License-Expression:" in metadata_content
            or "license-expression:" in metadata_content.lower()
        ):
            issues.append("Found 'license-expression' field (PyPI doesn't accept this)")

        if issues:
            print("\n❌ Issues found:")
            for issue in issues:
                print(f"  - {issue}")
            return False
        else:
            print("\n✅ No problematic fields found in wheel metadata")
            return True


def check_sdist_metadata(sdist_path: Path | str) -> bool:
    """Check sdist PKG-INFO for problematic fields."""
    print(f"\n📦 Checking sdist: {sdist_path}")

    with tarfile.open(sdist_path, "r:gz") as tar:
        pkg_info_files = [f for f in tar.getnames() if f.endswith("PKG-INFO")]
        if not pkg_info_files:
            print("❌ No PKG-INFO file found in sdist!")
            return False

        pkg_info_file = pkg_info_files[0]
        pkg_info_handle = tar.extractfile(pkg_info_file)
        if pkg_info_handle is None:
            print(f"❌ Could not read {pkg_info_file} from sdist")
            return False
        pkg_info_content = pkg_info_handle.read().decode("utf-8")

        print("\n📄 PKG-INFO content:")
        print("=" * 80)
        print(pkg_info_content)
        print("=" * 80)

        # Check for problematic fields
        issues = []
        # License-File is valid in modern Core Metadata (e.g., Metadata-Version: 2.4).
        if (
            "License-File:" in pkg_info_content
            or "license-file:" in pkg_info_content.lower()
        ):
            pass

        if (
            "License-Expression:" in pkg_info_content
            or "license-expression:" in pkg_info_content.lower()
        ):
            issues.append("Found 'license-expression' field (PyPI doesn't accept this)")

        if issues:
            print("\n❌ Issues found:")
            for issue in issues:
                print(f"  - {issue}")
            return False
        else:
            print("\n✅ No problematic fields found in PKG-INFO")
            return True


def main():
    dist_dir = Path("dist")

    if not dist_dir.exists():
        print("❌ dist/ directory not found!")
        print("\nTo build distributions:")
        print("  python3 -m maturin sdist --out dist")
        print("  python3 -m maturin build --out dist --strip --release")
        return 1

    wheels = list(dist_dir.glob("*.whl"))
    sdists = list(dist_dir.glob("*.tar.gz"))

    if not wheels and not sdists:
        print("❌ No distribution files found in dist/")
        return 1

    all_good = True

    for wheel in wheels:
        if not check_wheel_metadata(wheel):
            all_good = False

    for sdist in sdists:
        if not check_sdist_metadata(sdist):
            all_good = False

    if all_good:
        print("\n✅ All metadata checks passed!")
        return 0
    else:
        print("\n❌ Metadata validation failed!")
        return 1


if __name__ == "__main__":
    sys.exit(main())
