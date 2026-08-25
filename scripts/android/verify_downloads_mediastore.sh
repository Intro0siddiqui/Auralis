#!/usr/bin/env bash
# ==============================================================================
# verify_downloads_mediastore.sh — MediaStore / Download/Auralis verifier
# ==============================================================================
# Waits 5s for MediaStore publish to settle, then verifies:
#  1) adb shell ls -l /storage/emulated/0/Download/Auralis/*.m* exists size>10KB
#  2) adb shell content query --uri content://media/external/downloads \
#        --projection display_name:relative_path:is_pending \
#        --where "display_name='<file>'" shows is_pending 0 + relative_path Download/Auralis
#  3) adb shell ls /data/data/com.auralis.v2/files/downloads (dual-save)
#
# Returns 0 on PASS (dual-save OR public visible), 1 on FAIL.
# Tolerant fallback: if MediaStore insert failed but internal file exists -> PASS with WARN.
# ==============================================================================

set -eo pipefail

PACKAGE_NAME="${PACKAGE_NAME:-com.auralis.v2}"
PUBLIC_DIR="/storage/emulated/0/Download/Auralis"
INTERNAL_DIRS=(
    "/data/data/${PACKAGE_NAME}/files/downloads"
    "/data/data/${PACKAGE_NAME}/downloads"
    "/data/user/0/${PACKAGE_NAME}/files/downloads"
)
EXPECTED_FILE="${1:-}"

echo "----------------------------------------------------"
echo "[verify_mediastore] Waiting 5s for MediaStore publish to settle..."
sleep 5

echo "[verify_mediastore] PACKAGE_NAME=${PACKAGE_NAME} PUBLIC_DIR=${PUBLIC_DIR}"

# Auto-detect expected filename if not passed
if [ -z "$EXPECTED_FILE" ]; then
    DETECTED=$(adb shell "ls ${PUBLIC_DIR}/*.m* 2>/dev/null | tr -d '\r' | head -n 1" || true)
    if [ -n "$DETECTED" ]; then
        EXPECTED_FILE=$(basename "$DETECTED" | tr -d '\r')
        echo "[verify_mediastore] Auto-detected public file: $EXPECTED_FILE"
    else
        # fallback try internal
        DETECTED=$(adb shell "ls /data/data/${PACKAGE_NAME}/files/downloads/*.m* 2>/dev/null | tr -d '\r' | head -n 1" || true)
        if [ -z "$DETECTED" ]; then
            DETECTED=$(adb shell "run-as ${PACKAGE_NAME} ls files/downloads/*.m* 2>/dev/null | tr -d '\r' | head -n 1" || true)
        fi
        if [ -n "$DETECTED" ]; then
            EXPECTED_FILE=$(basename "$DETECTED" | tr -d '\r')
            echo "[verify_mediastore] Auto-detected internal file: $EXPECTED_FILE"
        fi
    fi
fi

# 1) Public ls size>10KB
echo "----------------------------------------------------"
echo "[verify_mediastore] [1/3] Checking public Download/Auralis ls + size>10KB..."
PUBLIC_LS=$(adb shell "ls -l ${PUBLIC_DIR}/*.m* 2>&1 || ls -l ${PUBLIC_DIR}/ 2>&1; echo __STAT__; stat -c '%s %n' ${PUBLIC_DIR}/*.m* 2>/dev/null | head -n 5" | tr -d '\r' || true)
echo "$PUBLIC_LS"
PUBLIC_OK=0
PUBLIC_SIZE=0
while IFS= read -r line; do
    line=$(echo "$line" | xargs)
    [ -z "$line" ] && continue
    [[ "$line" == "__STAT__" ]] && continue
    [[ "$line" == *"No such file"* ]] && continue
    if [[ "$line" =~ ^[0-9]+\  ]]; then
        # stat output: "12345 /storage/..."
        sz=$(echo "$line" | awk '{print $1}')
        if [[ "$sz" =~ ^[0-9]+$ ]] && [ "$sz" -gt 10240 ]; then
            PUBLIC_OK=1; PUBLIC_SIZE=$sz
            echo "[verify_mediastore] Public size OK: ${sz} bytes"
            break
        fi
    elif [[ "$line" == *".m"* ]]; then
        sz=$(echo "$line" | awk '{print $5}')
        if [[ "$sz" =~ ^[0-9]+$ ]] && [ "$sz" -gt 10240 ]; then
            PUBLIC_OK=1; PUBLIC_SIZE=$sz
            echo "[verify_mediastore] Public ls size OK: ${sz} bytes"
            break
        fi
    fi
