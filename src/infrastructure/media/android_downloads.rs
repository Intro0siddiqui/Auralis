//! Android MediaStore Downloads publisher
//!
//! Publishes a finished download from internal `app_data_dir/downloads` to the
//! user-visible `Download/Auralis/` folder via `MediaStore.Downloads` on
//! API 29+ (`IS_PENDING` protocol, no permissions needed for own files).
//! On API 26-28 falls back to `Environment.getExternalStoragePublicDirectory`
//! + `MediaScannerConnection.scanFile`.
//!
//! On non-Android or on any JNI failure,
//! returns `None` and the caller keeps the internal path.
//!
//! Downloader keeps `Range` pause/resume working by streaming to the internal
//! tmp first; this module is only called once on `complete` to copy.
//!
//! # The pending-row invariant (DL-07)
//!
//! The API 29+ branch creates its `MediaStore` row **invisible**
//! (`is_pending = 1`): `MediaProvider` hides such a row from the file manager,
//! from `MediaStore` queries and from the media scanner, and keeps the display
//! name reserved for as long as the row exists. A row that is inserted pending
//! and then dropped on the floor is therefore worse than no row at all — the
//! file "does not exist" *and* the next publish of the same track collides
//! with the ghost.
//!
//! So: **a row inserted pending is always either made visible or removed.**
//! Every exit path after the insert funnels through
//! [`resolve_pending_row`], which clears `is_pending` when the byte copy
//! completed and deletes the row in every other case (including a clear that
//! failed or matched no rows). The decision itself lives in the pure
//! [`next_step`] so it can be unit-tested without a device.

use std::path::Path;
#[cfg(target_os = "android")]
use tracing::{info, warn};

#[cfg(target_os = "android")]
use jni::{
    objects::{JObject, JString, JValue},
    JNIEnv,
};

/// `MediaStore.MediaColumns` names, spelled out so this module keeps building
/// (and behaving identically) without the Android SDK on the host.
#[cfg(target_os = "android")]
const COLUMN_IS_PENDING: &str = "is_pending";
#[cfg(target_os = "android")]
const COLUMN_DISPLAY_NAME: &str = "display_name";
#[cfg(target_os = "android")]
const COLUMN_MIME_TYPE: &str = "mime_type";
#[cfg(target_os = "android")]
const COLUMN_RELATIVE_PATH: &str = "relative_path";

/// Where the public copy lives, as a `MediaStore` relative path (API 29+) and
/// as the absolute path we hand back to the caller. The legacy branch builds
/// its destination from `Environment`, which is the same directory in practice;
/// if a device ever disagrees, the public path we return is still the one the
/// Q+ branch writes to.
#[cfg(target_os = "android")]
const PUBLIC_RELATIVE_PATH: &str = "Download/Auralis";
#[cfg(target_os = "android")]
const PUBLIC_ABSOLUTE_DIR: &str = "/storage/emulated/0/Download/Auralis";

/// Chunk size for the `OutputStream.write([B)` loop. A local-ref array per
/// chunk on a permanently attached thread, so keep it modest.
#[cfg(target_os = "android")]
const COPY_CHUNK_BYTES: usize = 64 * 1024;

/// MIME for a download ext, for MediaStore DISPLAY.
pub fn mime_for_ext(ext: &str) -> &'static str {
    match ext.to_ascii_lowercase().as_str() {
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "aac" => "audio/aac",
        "flac" => "audio/flac",
        "ogg" | "oga" => "audio/ogg",
        "opus" => "audio/opus",
        "wav" => "audio/wav",
        "webm" => "audio/webm",
        "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        _ => "audio/mpeg",
    }
}

/// Try to publish `src_path` (already fully written internal file) to
/// `Download/Auralis/<display_name>` via MediaStore.
/// Returns the public absolute path string (`/storage/emulated/0/Download/Auralis/...`)
/// on success, or `None` on non-Android / sdk<26 / JNI failure (caller keeps internal).
pub fn publish_to_downloads(src_path: &Path) -> Option<String> {
    #[cfg(not(target_os = "android"))]
    {
        let _ = src_path;
        None
    }
    #[cfg(target_os = "android")]
    {
        let display = src_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "audio_track.mp3".to_string());
        let ext = src_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("mp3");
        let mime = mime_for_ext(ext);
        match publish_inner(src_path, &display, mime) {
            Ok(public) => {
                info!(src = %src_path.display(), public = %public, "Published to Download/Auralis via MediaStore");
                Some(public)
            }
            Err(e) => {
                // `e` already carries the display name, the row id, the API
                // level and the JNI error string, because a release build has
                // no logcat and this line is the only evidence that survives.
                warn!(src = %src_path.display(), display = %display, error = %e, "MediaStore publish failed, keeping internal path");
                None
            }
        }
    }
}

#[cfg(target_os = "android")]
fn sdk_int(env: &mut JNIEnv<'_>) -> i32 {
    env.get_static_field("android/os/Build$VERSION", "SDK_INT", "I")
        .map(|v| v.i().unwrap_or(26))
        .unwrap_or(26)
}

#[cfg(target_os = "android")]
fn publish_inner(src_path: &Path, display_name: &str, mime: &str) -> Result<String, String> {
    let file_len = std::fs::metadata(src_path).map(|m| m.len()).unwrap_or(0);
    if file_len == 0 {
        return Err("source file empty or missing".into());
    }

    // Read source bytes in Rust (chunked later via JNI writes). For files < 50 MB this is fine;
    // for larger we still read fully — alternative would be fd dup but simpler to keep.
    // We read lazily in the JNI block to avoid holding env across I/O.

    with_attached_env(|env| {
        let sdk = sdk_int(env);
        let ctx = service_context().ok_or_else(|| "no android context".to_string())?;

        // resolver = ctx.getContentResolver()
        let resolver = env
            .call_method(
                &ctx,
                "getContentResolver",
                "()Landroid/content/ContentResolver;",
                &[],
            )
            .map_err(|e| format!("getContentResolver: {e}"))?
            .l()
            .map_err(|e| format!("resolver l: {e}"))?;

        if sdk >= 29 {
            // Q+ MediaStore path with IS_PENDING
            publish_q(env, &resolver, &ctx, sdk, src_path, display_name, mime)
        } else {
            publish_legacy(env, &resolver, &ctx, sdk, src_path, display_name)
        }
    })
    .ok_or_else(|| "JNI env unavailable".to_string())?
}

