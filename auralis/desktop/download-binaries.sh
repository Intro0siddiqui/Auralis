#!/bin/bash
# Download bundled binaries for Auralis desktop
# Run this script before building: ./download-binaries.sh

set -e

RESOURCES_DIR="src/main/resources/bin"
mkdir -p "$RESOURCES_DIR"

# Download yt-dlp (standalone binary, no Python needed)
echo "Downloading yt-dlp..."
curl -L -o "$RESOURCES_DIR/yt-dlp" "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp"
chmod +x "$RESOURCES_DIR/yt-dlp"

echo "Binaries downloaded to $RESOURCES_DIR"
echo "Note: ffmpeg must be installed separately on the system"
