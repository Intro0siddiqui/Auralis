#!/usr/bin/env bash
# ============================================================================
# Auralis v2 — Build Cache Guard
# Monitors Cargo & Android build cache sizes and cleans them when exceeding
# the configured threshold (default: 7 GB).
# ============================================================================
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET_DIR="$ROOT/target"
ANDROID_BUILD_DIR="$ROOT/gen/android/app/build"

# Threshold in Gigabytes (default: 7 GB)
THRESHOLD_GB="${AURALIS_MAX_CACHE_GB:-7}"
THRESHOLD_MB=$(( THRESHOLD_GB * 1024 ))

get_size_mb() {
    local dir="$1"
    if [ -d "$dir" ]; then
        du -sm "$dir" 2>/dev/null | awk '{print $1}'
    else
        echo "0"
    fi
}

format_mb() {
    local mb="$1"
    if [ "$mb" -ge 1024 ]; then
        awk -v m="$mb" 'BEGIN { printf "%.2f GB", m / 1024 }'
    else
        echo "${mb} MB"
    fi
}

FORCE_CLEAN=0
CHECK_ONLY=0

for arg in "$@"; do
    case "$arg" in
        --force|-f)
            FORCE_CLEAN=1
            ;;
        --check|-c)
            CHECK_ONLY=1
            ;;
        *)
            ;;
    esac
done

TARGET_MB=$(get_size_mb "$TARGET_DIR")
ANDROID_MB=$(get_size_mb "$ANDROID_BUILD_DIR")
TOTAL_MB=$(( TARGET_MB + ANDROID_MB ))

if [ "$CHECK_ONLY" -eq 1 ]; then
    echo "==> Auralis Build Cache Status:"
    echo "    Cargo target/:        $(format_mb "$TARGET_MB")"
    echo "    Android app/build/:   $(format_mb "$ANDROID_MB")"
    echo "    Total build cache:    $(format_mb "$TOTAL_MB") (Threshold: ${THRESHOLD_GB} GB)"
    exit 0
fi

if [ "$FORCE_CLEAN" -eq 1 ] || [ "$TOTAL_MB" -ge "$THRESHOLD_MB" ]; then
    if [ "$FORCE_CLEAN" -eq 1 ]; then
        echo "==> [CacheGuard] Force clean requested."
    else
        echo "==> [CacheGuard] ALERT: Build cache ($(format_mb "$TOTAL_MB")) exceeded threshold of ${THRESHOLD_GB} GB!"
    fi

    # 1. Preserve release binary and release APK if they exist
    BACKUP_BIN=""
    if [ -f "$TARGET_DIR/release/auralis" ]; then
        BACKUP_BIN="$(mktemp /tmp/auralis-release-XXXXXX)"
        cp "$TARGET_DIR/release/auralis" "$BACKUP_BIN"
    fi
    BACKUP_APK=""
    APK_PATH="$ANDROID_BUILD_DIR/outputs/apk/universal/release/app-universal-release.apk"
    if [ -f "$APK_PATH" ]; then
        BACKUP_APK="$(mktemp /tmp/auralis-apk-XXXXXX.apk)"
        cp "$APK_PATH" "$BACKUP_APK"
    fi

    # 2. Run cargo clean
    echo "    Running cargo clean..."
    (cd "$ROOT" && cargo clean 2>/dev/null || rm -rf "$TARGET_DIR")

    # 3. Clean Android Gradle build directory if present
    if [ -d "$ANDROID_BUILD_DIR" ]; then
        echo "    Cleaning Android gradle cache..."
        rm -rf "$ANDROID_BUILD_DIR" "$ROOT/gen/android/build" 2>/dev/null || true
    fi

    # 4. Restore preserved release binary and release APK
    if [ -n "$BACKUP_BIN" ] && [ -f "$BACKUP_BIN" ]; then
        mkdir -p "$TARGET_DIR/release"
        cp "$BACKUP_BIN" "$TARGET_DIR/release/auralis"
        chmod +x "$TARGET_DIR/release/auralis"
        rm -f "$BACKUP_BIN"
    fi
    if [ -n "$BACKUP_APK" ] && [ -f "$BACKUP_APK" ]; then
        mkdir -p "$(dirname "$APK_PATH")"
        cp "$BACKUP_APK" "$APK_PATH"
        rm -f "$BACKUP_APK"
    fi

    NEW_TARGET_MB=$(get_size_mb "$TARGET_DIR")
    NEW_ANDROID_MB=$(get_size_mb "$ANDROID_BUILD_DIR")
    NEW_TOTAL_MB=$(( NEW_TARGET_MB + NEW_ANDROID_MB ))

    echo "✓ [CacheGuard] Cleanup complete! Cache reclaimed from $(format_mb "$TOTAL_MB") to $(format_mb "$NEW_TOTAL_MB")."
else
    # Silent if under threshold, or quiet confirmation if run directly
    if [ "${TERM:-}" != "dumb" ] && [ -t 1 ]; then
        echo "✓ [CacheGuard] Cache OK: $(format_mb "$TOTAL_MB") / ${THRESHOLD_GB} GB limit."
    fi
fi
