#!/bin/sh
set -e

GITHUB_REPO="ffalcinelli/kaji"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m'

echo -e "${BLUE}Installing kaji...${NC}"

# Detect OS
OS="$(uname -s)"
case "$OS" in
  Linux) OS_TARGET="unknown-linux-gnu" ;;
  Darwin) OS_TARGET="apple-darwin" ;;
  *) echo -e "${RED}Unsupported OS: $OS${NC}"; exit 1 ;;
esac

# Detect Architecture
ARCH="$(uname -m)"
case "$ARCH" in
  x86_64|amd64) ARCH_TARGET="x86_64" ;;
  aarch64|arm64) ARCH_TARGET="aarch64" ;;
  *) echo -e "${RED}Unsupported architecture: $ARCH${NC}"; exit 1 ;;
esac

TARGET="${ARCH_TARGET}-${OS_TARGET}"
echo "Detected platform: $TARGET"

INSTALL_DIR="${HOME}/.local/bin"
mkdir -p "$INSTALL_DIR"

VERSION=${1:-latest}
if [ "$VERSION" = "latest" ]; then
    DOWNLOAD_URL="https://github.com/$GITHUB_REPO/releases/latest/download/kaji-${TARGET}.tar.gz"
else
    DOWNLOAD_URL="https://github.com/$GITHUB_REPO/releases/download/${VERSION}/kaji-${TARGET}.tar.gz"
fi

TEMP_DIR="$(mktemp -d)"
TAR_FILE="${TEMP_DIR}/kaji.tar.gz"

echo "Downloading $DOWNLOAD_URL..."
if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$DOWNLOAD_URL" -o "$TAR_FILE"
elif command -v wget >/dev/null 2>&1; then
    wget -qO "$TAR_FILE" "$DOWNLOAD_URL"
else
    echo -e "${RED}Error: curl or wget is required.${NC}"
    rm -rf "$TEMP_DIR"; exit 1
fi

echo "Extracting..."
tar -xzf "$TAR_FILE" -C "$TEMP_DIR"

# Find and install the executable
KAJI_BIN=$(find "$TEMP_DIR" -type f -name "kaji" | head -n 1)
if [ -z "$KAJI_BIN" ]; then
    echo -e "${RED}Error: kaji executable not found in archive.${NC}"
    rm -rf "$TEMP_DIR"; exit 1
fi

chmod +x "$KAJI_BIN"
mv "$KAJI_BIN" "$INSTALL_DIR/kaji"
rm -rf "$TEMP_DIR"

echo -e "${GREEN}Successfully installed kaji to $INSTALL_DIR/kaji${NC}"

# PATH verification
if ! echo "$PATH" | grep -q "$INSTALL_DIR"; then
    echo -e "\n${BLUE}Please add the installation directory to your PATH:${NC}"
    echo "  export PATH=\"$INSTALL_DIR:\$PATH\""
    echo "You can add this to your ~/.bashrc, ~/.zshrc, or ~/.profile."
fi