/// How the byte copy into the MediaStore stream ended.
#[cfg(any(target_os = "android", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CopyOutcome {
    /// Every source byte reached the row's file and the stream closed cleanly
    /// — the row now holds the whole track and is worth publishing.
    Complete,
    /// Anything else: the stream never opened, the source could not be read, a
    /// write/flush/close failed. The row holds a truncated file (or nothing)
    /// and must never become visible.
    Failed,
}

/// What the `is_pending = 0` update reported.
#[cfg(any(target_os = "android", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClearOutcome {
    /// Update was not attempted because the copy was not complete.
    NotAttempted,
    /// `ContentResolver.update` reported at least one affected row.
    Updated,
    /// It reported 0 rows — the provider did not match the row, so visibility
    /// was *not* achieved. Treating this as success is what let an invisible
    /// file ship unnoticed before.
    NoRows,
    /// The call itself failed (a thrown Java exception, a bad signature …).
    Failed,
}

/// The two ways a pending row can be resolved.
#[cfg(any(target_os = "android", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingStep {
    Clear,
    Delete,
}

/// The whole DL-07 invariant as a decision table, with no JNI in it so it can
/// be unit-tested off-device:
///
/// * a row is made visible **only** when the byte copy completed, and
/// * if the visibility step did not demonstrably happen, the row is removed.
///
/// An unclearable row is deleted rather than left pending: an invisible row is
/// not merely useless, it keeps `Download/Auralis/<name>` reserved so the next
/// publish of the same track collides with a ghost nobody can see or remove.
/// Deleting is safe because the URI addresses a row this process created
/// moments ago, and because the internal copy the library actually plays is
/// never touched from here.
#[cfg(any(target_os = "android", test))]
fn next_step(copy: CopyOutcome, clear: ClearOutcome) -> Option<PendingStep> {
    match (copy, clear) {
        (CopyOutcome::Complete, ClearOutcome::NotAttempted) => Some(PendingStep::Clear),
        (CopyOutcome::Complete, ClearOutcome::Updated) => None,
        (CopyOutcome::Complete, ClearOutcome::NoRows | ClearOutcome::Failed) => {
            Some(PendingStep::Delete)
        }
        // An incomplete copy is never published, whatever the clear said.
        (CopyOutcome::Failed, _) => Some(PendingStep::Delete),
    }
}

/// What [`resolve_pending_row`] did with the row. `Unresolved` is the only
/// state that can still leave something pending, so it is the one that has to
/// be loud in the log.
#[cfg(target_os = "android")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingOutcome {
    /// `is_pending = 0` was applied; the file is in the file manager.
    Visible,
    /// The row is gone; nothing pending was left behind.
    Removed,
    /// Deleting the row also failed. The row is still there and still
    /// invisible, and only a human can now clean it up.
    Unresolved,
}

/// A `MediaStore` row this process has just inserted and not yet resolved.
///
/// Deliberately *not* a `Drop` guard: resolving the row needs `&mut JNIEnv`,
/// which a `drop` impl cannot obtain safely (JNI during unwinding, and no way
/// to report the outcome), and an implicit resolver would hide the very exit
/// paths that leak rows in the first place. Every caller resolves the row
/// explicitly, on purpose, in the order the protocol requires.
#[cfg(target_os = "android")]
struct PendingRow<'local> {
    uri: JObject<'local>,
    /// Row id, i.e. the last path segment of the uri. Log-only: update/delete
    /// address the row by uri, but a human needs the id to clean up by hand.
    id: String,
    /// Full uri text, log-only, so a leaked row can be deleted with
    /// `content delete --uri <this>`.
    uri_string: String,
    display_name: String,
    api: i32,
}

/// The one place a pending row is made visible or removed.
///
/// Every exit path of [`publish_q`] after a successful insert must reach this
/// — the success path included. Holding the invariant across early `return`s is
/// the entire point of funnelling them all through one function.
#[cfg(target_os = "android")]
fn resolve_pending_row<'local>(
    env: &mut JNIEnv<'local>,
    resolver: &JObject<'_>,
    row: &PendingRow<'local>,
    copy: CopyOutcome,
) -> PendingOutcome {
    // `jni` reports a thrown Java exception as an `Err` but deliberately
    // leaves it *pending*, and the JNI spec only sanctions a handful of calls
    // (ExceptionCheck/Clear/Describe among them) while one is pending. Every
    // caller arrives straight off a failed JNI call, so drain it here first —
    // otherwise the update/delete below is undefined behaviour and can abort
    // the VM instead of merely failing.
    if let Some(exception) = take_pending_exception(env) {
        warn!(
            display_name = %row.display_name,
            row_id = %row.id,
            api = row.api as i64,
            exception = %exception,
            "Drained a pending Java exception before resolving the MediaStore row"
        );
    }

    let clear = match next_step(copy, ClearOutcome::NotAttempted) {
        Some(PendingStep::Clear) => clear_pending_flag(env, resolver, row),
        // Incomplete copy: no visibility step at all, the table below sends
        // the row straight to delete.
        Some(PendingStep::Delete) | None => ClearOutcome::NotAttempted,
    };

    if matches!(next_step(copy, clear), Some(PendingStep::Delete)) {
        return delete_pending_row(env, resolver, row);
    }

    // `Clear` cannot come back here: `clear` is no longer `NotAttempted`.
    info!(
        display_name = %row.display_name,
        row_id = %row.id,
        api = row.api as i64,
        uri = %row.uri_string,
        "MediaStore row is visible: is_pending cleared"
    );
    PendingOutcome::Visible
}

