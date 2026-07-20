#!/usr/bin/env bash
# Export script for Windows (Git Bash / MSYS2)
# Builds the standalone game and copies all required files to an output directory.
#
# Usage: ./scripts/export_windows.sh [--output-dir <path>] [--profile <release|shipping>]
#                                    [--target <standalone|mp-client>]
#                                    [--server-uri <uri>] [--module <name>]
#
# Targets (M9 D2): same binary either way — the target is configuration.
#   standalone : no net config in the bundle (and deletes a stale one)
#   mp-client  : writes net_config.ron (auto_connect) next to the exe

set -euo pipefail

BIN_NAME="game"
OUTPUT_DIR="build/export"
PROFILE="release"
TARGET="standalone"
SERVER_URI="http://127.0.0.1:3000"
MODULE="rust-engine-dev"

while [[ $# -gt 0 ]]; do
    case $1 in
        --output-dir) OUTPUT_DIR="$2"; shift 2 ;;
        --profile) PROFILE="$2"; shift 2 ;;
        --target) TARGET="$2"; shift 2 ;;
        --server-uri) SERVER_URI="$2"; shift 2 ;;
        --module) MODULE="$2"; shift 2 ;;
        *) echo "Unknown option: $1"; exit 1 ;;
    esac
done

if [[ "$TARGET" != "standalone" && "$TARGET" != "mp-client" ]]; then
    echo "ERROR: invalid target '$TARGET' (standalone|mp-client)"; exit 1
fi
if [[ "$TARGET" == "mp-client" ]]; then
    if [[ ! "$MODULE" =~ ^[a-z0-9]+(-[a-z0-9]+)*$ ]]; then
        echo "ERROR: invalid module name '$MODULE' (must match ^[a-z0-9]+(-[a-z0-9]+)*\$)"; exit 1
    fi
    if [[ ! "$SERVER_URI" =~ ^https?:// ]]; then
        echo "ERROR: invalid server URI '$SERVER_URI' (must be http(s)://...)"; exit 1
    fi
fi

echo "=== Rust Game Engine - Windows Export ==="
echo "Profile : $PROFILE"
echo "Target  : $TARGET"
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

# Cook static collision for all scenes (skips up-to-date cooks)
if compgen -G "content/scenes/*.scene" > /dev/null; then
    echo "Cooking static collision..."
    for scene in content/scenes/*.scene; do
        if ! cargo run --release --bin collision_cooker -- "scenes/$(basename "$scene")"; then
            echo "ERROR: collision cook failed for $(basename "$scene")"
            exit 1
        fi
    done
fi

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

# Net config (M9 D2): targets own their marker files — standalone deletes a
# stale config so re-exporting over an mp-client bundle can't auto-connect.
NET_CONFIG="$OUTPUT_DIR/net_config.ron"
if [[ "$TARGET" == "mp-client" ]]; then
    cat > "$NET_CONFIG" <<EOF
NetConfig(
    host: "$SERVER_URI",
    module: "$MODULE",
    auto_connect: true,
)
EOF
    echo "Wrote net_config.ron -> $SERVER_URI / $MODULE"
elif [[ -f "$NET_CONFIG" ]]; then
    rm -f "$NET_CONFIG"
    echo "Removed stale net_config.ron (standalone target)"
fi

echo ""
echo "=== Export complete: $OUTPUT_DIR ($TARGET) ==="
