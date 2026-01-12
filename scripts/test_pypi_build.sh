#!/bin/bash
# Script to test PyPI build locally without waiting for GitHub Actions

set -e

echo "🧹 Cleaning previous builds..."
rm -rf dist/ target/wheels/

echo "📦 Building source distribution..."
python3 -m maturin sdist --out dist

echo "🔨 Building wheel..."
python3 -m maturin build --out dist --strip --release

echo ""
echo "📋 Built files:"
ls -lah dist/

echo ""
echo "🔍 Checking wheel metadata..."
python3 -m pip install wheel > /dev/null 2>&1 || true
python3 -c "
import zipfile
import json
import sys

wheel_file = None
for f in __import__('os').listdir('dist'):
    if f.endswith('.whl'):
        wheel_file = f'dist/{f}'
        break

if not wheel_file:
    print('❌ No wheel file found!')
    sys.exit(1)

print(f'📦 Checking: {wheel_file}')
with zipfile.ZipFile(wheel_file, 'r') as z:
    # Find METADATA file
    metadata_files = [f for f in z.namelist() if f.endswith('METADATA')]
    if not metadata_files:
        print('❌ No METADATA file found in wheel!')
        sys.exit(1)
    
    metadata_file = metadata_files[0]
    metadata_content = z.read(metadata_file).decode('utf-8')
    
    print('\n📄 METADATA content:')
    print('=' * 80)
    print(metadata_content)
    print('=' * 80)
    
    # Check for problematic fields
    if 'License-File:' in metadata_content or 'license-file:' in metadata_content.lower():
        print('\n❌ ERROR: Found license-file field in metadata!')
        print('This will cause PyPI upload to fail.')
        sys.exit(1)
    else:
        print('\n✅ No license-file field found in metadata')
"

echo ""
echo "🔍 Checking sdist metadata..."
python3 -c "
import tarfile
import sys

sdist_file = None
for f in __import__('os').listdir('dist'):
    if f.endswith('.tar.gz'):
        sdist_file = f'dist/{f}'
        break

if not sdist_file:
    print('❌ No sdist file found!')
    sys.exit(1)

print(f'📦 Checking: {sdist_file}')
with tarfile.open(sdist_file, 'r:gz') as tar:
    # Find PKG-INFO file
    pkg_info_files = [f for f in tar.getnames() if f.endswith('PKG-INFO')]
    if not pkg_info_files:
        print('❌ No PKG-INFO file found in sdist!')
        sys.exit(1)
    
    pkg_info_file = pkg_info_files[0]
    pkg_info_content = tar.extractfile(pkg_info_file).read().decode('utf-8')
    
    print('\n📄 PKG-INFO content:')
    print('=' * 80)
    print(pkg_info_content)
    print('=' * 80)
    
    # Check for problematic fields
    if 'License-File:' in pkg_info_content or 'license-file:' in pkg_info_content.lower():
        print('\n❌ ERROR: Found license-file field in PKG-INFO!')
        print('This will cause PyPI upload to fail.')
        sys.exit(1)
    else:
        print('\n✅ No license-file field found in PKG-INFO')
"

echo ""
echo "✅ Local build validation complete!"
echo ""
echo "To test upload to TestPyPI (optional):"
echo "  twine upload --repository testpypi dist/*"
echo ""
echo "To validate with twine check:"
echo "  twine check dist/*"