/// `UPDATE row SET is_pending = 0` — the transition that makes the row appear
/// in the file manager. The affected-row count is checked: 0 is not evidence of
/// success, and the previous unchecked call here is how an invisible file
/// shipped without a word in any log.
#[cfg(target_os = "android")]
fn clear_pending_flag<'local>(
    env: &mut JNIEnv<'local>,
    resolver: &JObject<'_>,
    row: &PendingRow<'local>,
) -> ClearOutcome {
    let cv = match new_content_values(env) {
        Ok(cv) => cv,
        Err(e) => {
            warn!(
                display_name = %row.display_name,
                row_id = %row.id,
                api = row.api as i64,
                error = %e,
                "Could not build ContentValues for is_pending=0 — the row stays pending and will be deleted"
            );
            return ClearOutcome::Failed;
        }
    };
    if let Err(e) = put_int_column(env, &cv, COLUMN_IS_PENDING, 0) {
        warn!(
            display_name = %row.display_name,
            row_id = %row.id,
            api = row.api as i64,
            error = %e,
            "Could not set is_pending=0 in ContentValues — the row stays pending and will be deleted"
        );
        return ClearOutcome::Failed;
    }

    let result = env
        .call_method(
            resolver,
            "update",
            "(Landroid/net/Uri;Landroid/content/ContentValues;Ljava/lang/String;[Ljava/lang/String;)I",
            &[
                JValue::Object(&row.uri),
                JValue::Object(&cv),
                JValue::Object(&JObject::null()),
                JValue::Object(&JObject::null()),
            ],
        )
        .and_then(|value| value.i());

    match result {
        Ok(rows) if rows > 0 => ClearOutcome::Updated,
        Ok(rows) => {
            warn!(
                display_name = %row.display_name,
                row_id = %row.id,
                api = row.api as i64,
                rows_updated = rows as i64,
                uri = %row.uri_string,
                "Clearing is_pending matched no MediaStore row — the file cannot be made visible and the row will be deleted"
            );
            ClearOutcome::NoRows
        }
        Err(e) => {
            warn!(
                display_name = %row.display_name,
                row_id = %row.id,
                api = row.api as i64,
                error = %e,
                "Clearing is_pending failed — the file cannot be made visible and the row will be deleted"
            );
            ClearOutcome::Failed
        }
    }
}

/// `resolver.delete(uri, null, null)` — the documented way for an app to
/// remove a row it owns, and the only way an undeliverable pending row stops
/// reserving its display name.
#[cfg(target_os = "android")]
fn delete_pending_row<'local>(
    env: &mut JNIEnv<'local>,
    resolver: &JObject<'_>,
    row: &PendingRow<'local>,
) -> PendingOutcome {
    // The clear may itself have left an exception pending; deleting is a JNI
    // call like any other and has the same restriction.
    if let Some(exception) = take_pending_exception(env) {
        warn!(
            display_name = %row.display_name,
            row_id = %row.id,
            api = row.api as i64,
            exception = %exception,
            "Drained a pending Java exception before deleting the MediaStore row"
        );
    }

    let result = env
        .call_method(
            resolver,
            "delete",
            "(Landroid/net/Uri;Ljava/lang/String;[Ljava/lang/String;)I",
            &[
                JValue::Object(&row.uri),
                JValue::Object(&JObject::null()),
                JValue::Object(&JObject::null()),
            ],
        )
        .and_then(|value| value.i());

    match result {
        // 0 rows still satisfies the invariant: there is no pending row left.
        Ok(rows) if rows > 0 => {
            warn!(
                display_name = %row.display_name,
                row_id = %row.id,
                api = row.api as i64,
                rows_deleted = rows as i64,
                uri = %row.uri_string,
                "Deleted the unpublished MediaStore row so nothing stays invisible in Download/Auralis"
            );
            PendingOutcome::Removed
        }
        Ok(rows) => {
            warn!(
                display_name = %row.display_name,
                row_id = %row.id,
                api = row.api as i64,
                rows_deleted = rows as i64,
                uri = %row.uri_string,
                "The unpublished MediaStore row was already gone — nothing pending is left behind"
            );
            PendingOutcome::Removed
        }
        Err(e) => {
            warn!(
                display_name = %row.display_name,
                row_id = %row.id,
                api = row.api as i64,
                error = %e,
                uri = %row.uri_string,
                "Could NOT delete the unpublished MediaStore row — it is still pending and invisible; remove it with `content delete --uri <uri>`"
            );
            PendingOutcome::Unresolved
        }
    }
}

/// Print a pending Java exception to logcat and clear it, returning its
/// `toString()` for our own log line.
///
/// Two reasons this exists rather than a bare `exception_clear`:
///
/// * the message is what makes a `warn!` diagnosable — `jni::Error` renders a
///   thrown exception as the bare string `JavaException`, and
/// * the `update`/`delete` calls that follow a failure are only legal once the
///   exception is gone.
#[cfg(target_os = "android")]
fn take_pending_exception(env: &mut JNIEnv<'_>) -> Option<String> {
    match env.exception_check() {
        Ok(true) => {}
        // `Ok(false)`, or a failure of the check itself: nothing to drain.
        _ => return None,
    }
    // ExceptionDescribe writes the Java stack trace to logcat, the only place
    // a release build shows it.
    let _ = env.exception_describe();
    let throwable = env.exception_occurred().ok();
    let text = match throwable {
        Some(throwable) => {
            let called = env
                .call_method(&throwable, "toString", "()Ljava/lang/String;", &[])
                .ok();
            match called.and_then(|value| value.l().ok()) {
                Some(obj) if !obj.is_null() => {
                    let text = JString::from(obj);
                    match env.get_string(&text) {
                        Ok(java) => Some(java.into()),
                        Err(_) => None,
                    }
                }
                _ => None,
            }
        }
        None => None,
    };
    let _ = env.exception_clear();
    text
}

