#!/bin/bash
# WSL backend used by package.ps1. Signing always happens on Windows.
set -euo pipefail
export PATH="$HOME/.cargo/bin:/usr/bin:/bin:$PATH"
cd "$(dirname "$0")/.."
export CARGO_TARGET_DIR="${BT_BUILD_TARGET_DIR:-/tmp/bloqueio-taskbar-target}"
export ZIG="$PWD/scripts/llvm-resource-compiler.sh"
chmod +x "$ZIG"
target="${2:-x86_64-pc-windows-msvc}"
case "$target" in x86_64-pc-windows-msvc|i686-pc-windows-msvc) ;; *) exit 2;; esac
if [ "$target" = i686-pc-windows-msvc ]; then
    export XWIN_ARCH=x86
    export XWIN_CACHE_DIR="${BT_X86_SDK_CACHE:-$HOME/.cache/bloqueio-xwin-x86}"
fi
case "${1:-}" in
    app)
        cargo xwin build --locked --release --target "$target" --bin BloqueioTransparente
        cp "$CARGO_TARGET_DIR/$target/release/BloqueioTransparente.exe" "$3"
        ;;
    installer)
        export BT_APP_PAYLOAD="$3"
        cargo xwin build --locked --release --target "$target" -p bloqueio-transparente-installer
        cp "$CARGO_TARGET_DIR/$target/release/BloqueioTransparente-Setup.exe" "$4"
        ;;
    check)
        cargo xwin clippy --locked --release --target "$target" --all-targets -- -D warnings
        ;;
    check-installer)
        export BT_APP_PAYLOAD="$3"
        cargo xwin clippy --locked --release --target "$target" -p bloqueio-transparente-installer --all-targets -- -D warnings
        ;;
    test)
        cargo test --locked --lib --tests -- --skip installation_exposes_a_start_menu_entry --skip setup_can_create_start_menu_and_desktop_shortcuts_independently
        ;;
    *) echo "Use app, installer, check, check-installer ou test" >&2; exit 2;;
esac