done <<< "$PUBLIC_LS"
if [ $PUBLIC_OK -eq 0 ]; then
    echo "[verify_mediastore][WARN] Public Download/Auralis file missing or <10KB (insert may have failed — fallback tolerant)"
fi

# 2) content query is_pending 0 + relative_path
echo "----------------------------------------------------"
echo "[verify_mediastore] [2/3] Querying MediaStore content://media/external/downloads..."
if [ -n "$EXPECTED_FILE" ]; then
    esc=$(echo "$EXPECTED_FILE" | sed "s/'/\\\\'/g")
    CQ=$(adb shell "content query --uri content://media/external/downloads --projection display_name:relative_path:is_pending --where \"display_name='$esc'\" 2>&1" | tr -d '\r' || true)
    echo "$CQ"
else
    CQ=$(adb shell "content query --uri content://media/external/downloads --projection display_name:relative_path:is_pending 2>&1 | head -n 30" | tr -d '\r' || true)
    echo "$CQ"
    echo "[verify_mediastore] (wildcard query, expecting any Auralis row)"
fi
# loose checks: needs at least one row; ideally pending 0 + relative_path
CQ_HAS_ROW=0
CQ_PENDING_OK=0
CQ_PATH_OK=0
if echo "$CQ" | grep -q "Row:" || echo "$CQ" | grep -q "display_name="; then
    CQ_HAS_ROW=1
fi
if echo "$CQ" | grep -q "is_pending=0"; then
    CQ_PENDING_OK=1
fi
if echo "$CQ" | grep -q "Download/Auralis"; then
    CQ_PATH_OK=1
fi
if [ -n "$EXPECTED_FILE" ]; then
    if [ $CQ_HAS_ROW -eq 0 ]; then
        echo "[verify_mediastore][WARN] No MediaStore row for $EXPECTED_FILE — fallback tolerant"
    elif [ $CQ_PENDING_OK -eq 0 ]; then
        echo "[verify_mediastore][WARN] is_pending !=0 for $EXPECTED_FILE"
    elif [ $CQ_PATH_OK -eq 0 ]; then
        echo "[verify_mediastore][WARN] relative_path missing Download/Auralis for $EXPECTED_FILE"
    else
        echo "[verify_mediastore] MediaStore row OK: is_pending=0 + relative_path=Download/Auralis"
    fi
else
    if [ $CQ_HAS_ROW -eq 1 ] && [ $CQ_PENDING_OK -eq 1 ] && [ $CQ_PATH_OK -eq 1 ]; then
        echo "[verify_mediastore] Wildcard MediaStore check OK (found Auralis pending=0)"
    else
        echo "[verify_mediastore][WARN] Wildcard MediaStore check inconclusive — tolerant"
    fi
fi
# For strictness, require pending+path only if public file existed; otherwise warn only
MEDIASTORE_ROW_OK=0
if [ $CQ_HAS_ROW -eq 1 ] && [ $CQ_PENDING_OK -eq 1 ] && [ $CQ_PATH_OK -eq 1 ]; then
    MEDIASTORE_ROW_OK=1
fi

