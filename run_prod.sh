#!/bin/sh
# Launch the native release binary without Node.js or Python.
set -eu

repo_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
rust_dir="$repo_dir/codex-rs"
binary="$rust_dir/target/release/codex"

rebuild=false
if [ "${1:-}" = "--rebuild" ]; then
    rebuild=true
    shift
fi

if [ "$rebuild" = true ] || [ ! -x "$binary" ]; then
    if ! command -v cargo >/dev/null 2>&1; then
        printf '%s\n' 'Rust/Cargo is required to build Evo Codex. Install Rust with rustup, then try again.' >&2
        exit 1
    fi
    printf '%s\n' 'Building Evo Codex in release mode. This may take several minutes.' >&2
    (
        cd "$rust_dir"
        CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}" cargo build \
            --release --locked -p codex-cli --target-dir "$rust_dir/target"
    )
fi

if [ ! -x "$binary" ]; then
    printf 'Release executable not found: %s\n' "$binary" >&2
    exit 1
fi

# Source builds do not contain the package required by the shared daemon.
# Opt in only when a complete package or a remote server is available.
if [ "${EVO_CODEX_USE_SHARED_SERVER:-0}" != "1" ]; then
    add_no_daemon=true
    for arg do
        case "$arg" in
            --) break ;;
            --no-daemon|--remote|--remote=*) add_no_daemon=false; break ;;
        esac
    done
    if [ "$add_no_daemon" = true ]; then
        set -- --no-daemon "$@"
    fi
fi

# Preserve the caller's working directory and forward every CLI argument.
if [ "${EVO_CODEX_HARDWARE_PANEL:-1}" != "0" ]; then
    set -- -c tui.fullscreen_transcript=true "$@"
fi
exec "$binary" "$@"