/// `Uri.toString()` plus the row id parsed from it. Log-only; both are needed
/// to act on a row by hand from an adb shell.
#[cfg(target_os = "android")]
fn describe_uri(env: &mut JNIEnv<'_>, uri: &JObject<'_>) -> (String, String) {
    let text = match env.call_method(uri, "toString", "()Ljava/lang/String;", &[]) {
        Ok(value) => match value.l() {
            Ok(obj) if !obj.is_null() => {
                let text = JString::from(obj);
                match env.get_string(&text) {
                    Ok(java) => java.into(),
                    Err(_) => String::new(),
                }
            }
            _ => String::new(),
        },
        Err(_) => String::new(),
    };
    let id = text
        .rsplit('/')
        .find(|segment| !segment.is_empty())
        .unwrap_or("unknown")
        .to_string();
    (text, id)
}

#[cfg(target_os = "android")]
fn new_content_values<'local>(env: &mut JNIEnv<'local>) -> Result<JObject<'local>, String> {
    env.new_object("android/content/ContentValues", "()V", &[])
        .map_err(|e| format!("new ContentValues: {e}"))
}

#[cfg(target_os = "android")]
fn put_string_column<'local>(
    env: &mut JNIEnv<'local>,
    cv: &JObject<'_>,
    key: &str,
    value: &str,
) -> Result<(), String> {
    let j_key = env
        .new_string(key)
        .map_err(|e| format!("new_string({key}): {e}"))?;
    let j_value = env
        .new_string(value)
        .map_err(|e| format!("new_string({key} value): {e}"))?;
    env.call_method(
        cv,
        "put",
        "(Ljava/lang/String;Ljava/lang/String;)V",
        &[
            JValue::Object(&j_key.into()),
            JValue::Object(&j_value.into()),
        ],
    )
    .map_err(|e| format!("ContentValues.put({key}): {e}"))?;
    Ok(())
}

#[cfg(target_os = "android")]
fn put_int_column<'local>(
    env: &mut JNIEnv<'local>,
    cv: &JObject<'_>,
    key: &str,
    value: i32,
) -> Result<(), String> {
    let j_key = env
        .new_string(key)
        .map_err(|e| format!("new_string({key}): {e}"))?;
    let j_value = env
        .new_object("java/lang/Integer", "(I)V", &[JValue::Int(value)])
        .map_err(|e| format!("new Integer({key}={value}): {e}"))?;
    env.call_method(
        cv,
        "put",
        "(Ljava/lang/String;Ljava/lang/Integer;)V",
        &[JValue::Object(&j_key.into()), JValue::Object(&j_value)],
    )
    .map_err(|e| format!("ContentValues.put({key}): {e}"))?;
    Ok(())
}

#[cfg(target_os = "android")]
fn open_output_stream<'local>(
    env: &mut JNIEnv<'local>,
    resolver: &JObject<'_>,
    uri: &JObject<'_>,
) -> Result<JObject<'local>, String> {
    let os = env
        .call_method(
            resolver,
            "openOutputStream",
            "(Landroid/net/Uri;)Ljava/io/OutputStream;",
            &[JValue::Object(uri)],
        )
        .and_then(|value| value.l())
        .map_err(|e| format!("openOutputStream: {e}"))?;
    if os.is_null() {
        // A null stream is how `FileNotFoundException` surfaces when the
        // provider declined the row; the exception itself is still pending and
        // is drained by the caller.
        return Err("openOutputStream returned null".into());
    }
    Ok(os)
}

/// Stream `src_path` into the row's `OutputStream`.
///
/// `close` is attempted on every path — including the failure paths — because
/// `MediaProvider` keeps the row's size pinned until its last connection is
/// closed, and because an abandoned stream leaks a provider connection. A
/// failed `close` counts as a failed copy: the row is only published when the
/// provider confirmed the whole write, and the internal copy (the one the
/// library plays) is untouched either way.
#[cfg(target_os = "android")]
fn copy_into_media_store(
    env: &mut JNIEnv<'_>,
    os: &JObject<'_>,
    src_path: &Path,
) -> Result<(), String> {
    let mut result: Result<(), String> = Ok(());
    let mut file = match std::fs::File::open(src_path) {
        Ok(file) => file,
        Err(e) => {
            close_media_stream(env, os);
            return Err(format!("open source for MediaStore copy: {e}"));
        }
    };
    let mut buf = vec![0u8; COPY_CHUNK_BYTES];
    loop {
        let n = match std::io::Read::read(&mut file, &mut buf) {
            Ok(n) => n,
            Err(e) => {
                result = Err(format!("read source for MediaStore copy: {e}"));
                break;
            }
        };
        if n == 0 {
            break;
        }
        let chunk = match env.byte_array_from_slice(&buf[..n]) {
            Ok(chunk) => chunk,
            Err(e) => {
                result = Err(format!("byte_array_from_slice: {e}"));
                break;
            }
        };
        let chunk = JObject::from(chunk);
        if let Err(e) = env.call_method(os, "write", "([B)V", &[JValue::Object(&chunk)]) {
            result = Err(format!("write to MediaStore row: {e}"));
            break;
        }
    }
    if result.is_ok() {
        if let Err(e) = env.call_method(os, "flush", "()V", &[]) {
            result = Err(format!("flush MediaStore row: {e}"));
        }
    }

    // A failed JNI call above left a Java exception pending, and JNI forbids
    // (almost) all calls while one is — so drain it before closing.
    if result.is_err() {
        if let Some(exception) = take_pending_exception(env) {
            warn!(
                src = %src_path.display(),
                exception = %exception,
                "Drained a pending Java exception before closing the MediaStore stream"
            );
        }
    }
    if let Err(e) = env.call_method(os, "close", "()V", &[]) {
        let _ = take_pending_exception(env);
        if result.is_ok() {
            result = Err(format!("close MediaStore row: {e}"));
        } else {
            warn!(
                src = %src_path.display(),
                error = %e,
                "Closing the MediaStore stream failed on top of an already failed copy"
            );
        }
    }
    result
}

/// Best-effort `OutputStream.close()`. Used on the paths where the copy never
/// started, so a stream is never left open without a log line.
#[cfg(target_os = "android")]
fn close_media_stream(env: &mut JNIEnv<'_>, os: &JObject<'_>) {
    if let Err(e) = env.call_method(os, "close", "()V", &[]) {
        let _ = take_pending_exception(env);
        warn!(error = %e, "Closing the MediaStore stream failed");
    }
}

