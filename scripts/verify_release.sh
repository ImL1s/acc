#!/usr/bin/env bash
# Release Verification Script for acc
# Verifies GitHub release status, installer syntax, artifact download/extraction, and binary functionality.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TAG="${1:-v0.1.0}"
REPO="ImL1s/acc"

echo "=========================================="
echo " Starting acc Release Verification: $TAG"
echo "=========================================="

# Prerequisites check
for cmd in gh bash tar curl; do
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "ERROR: Required command '$cmd' is not installed or not in PATH." >&2
    exit 1
  fi
done

# Step 1: Verify GitHub Release exists
echo "[1/4] Checking GitHub Release '$TAG'..."
if gh release view "$TAG" --repo "$REPO" >/dev/null 2>&1; then
  echo "  ✓ GitHub Release '$TAG' exists on $REPO."
else
  echo "  ✗ ERROR: GitHub Release '$TAG' not found on $REPO!" >&2
  exit 1
fi

# Step 2: Validate install.sh syntax
echo "[2/4] Validating install.sh syntax..."
INSTALL_SH="$ROOT/install.sh"
if [[ ! -f "$INSTALL_SH" ]]; then
  echo "  ✗ ERROR: $INSTALL_SH does not exist!" >&2
  exit 1
fi

if bash -n "$INSTALL_SH"; then
  echo "  ✓ install.sh syntax check passed (bash -n)."
else
  echo "  ✗ ERROR: install.sh contains bash syntax errors!" >&2
  exit 1
fi

# Step 3: Detect platform architecture and determine asset name
echo "[3/4] Detecting host platform for artifact verification..."
OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
  Linux)
    TARGET_OS="linux"
    ;;
  Darwin)
    TARGET_OS="macos"
    ;;
  *)
    echo "  ✗ ERROR: Unsupported operating system: $OS" >&2
    exit 1
    ;;
esac

case "$ARCH" in
  x86_64|amd64)
    TARGET_ARCH="x86_64"
    ;;
  aarch64|arm64)
    TARGET_ARCH="aarch64"
    ;;
  *)
    echo "  ✗ ERROR: Unsupported architecture: $ARCH" >&2
    exit 1
    ;;
esac

ASSET_NAME="acc-${TARGET_ARCH}-${TARGET_OS}.tar.gz"
echo "  Detected platform asset: $ASSET_NAME"

# Step 4: Download, extract artifact and verify binary execution
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

echo "  Downloading $ASSET_NAME from release $TAG..."
gh release download "$TAG" --repo "$REPO" --pattern "$ASSET_NAME" --dir "$TMP_DIR"

if [[ ! -f "$TMP_DIR/$ASSET_NAME" ]]; then
  echo "  ✗ ERROR: Failed to download $ASSET_NAME!" >&2
  exit 1
fi

echo "  Extracting $ASSET_NAME..."
tar -xzf "$TMP_DIR/$ASSET_NAME" -C "$TMP_DIR"

ACC_BIN="$TMP_DIR/acc"
if [[ ! -x "$ACC_BIN" ]]; then
  echo "  ✗ ERROR: Extracted binary $ACC_BIN does not exist or is not executable!" >&2
  exit 1
fi

echo "[4/4] Testing extracted binary './acc --help'..."
HELP_OUTPUT="$("$ACC_BIN" --help)"
if echo "$HELP_OUTPUT" | grep -qi "usage\|help"; then
  echo "  ✓ Extracted binary acc executed successfully (exit 0, help output verified)."
else
  echo "  ✗ ERROR: Extracted binary stdout did not contain expected help string." >&2
  exit 1
fi

echo "=========================================="
echo " 🎉 All release verification checks PASSED!"
echo "=========================================="
