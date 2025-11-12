#!/usr/bin/env bash

set -e

echo "Building osu-collect..."
echo ""

# Colors
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
NC='\033[0m'

# Get the project directory
PROJECT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BUILD_DIR="${PROJECT_DIR}/build"

# Clean and create build directory
echo -e "${BLUE}Setup build directory...${NC}"
rm -rf "${BUILD_DIR}"
mkdir -p "${BUILD_DIR}"

# Get version from Cargo.toml
VERSION=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
echo -e "${BLUE}Version: ${VERSION}${NC}"
echo ""

# Build for Linux
echo -e "${GREEN}Building for Linux (x86_64-unknown-linux-gnu)...${NC}"
cargo build --release --target x86_64-unknown-linux-gnu

if [ -f "target/x86_64-unknown-linux-gnu/release/osu-collect" ]; then
    cp "target/x86_64-unknown-linux-gnu/release/osu-collect" "${BUILD_DIR}/osu-collect-linux-x64"
    chmod +x "${BUILD_DIR}/osu-collect-linux-x64"
    echo -e "${GREEN}Linux build complete: build/osu-collect-linux-x64${NC}"
else
    echo -e "${YELLOW}Linux binary not found at expected location${NC}"
fi

echo ""

# Build for Windows
echo -e "${GREEN}Building for Windows (x86_64-pc-windows-gnu)...${NC}"

# Check if Windows target is installed
if ! rustup target list | grep -q "x86_64-pc-windows-gnu (installed)"; then
    echo -e "${YELLOW}Installing Windows target...${NC}"
    rustup target add x86_64-pc-windows-gnu
fi

# Check if mingw-w64 is available
if ! command -v x86_64-w64-mingw32-gcc &> /dev/null; then
    echo -e "${YELLOW}  mingw-w64 not found.${NC}"
    echo ""
fi

cargo build --release --target x86_64-pc-windows-gnu

if [ -f "target/x86_64-pc-windows-gnu/release/osu-collect.exe" ]; then
    cp "target/x86_64-pc-windows-gnu/release/osu-collect.exe" "${BUILD_DIR}/osu-collect-windows-x64.exe"
    echo -e "${GREEN}Windows build complete: build/osu-collect-windows-x64.exe${NC}"
else
    echo -e "${YELLOW}Windows binary not found at expected location${NC}"
fi

echo ""
echo -e "${GREEN}Build complete!${NC}"
echo ""
echo "Build artifacts:"
ls -lh "${BUILD_DIR}"
echo ""
echo "Build directory: ${BUILD_DIR}"