#[cfg(target_os = "android")]
fn publish_q<'local>(
    env: &mut JNIEnv<'local>,
    resolver: &JObject<'_>,
    _ctx: &JObject<'_>,
    sdk: i32,
    src_path: &Path,
    display_name: &str,
    mime: &str,
) -> Result<String, String> {
    // --- insert: this is the point of no return, the row exists from here on --
    let cv = new_content_values(env)?;
    put_string_column(env, &cv, COLUMN_DISPLAY_NAME, display_name)?;
    put_string_column(env, &cv, COLUMN_MIME_TYPE, mime)?;
    put_string_column(env, &cv, COLUMN_RELATIVE_PATH, PUBLIC_RELATIVE_PATH)?;
    put_int_column(env, &cv, COLUMN_IS_PENDING, 1)?;

    let downloads_class = env
        .find_class("android/provider/MediaStore$Downloads")
        .map_err(|e| e.to_string())?;
    let uri = env
        .get_static_field(downloads_class, "EXTERNAL_CONTENT_URI", "Landroid/net/Uri;")
        .map_err(|e| e.to_string())?
        .l()
        .map_err(|e| e.to_string())?;

    let out_uri = match env
        .call_method(
            resolver,
            "insert",
            "(Landroid/net/Uri;Landroid/content/ContentValues;)Landroid/net/Uri;",
            &[JValue::Object(&uri), JValue::Object(&cv)],
        )
        .and_then(|value| value.l())
    {
        Ok(out_uri) => out_uri,
        Err(e) => {
            // No row was created, so there is nothing to resolve.
            let exception = take_pending_exception(env);
            return Err(format!(
                "MediaStore insert failed for '{}' (api {}): {}{}",
                display_name,
                sdk,
                e,
                exception_note(exception.as_deref())
            ));
        }
    };
    if out_uri.is_null() {
        let exception = take_pending_exception(env);
        warn!(
            display_name = %display_name,
            api = sdk as i64,
            exception = %exception.as_deref().unwrap_or("none"),
            "MediaStore insert returned a null uri — no row was created"
        );
        return Err(format!(
            "MediaStore insert returned a null uri for '{}' (api {}){}",
            display_name,
            sdk,
            exception_note(exception.as_deref())
        ));
    }

    let (uri_string, row_id) = describe_uri(env, &out_uri);
    let row = PendingRow {
        uri: out_uri,
        id: row_id,
        uri_string,
        display_name: display_name.to_string(),
        api: sdk,
    };
    // Belt and braces: nothing above should have left a Java exception
    // pending (`jni` reports a throw as `Err`), but every JNI call below this
    // line is only legal if that is true, and the alternative to a drain here
    // is an abort inside the VM.
    if let Some(exception) = take_pending_exception(env) {
        warn!(
            display_name = %row.display_name,
            row_id = %row.id,
            api = row.api as i64,
            exception = %exception,
            "Drained a pending Java exception after the MediaStore insert"
        );
    }
    info!(
        display_name = %row.display_name,
        row_id = %row.id,
        api = row.api as i64,
        uri = %row.uri_string,
        "Inserted MediaStore row with is_pending=1 (invisible until cleared)"
    );

    // --- open the row's stream --
    let os = match open_output_stream(env, resolver, &row.uri) {
        Ok(os) => os,
        Err(e) => {
            // Nothing was written, so the row must not be published: resolve it
            // (which deletes it) before bailing out.
            let outcome = resolve_pending_row(env, resolver, &row, CopyOutcome::Failed);
            return Err(format!(
                "{} [{}]",
                e,
                unresolved_note(&row, outcome, "no bytes were written")
            ));
        }
    };

    // --- copy the bytes --
    let copy_res = copy_into_media_store(env, &os, src_path);
    let (copy, note) = match &copy_res {
        Ok(()) => (CopyOutcome::Complete, "the byte copy completed".to_string()),
        Err(e) => (CopyOutcome::Failed, e.clone()),
    };
    if let Err(e) = &copy_res {
        warn!(
            display_name = %row.display_name,
            row_id = %row.id,
            api = row.api as i64,
            error = %e,
            "MediaStore byte copy failed — the row will not be published"
        );
    }

    // --- resolve the row: always, success included --
    match resolve_pending_row(env, resolver, &row, copy) {
        PendingOutcome::Visible => Ok(format!("{PUBLIC_ABSOLUTE_DIR}/{display_name}")),
        outcome => Err(unresolved_note(&row, outcome, &note)),
    }
}

/// One-line summary of what happened to a row that never became visible, used
/// as the error the caller (and its `warn!`) reports.
#[cfg(target_os = "android")]
fn unresolved_note(row: &PendingRow<'_>, outcome: PendingOutcome, reason: &str) -> String {
    match outcome {
        PendingOutcome::Visible => {
            format!("MediaStore row {} for '{}' is visible", row.id, row.display_name)
        }
        PendingOutcome::Removed => format!(
            "MediaStore copy of '{}' (api {}) was not published because {}; the pending row {} was removed",
            row.display_name, row.api, reason, row.id
        ),
        PendingOutcome::Unresolved => format!(
            "MediaStore copy of '{}' (api {}) was not published because {}; the pending row {} could NOT be deleted and is still invisible at {}",
            row.display_name, row.api, reason, row.id, row.uri_string
        ),
    }
}

#[cfg(target_os = "android")]
fn exception_note(exception: Option<&str>) -> String {
    match exception {
        Some(text) => format!(" [java exception: {text}]"),
        None => String::new(),
    }
}

