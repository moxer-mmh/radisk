#!/usr/bin/env sh
set -e

REPO="mimobn/radisk"
VERSION="0.1.0"
BINARY_NAME="radisk"

info() { printf "\033[1;34m%s\033[0m\n" "$1"; }
ok()   { printf "\033[1;32m%s\033[0m\n" "$1"; }
err()  { printf "\033[1;31m%s\033[0m\n" "$1" >&2; }

has_command() { command -v "$1" >/dev/null 2>&1; }

install_rust() {
    if has_command cargo; then
        ok "Rust/Cargo already installed ($(cargo --version))"
        return
    fi

    info "Rust/Cargo not found. Installing rustup..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y

    if [ -f "$HOME/.cargo/env" ]; then
        . "$HOME/.cargo/env"
    fi

    if ! has_command cargo; then
        err "Rust installation failed. Please install Rust manually: https://rustup.rs"
        exit 1
    fi

    ok "Rust/Cargo installed ($(cargo --version))"
}

main() {
    info "RaDisk Installer v${VERSION}"
    echo ""

    install_rust

    TMPDIR=$(mktemp -d)
    trap 'rm -rf "$TMPDIR"' EXIT

    info "Downloading RaDisk v${VERSION}..."
    curl -sSfL "https://github.com/${REPO}/archive/refs/tags/v${VERSION}.tar.gz" | tar xz -C "$TMPDIR"

    info "Building and installing..."
    cd "$TMPDIR/${BINARY_NAME}-${VERSION}"
    cargo install --path . --root "$HOME/.radisk-install"

    BIN_DIR="$HOME/.radisk-install/bin"

    if [ -f "$BIN_DIR/$BINARY_NAME" ]; then
        echo ""
        ok "RaDisk v${VERSION} installed successfully!"
        echo ""
        info "Binary location: $BIN_DIR/$BINARY_NAME"
        echo ""
        info "Add to your PATH by adding this to your shell config (~/.bashrc, ~/.zshrc, etc.):"
        echo ""
        echo "    export PATH=\"\$HOME/.radisk-install/bin:\$PATH\""
        echo ""
    else
        err "Installation failed — binary not found at $BIN_DIR/$BINARY_NAME"
        exit 1
    fi
}

main
