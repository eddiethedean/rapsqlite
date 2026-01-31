#!/usr/bin/env bash
# Run Rust unit tests (cargo test). The rapsqlite crate is a PyO3 extension that
# normally builds without linking libpython. To run tests we build without the
# extension-module feature so the test binary links libpython; we then set
# the library path so the binary can find libpython at runtime.

set -e

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

# Build the test binary so we can detect which libpython it needs
cargo build --tests --no-default-features --quiet 2>/dev/null || true

LIBDIR=""
BIN=""
for f in target/debug/deps/rapsqlite-*; do
    if [[ -f "$f" && -x "$f" && "$f" != *.* ]]; then
        BIN="$f"
        break
    fi
done

if [[ -n "$BIN" ]]; then
    case "$(uname -s)" in
        Darwin*)
            # Detect libpython required by the test binary (e.g. libpython3.12.dylib)
            LIBNAME=$(otool -L "$BIN" 2>/dev/null | awk '/libpython[0-9]+\.[0-9]+\.(dylib|so)/ { gsub(/.*\//, ""); print $1; exit }')
            if [[ -n "$LIBNAME" ]]; then
                for base in "$HOME/.pyenv/versions"/*/lib /opt/homebrew/opt/python*/lib /usr/local/lib "$HOME/anaconda3/lib" "$HOME/miniconda3/lib" /opt/anaconda3/lib; do
                    if [[ -f "${base}/${LIBNAME}" ]]; then
                        LIBDIR="$base"
                        break
                    fi
                done
            fi
            ;;
        Linux*)
            # ldd output: libname => /path/to/lib (0x...); path is $3
            LIBPATH=$(ldd "$BIN" 2>/dev/null | awk '/libpython[0-9]+\.[0-9]+\.so/ { print $3; exit }')
            if [[ -n "$LIBPATH" && -f "$LIBPATH" ]]; then
                LIBDIR=$(dirname "$LIBPATH")
            fi
            ;;
    esac
fi

# Fallback: use PYO3_PYTHON or python3 to get lib dir (must match build Python)
if [[ -z "$LIBDIR" || ! -d "$LIBDIR" ]]; then
    PYTHON="${PYO3_PYTHON:-python3}"
    if command -v "$PYTHON" &>/dev/null; then
        LIBDIR="$("$PYTHON" -c "
import sysconfig
d = sysconfig.get_config_var('LIBDIR')
if d:
    print(d)
else:
    print((sysconfig.get_config_var('prefix') or '') + '/lib')
" 2>/dev/null)" || true
    fi
fi

if [[ -z "$LIBDIR" || ! -d "$LIBDIR" ]]; then
    echo "Error: Could not find libpython for the test binary. Set PYO3_PYTHON to the Python used for building, or install that Python (e.g. pyenv) so libpython can be found." >&2
    exit 1
fi

# macOS: dyld looks in DYLD_LIBRARY_PATH and @rpath
# Linux: ld looks in LD_LIBRARY_PATH
case "$(uname -s)" in
    Darwin*)
        export DYLD_LIBRARY_PATH="${LIBDIR}${DYLD_LIBRARY_PATH:+:$DYLD_LIBRARY_PATH}"
        ;;
    Linux*)
        export LD_LIBRARY_PATH="${LIBDIR}${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
        ;;
esac

# Run only lib unit tests; doc tests in this crate contain Python examples, not Rust.
echo "Running: cargo test --no-default-features --lib (Python lib: $LIBDIR)"
exec cargo test --no-default-features --lib "$@"