/// API 26-28: no `is_pending` protocol exists here, so there is no pending row
/// to leak — the file lands on the filesystem and the media scanner indexes it
/// directly. The only discipline worth adding is to say *where* it went and on
/// which API, since a `scanFile` that throws leaves a file the file manager may
/// only pick up on the next boot scan.
#[cfg(target_os = "android")]
fn publish_legacy(
    env: &mut JNIEnv<'_>,
    resolver: &JObject<'_>,
    ctx: &JObject<'_>,
    sdk: i32,
    src_path: &Path,
    display_name: &str,
) -> Result<String, String> {
    // Environment.getExternalStoragePublicDirectory(DIRECTORY_DOWNLOADS) + "/Auralis"
    let env_class = env
        .find_class("android/os/Environment")
        .map_err(|e| e.to_string())?;
    let j_downloads = env
        .get_static_field(env_class, "DIRECTORY_DOWNLOADS", "Ljava/lang/String;")
        .map_err(|e| e.to_string())?
        .l()
        .map_err(|e| e.to_string())?;
    let pub_dir = env
        .call_static_method(
            "android/os/Environment",
            "getExternalStoragePublicDirectory",
            "(Ljava/lang/String;)Ljava/io/File;",
            &[JValue::Object(&j_downloads)],
        )
        .map_err(|e| e.to_string())?
        .l()
        .map_err(|e| e.to_string())?;
    let j_auralis = env.new_string("Auralis").map_err(|e| e.to_string())?;
    let auralis_dir = env
        .new_object(
            "java/io/File",
            "(Ljava/io/File;Ljava/lang/String;)V",
            &[JValue::Object(&pub_dir), JValue::Object(&j_auralis.into())],
        )
        .map_err(|e| e.to_string())?;
    env.call_method(&auralis_dir, "mkdirs", "()Z", &[])
        .map_err(|e| e.to_string())?;
    let j_display = env.new_string(display_name).map_err(|e| e.to_string())?;
    let dest = env
        .new_object(
            "java/io/File",
            "(Ljava/io/File;Ljava/lang/String;)V",
            &[
                JValue::Object(&auralis_dir),
                JValue::Object(&j_display.into()),
            ],
        )
        .map_err(|e| e.to_string())?;
    let dest_path_jstr = env
        .call_method(&dest, "getAbsolutePath", "()Ljava/lang/String;", &[])
        .map_err(|e| e.to_string())?
        .l()
        .map_err(|e| e.to_string())?;
    let dest_path: String = env
        .get_string(&JString::from(dest_path_jstr))
        .map(|s| s.into())
        .map_err(|e| e.to_string())?;

    // Copy via Rust fs copy (pos ix path same as Java File)
    std::fs::create_dir_all(
        std::path::Path::new(&dest_path)
            .parent()
            .unwrap_or(std::path::Path::new(PUBLIC_ABSOLUTE_DIR)),
    )
    .map_err(|e| format!("create {}: {}", PUBLIC_ABSOLUTE_DIR, e))?;
    std::fs::copy(src_path, &dest_path).map_err(|e| format!("copy to {dest_path}: {e}"))?;

    // MediaScannerConnection.scanFile(ctx, [path], null, null)
    let scanner_class = env
        .find_class("android/media/MediaScannerConnection")
        .map_err(|e| e.to_string())?;
    let j_path = env.new_string(&dest_path).map_err(|e| e.to_string())?;
    let arr = env
        .new_object_array(1, "java/lang/String", &j_path)
        .map_err(|e| e.to_string())?;
    let scan = env
        .call_static_method(
            scanner_class,
            "scanFile",
            "(Landroid/content/Context;[Ljava/lang/String;[Ljava/lang/String;Landroid/media/MediaScannerConnection$OnScanCompletedListener;)V",
            &[
                JValue::Object(ctx),
                JValue::Object(&arr.into()),
                JValue::Object(&JObject::null()),
                JValue::Object(&JObject::null()),
            ],
        )
        .map_err(|e| format!("MediaScannerConnection.scanFile: {e}"));
    if let Err(e) = &scan {
        // The file is already on disk, so this is cosmetic — but the file
        // manager may not list it until the next boot scan, and this is the
        // only hint that says why.
        let exception = take_pending_exception(env);
        warn!(
            display = %display_name,
            api = sdk as i64,
            path = %dest_path,
            error = %e,
            exception = %exception.as_deref().unwrap_or("none"),
            "MediaScannerConnection.scanFile failed; the file is on disk but may stay unindexed"
        );
        return Err(format!("{}{}", e, exception_note(exception.as_deref())));
    }
    let _ = resolver;
    info!(
        display = %display_name,
        api = sdk as i64,
        path = %dest_path,
        "Legacy publish: file copied and handed to the media scanner (no is_pending below API 29)"
    );
    Ok(dest_path)
}

#[cfg(target_os = "android")]
fn cached_vm() -> Option<&'static jni::JavaVM> {
    use std::sync::OnceLock;
    static VM: OnceLock<jni::JavaVM> = OnceLock::new();
    // Fast path: already cached successfully
    if let Some(vm) = VM.get() {
        return Some(vm);
    }
    // Check current JavaVM pointer — don't cache `None` permanently, as
    // the VM may not be seeded yet during early `JNI_OnLoad` race. Retry
    // on every call until we succeed, then cache the success.
    let ptr = crate::android_jni::INITIAL_VM.load(std::sync::atomic::Ordering::SeqCst);
    if ptr.is_null() {
        return None;
    }
    if let Ok(vm) = unsafe { jni::JavaVM::from_raw(ptr as *mut jni::sys::JavaVM) } {
        // `set` may fail if another thread raced to init — that's fine.
        let _ = VM.set(vm);
        return VM.get();
    }
    None
}

#[cfg(target_os = "android")]
fn with_attached_env<T>(
    f: impl FnOnce(&mut JNIEnv<'_>) -> Result<T, String>,
) -> Option<Result<T, String>> {
    let vm = cached_vm()?;
    let mut guard = vm.attach_current_thread().ok()?;
    let res = f(&mut guard);
    // Outer safety net: the publish paths drain a pending exception before
    // every follow-up JNI call, but a thread that hands back to Java with one
    // still pending makes the *next* JNI call on that thread undefined
    // behaviour, so nothing may leave here with one.
    if guard.exception_check().unwrap_or(false) {
        let _ = guard.exception_clear();
    }
    Some(res)
}

#[cfg(target_os = "android")]
fn service_context() -> Option<JObject<'static>> {
    let ctx = ndk_context::android_context().context();
    if ctx.is_null() {
        return None;
    }
    Some(unsafe { JObject::from_raw(ctx as jni::sys::jobject) })
}

