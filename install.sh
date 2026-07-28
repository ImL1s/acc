#!/bin/sh
set -eu
set -o pipefail 2>/dev/null || true

# Pre-flight check: tar requirement
if ! command -v tar >/dev/null 2>&1; then
  echo "Error: 'tar' utility is required to extract release archives." >&2
  exit 1
fi

# Detect Operating System
OS="$(uname -s)"
case "$OS" in
  Linux)
    TARGET_OS="linux"
    ;;
  Darwin)
    TARGET_OS="macos"
    ;;
  *)
    echo "Error: Unsupported operating system '$OS'." >&2
    echo "Currently supported OS: Linux, macOS (Darwin)." >&2
    exit 1
    ;;
esac

# Detect CPU Architecture
ARCH="$(uname -m)"
case "$ARCH" in
  x86_64|amd64)
    TARGET_ARCH="x86_64"
    ;;
  aarch64|arm64|armv8*)
    TARGET_ARCH="aarch64"
    ;;
  *)
    echo "Error: Unsupported CPU architecture '$ARCH'." >&2
    echo "Currently supported architectures: x86_64, aarch64 (arm64)." >&2
    exit 1
    ;;
esac

# Determine Version & Release Asset URL
VERSION="${ACC_VERSION:-${VERSION:-latest}}"
ARTIFACT_NAME="acc-${TARGET_ARCH}-${TARGET_OS}.tar.gz"

if [ "$VERSION" = "latest" ]; then
  DOWNLOAD_URL="https://github.com/ImL1s/acc/releases/latest/download/${ARTIFACT_NAME}"
else
  DOWNLOAD_URL="https://github.com/ImL1s/acc/releases/download/${VERSION}/${ARTIFACT_NAME}"
fi

INSTALL_DIR="${ACC_INSTALL_DIR:-${INSTALL_DIR:-$HOME/.local/bin}}"

# Create temporary directory and set exit/signal trap
TMP_DIR="$(mktemp -d 2>/dev/null || mktemp -d -t 'acc-install')"
cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT HUP INT TERM

echo "Downloading acc release asset (${TARGET_ARCH}-${TARGET_OS})..."
echo "URL: $DOWNLOAD_URL"

if command -v curl >/dev/null 2>&1; then
  curl -fsSL "$DOWNLOAD_URL" -o "$TMP_DIR/$ARTIFACT_NAME"
elif command -v wget >/dev/null 2>&1; then
  wget -q "$DOWNLOAD_URL" -O "$TMP_DIR/$ARTIFACT_NAME"
else
  echo "Error: Neither 'curl' nor 'wget' was found on your system. Please install one of them." >&2
  exit 1
fi

echo "Installing acc to $INSTALL_DIR..."
mkdir -p "$INSTALL_DIR"
tar -xzf "$TMP_DIR/$ARTIFACT_NAME" -C "$TMP_DIR"

if [ ! -f "$TMP_DIR/acc" ]; then
  echo "Error: Extraction failed; 'acc' binary not found in release archive." >&2
  exit 1
fi

chmod 755 "$TMP_DIR/acc"
mv "$TMP_DIR/acc" "$INSTALL_DIR/acc"

echo "Successfully installed acc to $INSTALL_DIR/acc"

# Check if INSTALL_DIR is in PATH
case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *)
    echo ""
    echo "Notice: $INSTALL_DIR is not currently in your PATH."
    echo "To use 'acc' from any terminal, add the following line to your shell profile:"
    echo "  export PATH=\"$INSTALL_DIR:\$PATH\""
    ;;
esac
