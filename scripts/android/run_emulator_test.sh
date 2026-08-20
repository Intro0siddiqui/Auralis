#!/usr/bin/env bash

# ==============================================================================
# Auralis Android Emulator End-to-End Test Runner
# ==============================================================================
# Installs APK on running Android emulator/device, grants permissions,
# forwards WebView DevTools socket, runs CDP E2E download test, and verifies
# that downloaded audio file exists in app sandboxed storage with size > 10000 bytes.
# ==============================================================================

set -eo pipefail

APK_PATH="${1:-}"

if [ -z "$APK_PATH" ]; then
    echo "Usage: $0 <path-to-apk>"
    exit 1
fi

if [ ! -f "$APK_PATH" ]; then
    echo "Error: Specified APK file does not exist: $APK_PATH"
    exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PACKAGE_NAME="com.auralis.v2"
MAIN_ACTIVITY="${PACKAGE_NAME}/.MainActivity"
SANDBOX_DOWNLOAD_DIR="/data/data/${PACKAGE_NAME}/files/downloads"
PORT="9222"

cleanup() {
    local exit_code=$?
    echo "Cleaning up port forwarding on tcp:${PORT}..."
    adb forward --remove "tcp:${PORT}" 2>/dev/null || true

    if [ "$exit_code" -ne 0 ]; then
        echo "----------------------------------------------------"
        echo "[ERROR] Test failed with exit code $exit_code. Dumping recent Android logcat:"
        echo "----------------------------------------------------"
        adb logcat -d -t 400 2>/dev/null || true
    fi
}

trap cleanup EXIT

echo "===================================================="
echo "  Auralis Android Emulator E2E Test Runner          "
echo "===================================================="
echo "APK Path:       $APK_PATH"
echo "Package Name:   $PACKAGE_NAME"
echo "Main Activity:  $MAIN_ACTIVITY"
echo "CDP Port:       $PORT"
echo "----------------------------------------------------"

# 1. Wait for emulator/device to be ready
echo "[1/6] Waiting for Android device / emulator..."
adb wait-for-device

# Wait until boot animation is completely finished
echo "Waiting for system boot completion..."
until [ "$(adb shell getprop sys.boot_completed 2>/dev/null | tr -d '\r')" = "1" ]; do
    sleep 1
done
echo "Device boot completed."

# 2. Install APK
echo "[2/6] Installing APK: $APK_PATH..."
adb install -r "$APK_PATH"

# 3. Grant runtime permissions
echo "[3/6] Granting application permissions..."
adb shell pm grant "$PACKAGE_NAME" android.permission.READ_MEDIA_AUDIO 2>/dev/null || true
adb shell pm grant "$PACKAGE_NAME" android.permission.READ_EXTERNAL_STORAGE 2>/dev/null || true
adb shell pm grant "$PACKAGE_NAME" android.permission.WRITE_EXTERNAL_STORAGE 2>/dev/null || true
adb shell pm grant "$PACKAGE_NAME" android.permission.POST_NOTIFICATIONS 2>/dev/null || true

# 4. Launch MainActivity
echo "[4/6] Launching MainActivity: $MAIN_ACTIVITY..."
adb shell am start -W -n "$MAIN_ACTIVITY"

# 5. Wait for WebView DevTools socket to appear
echo "[5/6] Waiting for WebView DevTools socket..."
DEVTOOLS_SOCKET=""
for i in $(seq 1 35); do
    SOCKET_LINE=$(adb shell "cat /proc/net/unix 2>/dev/null" | grep -E "webview_devtools_remote|devtools_remote" | head -n 1 | tr -d '\r' || true)
    if [ -n "$SOCKET_LINE" ]; then
        # Extract abstract socket name (strip leading @)
        DEVTOOLS_SOCKET=$(echo "$SOCKET_LINE" | awk '{print $NF}' | sed 's/^@//')
        if [ -n "$DEVTOOLS_SOCKET" ]; then
            echo "Found DevTools socket: $DEVTOOLS_SOCKET (detected in ${i}s)"
            break
        fi
    fi
    sleep 1
done

if [ -z "$DEVTOOLS_SOCKET" ]; then
    echo "Error: WebView devtools socket not found after 35 seconds!"
    echo "Inspecting unix sockets for package $PACKAGE_NAME:"
    adb shell "cat /proc/net/unix | grep -E 'devtools|webview|$PACKAGE_NAME'" || true
    exit 1
fi

echo "Forwarding port tcp:${PORT} -> localabstract:${DEVTOOLS_SOCKET}..."
adb forward "tcp:${PORT}" "localabstract:${DEVTOOLS_SOCKET}"

# 6. Run CDP Node.js E2E test
echo "[6/6] Executing E2E test script via Node.js..."
CDP_PORT="$PORT" node "${SCRIPT_DIR}/e2e_download_test.js"

# 7. Assert downloaded .m4a file presence and size in app sandboxed storage
echo "----------------------------------------------------"
echo "Asserting downloaded audio file in sandboxed storage..."

# Enable root if supported on userdebug / google_apis emulator
adb root 2>/dev/null || true
sleep 1

DOWNLOAD_FILES=$(adb shell "ls -l ${SANDBOX_DOWNLOAD_DIR}/*.m4a 2>/dev/null" | tr -d '\r' || true)
if [ -z "$DOWNLOAD_FILES" ] || echo "$DOWNLOAD_FILES" | grep -q "No such file"; then
    # Try run-as com.auralis.v2 fallback
    DOWNLOAD_FILES=$(adb shell "run-as ${PACKAGE_NAME} ls -l files/downloads/*.m4a 2>/dev/null" | tr -d '\r' || true)
fi

echo "Storage listing:"
echo "$DOWNLOAD_FILES"

if [ -z "$DOWNLOAD_FILES" ] || echo "$DOWNLOAD_FILES" | grep -q "No such file"; then
    echo "Error: No .m4a file found in sandboxed storage ${SANDBOX_DOWNLOAD_DIR}/"
    exit 1
fi

# Extract file size (5th column in standard ls -l)
FILE_SIZE=$(echo "$DOWNLOAD_FILES" | head -n 1 | awk '{print $5}')
if [ -z "$FILE_SIZE" ] || ! [[ "$FILE_SIZE" =~ ^[0-9]+$ ]]; then
    FILE_SIZE=$(adb shell "stat -c %s ${SANDBOX_DOWNLOAD_DIR}/*.m4a 2>/dev/null" | head -n 1 | tr -d '\r' || true)
fi

echo "Validated downloaded file size: ${FILE_SIZE} bytes"

if [ -z "$FILE_SIZE" ] || [ "$FILE_SIZE" -le 10000 ]; then
    echo "Error: Downloaded file size (${FILE_SIZE} bytes) is not greater than 10000 bytes!"
    exit 1
fi

echo "===================================================="
echo "  ✓ Android Emulator E2E Download Test PASSED!      "
echo "  File verified: >10000 bytes in ${SANDBOX_DOWNLOAD_DIR}"
echo "===================================================="
exit 0