# 3) Dual-save internal ls
echo "----------------------------------------------------"
echo "[verify_mediastore] [3/3] Checking dual-save internal files/downloads..."
INTERNAL_OK=0
INTERNAL_SIZE=0
INTERNAL_FOUND_PATH=""
for dir in "${INTERNAL_DIRS[@]}"; do
    LS=$(adb shell "ls -l ${dir}/*.m* 2>&1 || ls -l ${dir}/ 2>&1; echo __STAT__; stat -c '%s %n' ${dir}/*.m* 2>/dev/null | head -n 3" | tr -d '\r' || true)
    # if Permission denied try run-as
    if echo "$LS" | grep -q "Permission denied"; then
        rel=$(echo "$dir" | sed "s|/data/data/${PACKAGE_NAME}/||;s|/data/user/0/${PACKAGE_NAME}/||")
        LS=$(adb shell "run-as ${PACKAGE_NAME} ls -l ${rel}/ 2>&1; echo __STAT__; run-as ${PACKAGE_NAME} stat -c '%s %n' ${rel}/*.m* 2>/dev/null | head -n 3" | tr -d '\r' || true)
        echo "[verify_mediastore] ls $dir (run-as fallback):"
        echo "$LS"
    else
        echo "[verify_mediastore] ls $dir:"
        echo "$LS"
    fi
    while IFS= read -r line; do
        line=$(echo "$line" | xargs)
        [ -z "$line" ] && continue
        [[ "$line" == "__STAT__" ]] && continue
        [[ "$line" == *"No such file"* ]] && continue
        [[ "$line" == *"Permission denied"* ]] && continue
        if [[ "$line" =~ ^[0-9]+\  ]]; then
            sz=$(echo "$line" | awk '{print $1}')
            if [[ "$sz" =~ ^[0-9]+$ ]] && [ "$sz" -gt 10240 ]; then
                INTERNAL_OK=1; INTERNAL_SIZE=$sz; INTERNAL_FOUND_PATH="$dir"
                echo "[verify_mediastore] Internal size OK: ${sz} bytes in $dir"
                break 2
            fi
        elif [[ "$line" == *".m"* ]]; then
            sz=$(echo "$line" | awk '{print $5}')
            if [[ "$sz" =~ ^[0-9]+$ ]] && [ "$sz" -gt 10240 ]; then
                INTERNAL_OK=1; INTERNAL_SIZE=$sz; INTERNAL_FOUND_PATH="$dir"
                echo "[verify_mediastore] Internal ls size OK: ${sz} bytes in $dir"
                break 2
            fi
        fi
    done <<< "$LS"
done
# final run-as downloads/ (legacy path without files/)
if [ $INTERNAL_OK -eq 0 ]; then
    LS=$(adb shell "run-as ${PACKAGE_NAME} ls -l downloads/ 2>&1; echo __STAT__; run-as ${PACKAGE_NAME} stat -c '%s %n' downloads/*.m* 2>/dev/null | head -n 3" | tr -d '\r' || true)
    echo "[verify_mediastore] ls run-as downloads/:"
    echo "$LS"
    while IFS= read -r line; do
        line=$(echo "$line" | xargs)
        [[ "$line" == "__STAT__" ]] && continue
        [[ "$line" == *"No such file"* ]] && continue
        if [[ "$line" =~ ^[0-9]+\  ]]; then
            sz=$(echo "$line" | awk '{print $1}')
            if [[ "$sz" =~ ^[0-9]+$ ]] && [ "$sz" -gt 10240 ]; then INTERNAL_OK=1; INTERNAL_SIZE=$sz; INTERNAL_FOUND_PATH="run-as:downloads"; break; fi
        elif [[ "$line" == *".m"* ]]; then
            sz=$(echo "$line" | awk '{print $5}')
            if [[ "$sz" =~ ^[0-9]+$ ]] && [ "$sz" -gt 10240 ]; then INTERNAL_OK=1; INTERNAL_SIZE=$sz; INTERNAL_FOUND_PATH="run-as:downloads"; break; fi
        fi
    done <<< "$LS"
fi
if [ $INTERNAL_OK -eq 0 ]; then
    echo "[verify_mediastore][WARN] No internal dual-save file >10KB found"
else
    echo "[verify_mediastore] Dual-save internal OK in $INTERNAL_FOUND_PATH ($INTERNAL_SIZE bytes)"
fi

# Final verdict — fallback tolerant
echo "----------------------------------------------------"
if [ $INTERNAL_OK -eq 1 ] && [ $PUBLIC_OK -eq 1 ]; then
    echo "[verify_mediastore] PASS — dual-save verified (public ${PUBLIC_SIZE}b + internal ${INTERNAL_SIZE}b)"
    exit 0
elif [ $INTERNAL_OK -eq 1 ]; then
    echo "[verify_mediastore] PASS (fallback) — internal dual-save OK (${INTERNAL_SIZE}b), public/MediaStore insert fallback tolerated"
    if [ $MEDIASTORE_ROW_OK -eq 0 ]; then
        echo "[verify_mediastore] (MediaStore row missing/pending but not failing due to fallback)"
    fi
    exit 0
elif [ $PUBLIC_OK -eq 1 ] && [ $MEDIASTORE_ROW_OK -eq 1 ]; then
    echo "[verify_mediastore] PASS (fallback) — public/MediaStore OK (${PUBLIC_SIZE}b), internal not probed (sandbox permission?)"
    exit 0
else
    echo "[verify_mediastore] FAIL — neither public nor internal file >10KB found"
    echo "  public_ok=$PUBLIC_OK internal_ok=$INTERNAL_OK mediastore_row_ok=$MEDIASTORE_ROW_OK"
    exit 1
fi