/// Fallback: if `File::open(path)` fails for a `Download/Auralis` or `content://`
/// path, try to materialize a cache copy via `ContentResolver` so `rodio` can
/// still decode. Returns the cache file path or `None`.
#[cfg(target_os = "android")]
pub fn cached_copy_for_path(path: &str) -> Option<std::path::PathBuf> {
    // Only handle paths that look like shared storage or content URIs
    if !path.contains("Download") && !path.starts_with("content://") {
        return None;
    }
    let display = std::path::Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    if display.is_empty() {
        return None;
    }
    with_attached_env(|env| -> Result<std::path::PathBuf, String> {
        let ctx = service_context().ok_or("no context")?;
        let resolver = env
            .call_method(&ctx, "getContentResolver", "()Landroid/content/ContentResolver;", &[])
            .map_err(|e| e.to_string())?
            .l()
            .map_err(|e| e.to_string())?;
        // Resolve uri: if path already content:// parse, else query MediaStore by display_name
        let uri_obj = if path.starts_with("content://") {
            let uri_class = env.find_class("android/net/Uri").map_err(|e| e.to_string())?;
            let j_str = env.new_string(path).map_err(|e| e.to_string())?;
            env.call_static_method(
                uri_class,
                "parse",
                "(Ljava/lang/String;)Landroid/net/Uri;",
                &[JValue::Object(&j_str.into())],
            )
            .map_err(|e| e.to_string())?
            .l()
            .map_err(|e| e.to_string())?
        } else {
            // Query MediaStore.Downloads for _id where display_name = ?
            let downloads_class = env
                .find_class("android/provider/MediaStore$Downloads")
                .map_err(|e| e.to_string())?;
            let ext_uri = env
                .get_static_field(downloads_class, "EXTERNAL_CONTENT_URI", "Landroid/net/Uri;")
                .map_err(|e| e.to_string())?
                .l()
                .map_err(|e| e.to_string())?;
            let j_display = env.new_string(&display).map_err(|e| e.to_string())?;
            let j_sel = env.new_string("display_name=?").map_err(|e| e.to_string())?;
            let arr = env
                .new_object_array(1, "java/lang/String", &j_display)
                .map_err(|e| e.to_string())?;
            let proj: JObject<'_> = JObject::null();
            let cursor = env
                .call_method(
                    &resolver,
                    "query",
                    "(Landroid/net/Uri;[Ljava/lang/String;Ljava/lang/String;[Ljava/lang/String;Ljava/lang/String;)Landroid/database/Cursor;",
                    &[
                        JValue::Object(&ext_uri),
                        JValue::Object(&proj),
                        JValue::Object(&j_sel.into()),
                        JValue::Object(&arr.into()),
                        JValue::Object(&JObject::null()),
                    ],
                )
                .map_err(|e| e.to_string())?
                .l()
                .map_err(|e| e.to_string())?;
            if cursor.is_null() {
                return Err("query returned null cursor".into());
            }
            let has_row = env
                .call_method(&cursor, "moveToFirst", "()Z", &[])
                .map_err(|e| e.to_string())?
                .z()
                .map_err(|e| e.to_string())?;
            if !has_row {
                env.call_method(&cursor, "close", "()V", &[]).map_err(|e| e.to_string())?;
                return Err(format!("no MediaStore entry for {display}"));
            }
            let id_col = env
                .call_method(&cursor, "getColumnIndex", "(Ljava/lang/String;)I", &[JValue::Object(&env.new_string("_id").map_err(|e| e.to_string())?.into())])
                .map_err(|e| e.to_string())?
                .i()
                .map_err(|e| e.to_string())?;
            let id = env
                .call_method(&cursor, "getLong", "(I)J", &[JValue::Int(id_col)])
                .map_err(|e| e.to_string())?
                .j()
                .map_err(|e| e.to_string())?;
            env.call_method(&cursor, "close", "()V", &[]).map_err(|e| e.to_string())?;
            // Build content uri: content://media/external/downloads/<id>
                        let base = env
                .call_static_method(
                    "android/net/Uri",
                    "parse",
                    "(Ljava/lang/String;)Landroid/net/Uri;",
                    &[JValue::Object(&env.new_string("content://media/external/downloads").map_err(|e| e.to_string())?.into())],
                )
                .map_err(|e| e.to_string())?
                .l()
                .map_err(|e| e.to_string())?;
            let builder_obj = env.call_method(&base, "buildUpon", "()Landroid/net/Uri$Builder;", &[]).map_err(|e| e.to_string())?.l().map_err(|e| e.to_string())?;
            env.call_method(&builder_obj, "appendPath", "(Ljava/lang/String;)Landroid/net/Uri$Builder;", &[JValue::Object(&JObject::from(env.new_string(id.to_string()).map_err(|e| e.to_string())?))]).map_err(|e| e.to_string())?;
            let uri_result = env.call_method(&builder_obj, "build", "()Landroid/net/Uri;", &[]).map_err(|e| e.to_string())?.l().map_err(|e| e.to_string())?;
            uri_result
        };
        if uri_obj.is_null() {
            return Err("resolved uri is null".into());
        }
        // Open InputStream and copy to cache file
        let is = env
            .call_method(
                &resolver,
                "openInputStream",
                "(Landroid/net/Uri;)Ljava/io/InputStream;",
                &[JValue::Object(&uri_obj)],
            )
            .map_err(|e| e.to_string())?
            .l()
            .map_err(|e| e.to_string())?;
        if is.is_null() {
            return Err("openInputStream returned null".into());
        }
        // Get cache dir: ctx.getCacheDir()
        let cache_file_obj = env
            .call_method(&ctx, "getCacheDir", "()Ljava/io/File;", &[])
            .map_err(|e| e.to_string())?
            .l()
            .map_err(|e| e.to_string())?;
        let j_cache_name = env.new_string("auralis_play_cache").map_err(|e| e.to_string())?;
        let cache_dir_obj = env
            .new_object(
                "java/io/File",
                "(Ljava/io/File;Ljava/lang/String;)V",
                &[JValue::Object(&cache_file_obj), JValue::Object(&j_cache_name.into())],
            )
            .map_err(|e| e.to_string())?;
        env.call_method(&cache_dir_obj, "mkdirs", "()Z", &[]).map_err(|e| e.to_string())?;
        let j_display2 = env.new_string(&display).map_err(|e| e.to_string())?;
        let cache_file = env
            .new_object(
                "java/io/File",
                "(Ljava/io/File;Ljava/lang/String;)V",
                &[JValue::Object(&cache_dir_obj), JValue::Object(&j_display2.into())],
            )
            .map_err(|e| e.to_string())?;
        let cache_path_j = env
            .call_method(&cache_file, "getAbsolutePath", "()Ljava/lang/String;", &[])
            .map_err(|e| e.to_string())?
            .l()
            .map_err(|e| e.to_string())?;
        let cache_path: String = env
            .get_string(&JString::from(cache_path_j))
            .map(|s| s.into())
            .map_err(|e| e.to_string())?;
        // FileOutputStream
        let fos = env
            .new_object("java/io/FileOutputStream", "(Ljava/io/File;)V", &[JValue::Object(&cache_file)])
            .map_err(|e| e.to_string())?;
        let _buf_class = env.find_class("java/io/InputStream").map_err(|e| e.to_string())?;
        // 64KB buffer
        let j_buf = env.new_byte_array(64 * 1024).map_err(|e| e.to_string())?;
        let j_buf_raw = j_buf.as_raw();
        loop {
            let j_buf_obj = unsafe { JObject::from_raw(j_buf_raw) };
            let n = env
                .call_method(&is, "read", "([B)I", &[JValue::Object(&j_buf_obj)])
                .map_err(|e| e.to_string())?
                .i()
                .map_err(|e| e.to_string())?;
            if n <= 0 {
                break;
            }
            let j_buf_obj2 = unsafe { JObject::from_raw(j_buf_raw) };
            env.call_method(
                &fos,
                "write",
                "([BII)V",
                &[JValue::Object(&j_buf_obj2), JValue::Int(0), JValue::Int(n)],
            )
            .map_err(|e| e.to_string())?;
        }
        env.call_method(&fos, "close", "()V", &[]).map_err(|e| e.to_string())?;
        env.call_method(&is, "close", "()V", &[]).map_err(|e| e.to_string())?;
        Ok(std::path::PathBuf::from(cache_path))
    })
    .and_then(|r| r.ok())
}

