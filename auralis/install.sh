#!/bin/bash
set -e

INSTALL_DIR="$HOME/.local/share/auralis"
BIN_DIR="$HOME/.local/bin"
DESKTOP_DIR="$HOME/.local/share/applications"
ICON_DIR="$HOME/.local/share/icons"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

echo "Installing Auralis..."

mkdir -p "$INSTALL_DIR" "$BIN_DIR" "$DESKTOP_DIR" "$ICON_DIR"

JAR_PATH="$SCRIPT_DIR/desktop/build/libs/desktop.jar"
if [ ! -f "$JAR_PATH" ]; then
    echo "Building Auralis..."
    cd "$SCRIPT_DIR"
    ./gradlew :desktop:jar
    JAR_PATH="$(pwd)/desktop/build/libs/desktop.jar"
fi

cp "$JAR_PATH" "$INSTALL_DIR/auralis.jar"

cat > "$BIN_DIR/auralis" << 'LAUNCHER'
#!/bin/bash
exec java -Xmx512m -jar "$HOME/.local/share/auralis/auralis.jar" "$@"
LAUNCHER
chmod +x "$BIN_DIR/auralis"

cp "$SCRIPT_DIR/desktop/src/main/resources/auralis.desktop" "$DESKTOP_DIR/auralis.desktop"

if [ -f "$SCRIPT_DIR/desktop/icon.png" ]; then
    cp "$SCRIPT_DIR/desktop/icon.png" "$ICON_DIR/auralis.png"
fi

update-desktop-database "$DESKTOP_DIR" 2>/dev/null || true

echo ""
echo "Auralis installed successfully!"
echo "  Binary: $BIN_DIR/auralis"
echo "  Jar:    $INSTALL_DIR/auralis.jar"
echo ""
echo "Run 'auralis' from terminal or find it in your app launcher."
echo ""
echo "Distro-specific packages:"
echo "  Debian/Ubuntu:  sudo dpkg -i desktop/build/compose/binaries/main/deb/*.deb"
echo "  Fedora/RHEL:    sudo rpm -i desktop/build/compose/binaries/main/rpm/*.rpm"
echo "  Arch Linux:     sudo pacman -U desktop/build/compose/binaries/main/arch/*.pkg.tar.zst"
echo "  Any Linux:      tar xzf desktop/build/app-image/Auralis-linux-x86_64.tar.gz"
