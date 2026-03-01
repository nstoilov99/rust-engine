#!/usr/bin/env bash
# Export script for Windows (Git Bash / MSYS2)
# Builds the standalone game and copies all required files to an output directory.
#
# Usage: ./scripts/export_windows.sh [--output-dir <path>] [--profile <release|shipping>]

set -euo pipefail

BIN_NAME="game"
OUTPUT_DIR="build/export"
PROFILE="release"

while [[ $# -gt 0 ]]; do
    case $1 in
        --output-dir) OUTPUT_DIR="$2"; shift 2 ;;
        --profile) PROFILE="$2"; shift 2 ;;
        *) echo "Unknown option: $1"; exit 1 ;;
    esac
done

echo "=== Rust Game Engine - Windows Export ==="
echo "Profile : $PROFILE"
echo "Output  : $OUTPUT_DIR"
echo ""

# Build
echo "Building ($PROFILE)..."
if [ "$PROFILE" = "shipping" ]; then
    cargo build --profile shipping
else
    cargo build --release
fi
echo "Build OK"

# Determine build output directory
if [ "$PROFILE" = "shipping" ]; then
    BUILD_DIR="target/shipping"
else
    BUILD_DIR="target/release"
fi

# Create output directory
mkdir -p "$OUTPUT_DIR"

# Copy executable
EXE_PATH="$BUILD_DIR/$BIN_NAME.exe"
if [ -f "$EXE_PATH" ]; then
    cp "$EXE_PATH" "$OUTPUT_DIR/"
    SIZE=$(du -h "$EXE_PATH" | cut -f1)
    echo "Copied $BIN_NAME.exe ($SIZE)"
else
    echo "ERROR: $EXE_PATH not found"
    exit 1
fi

# Copy DLLs
for dll in "$BUILD_DIR"/*.dll; do
    [ -f "$dll" ] || continue
    cp "$dll" "$OUTPUT_DIR/"
    echo "Copied $(basename "$dll")"
done

# Pack content into game.pak
if [ -d "content" ]; then
    echo "Packing content/ into game.pak..."
    if cargo run --release --bin pak_tool -- pack content "$OUTPUT_DIR/game.pak"; then
        SIZE=$(du -h "$OUTPUT_DIR/game.pak" | cut -f1)
        echo "Created game.pak ($SIZE)"
    else
        echo "WARNING: pak_tool failed, falling back to raw copy"
        rm -rf "$OUTPUT_DIR/content"
        cp -r content "$OUTPUT_DIR/content"
        FILE_COUNT=$(find "$OUTPUT_DIR/content" -type f | wc -l)
        echo "Copied content/ ($FILE_COUNT files)"
    fi
else
    echo "WARNING: content/ directory not found"
fi

echo ""
echo "=== Export complete: $OUTPUT_DIR ==="