/// The JNI side of publishing cannot be exercised off-device, but the decision
/// that makes DL-07 impossible to regress — "clear, or delete?" — is pure and
/// lives in [`next_step`]. These tests are the executable form of the
/// invariant; [`resolve_pending_row`] is the only caller and is a straight-line
/// transcription of the table.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_copy_clears_then_stops() {
        // The happy path stops right after a successful clear: no delete, so a
        // published row is never removed.
        assert_eq!(
            next_step(CopyOutcome::Complete, ClearOutcome::NotAttempted),
            Some(PendingStep::Clear)
        );
        assert_eq!(
            next_step(CopyOutcome::Complete, ClearOutcome::Updated),
            None
        );
    }

    #[test]
    fn incomplete_copy_is_never_cleared() {
        // A truncated row must not be published, whatever the clear said.
        for clear in [
            ClearOutcome::NotAttempted,
            ClearOutcome::Updated,
            ClearOutcome::NoRows,
            ClearOutcome::Failed,
        ] {
            assert_eq!(
                next_step(CopyOutcome::Failed, clear),
                Some(PendingStep::Delete),
                "a failed copy must delete, never clear (clear = {clear:?})"
            );
        }
    }

    #[test]
    fn unclearable_row_is_deleted_not_left_pending() {
        // DL-07: a clear that throws, or that matches 0 rows, leaves the file
        // invisible either way. Deleting is the only outcome that satisfies
        // "always made visible or removed".
        assert_eq!(
            next_step(CopyOutcome::Complete, ClearOutcome::Failed),
            Some(PendingStep::Delete)
        );
        assert_eq!(
            next_step(CopyOutcome::Complete, ClearOutcome::NoRows),
            Some(PendingStep::Delete)
        );
    }

    #[test]
    fn every_copy_and_clear_combination_terminates() {
        // The invariant: `Clear` comes back at most once and only for a
        // complete copy whose clear has not been attempted — so the caller's
        // two-step sequence cannot loop, cannot retry a clear, and cannot end
        // with a pending row nobody resolved.
        for copy in [CopyOutcome::Complete, CopyOutcome::Failed] {
            for clear in [
                ClearOutcome::NotAttempted,
                ClearOutcome::Updated,
                ClearOutcome::NoRows,
                ClearOutcome::Failed,
            ] {
                let step = next_step(copy, clear);
                if step == Some(PendingStep::Clear) {
                    assert_eq!(copy, CopyOutcome::Complete, "{copy:?}/{clear:?}");
                    assert_eq!(clear, ClearOutcome::NotAttempted, "{copy:?}/{clear:?}");
                }
                // The only way out without a step is a published row.
                if step == None {
                    assert_eq!(copy, CopyOutcome::Complete, "{copy:?}/{clear:?}");
                    assert_eq!(clear, ClearOutcome::Updated, "{copy:?}/{clear:?}");
                }
            }
        }
    }

    #[test]
    fn mime_for_ext_covers_the_formats_we_publish() {
        assert_eq!(mime_for_ext("mp3"), "audio/mpeg");
        assert_eq!(mime_for_ext("M4A"), "audio/mp4");
        assert_eq!(mime_for_ext("opus"), "audio/opus");
        assert_eq!(mime_for_ext("unknown"), "audio/mpeg");
    }
}
