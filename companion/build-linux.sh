#!/bin/bash
# Build script for Enkodu companion on Linux
#
# Requirements:
# - Rust (stable toolchain)
# - notify-send (for desktop notifications)
# - xdg-utils (for opening URLs/files)
# - ffprobe (for video file verification)

set -e

BIN_NAME="enkodu"
TARGET_DIR="target/release"

# Build in release mode
echo "Building Enkodu companion for Linux..."
cargo build --release

# Create a tarball for distribution
echo "Creating distribution tarball..."
TARBALL="${BIN_NAME}-linux-x86_64.tar.gz"

# Create a temporary directory for the release
RELEASE_DIR=$(mktemp -d)
trap "rm -rf $RELEASE_DIR" EXIT

# Copy the binary
cp "${TARGET_DIR}/${BIN_NAME}" "$RELEASE_DIR/"

# Copy README if it exists
if [ -f "README.md" ]; then
    cp README.md "$RELEASE_DIR/"
fi

# Create tarball
cd "$RELEASE_DIR"
tar czf "../${TARBALL}" ./*
cd -

mv "$RELEASE_DIR/../${TARBALL}" .

echo "Build complete!"
echo "Distribution: ${TARBALL}"
echo ""
echo "To install:"
echo "  1. Extract the tarball: tar xzf ${TARBALL}"
echo "  2. Copy the binary to /usr/local/bin: sudo cp ${BIN_NAME} /usr/local/bin/"
echo "  3. Run: ${BIN_NAME}"
echo ""
echo "Note: Ensure notify-send, xdg-open, and ffprobe are installed on your system."
