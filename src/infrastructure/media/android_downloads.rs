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

use std::path::Path;
#[cfg(target_os = "android")]
use tracing::{info, warn};

#[cfg(target_os = "android")]
use jni::{
    objects::{JObject, JString, JValue},
    JNIEnv,
};

#[cfg(target_os = "android")]
#[allow(dead_code)]
const SERVICE_CLASS: &str = "com/auralis/v2/MediaPlaybackService";

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
                warn!(src = %src_path.display(), error = %e, "MediaStore publish failed, keeping internal path");
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
            publish_q(env, &resolver, &ctx, src_path, display_name, mime)
        } else {
            publish_legacy(env, &resolver, &ctx, src_path, display_name)
        }
    })
    .ok_or_else(|| "JNI env unavailable".to_string())?
}

#[cfg(target_os = "android")]
fn publish_q(
    env: &mut JNIEnv<'_>,
    resolver: &JObject<'_>,
    _ctx: &JObject<'_>,
    src_path: &Path,
    display_name: &str,
    mime: &str,
) -> Result<String, String> {
    // Build ContentValues
    let cv_class = env
        .find_class("android/content/ContentValues")
        .map_err(|e| e.to_string())?;
    let cv = env
        .new_object(cv_class, "()V", &[])
        .map_err(|e| e.to_string())?;
    let j_display = env.new_string(display_name).map_err(|e| e.to_string())?;
    let j_mime = env.new_string(mime).map_err(|e| e.to_string())?;
    let j_rel = env
        .new_string("Download/Auralis")
        .map_err(|e| e.to_string())?;
    // put(String, String)
    env.call_method(
        &cv,
        "put",
        "(Ljava/lang/String;Ljava/lang/String;)V",
        &[
            JValue::Object(
                &env.new_string("display_name")
                    .map_err(|e| e.to_string())?
                    .into(),
            ),
            JValue::Object(&j_display.into()),
        ],
    )
    .map_err(|e| e.to_string())?;
    env.call_method(
        &cv,
        "put",
        "(Ljava/lang/String;Ljava/lang/String;)V",
        &[
            JValue::Object(
                &env.new_string("mime_type")
                    .map_err(|e| e.to_string())?
                    .into(),
            ),
            JValue::Object(&j_mime.into()),
        ],
    )
    .map_err(|e| e.to_string())?;
    env.call_method(
        &cv,
        "put",
        "(Ljava/lang/String;Ljava/lang/String;)V",
        &[
            JValue::Object(
                &env.new_string("relative_path")
                    .map_err(|e| e.to_string())?
                    .into(),
            ),
            JValue::Object(&j_rel.into()),
        ],
    )
    .map_err(|e| e.to_string())?;
    // put(String, Integer) for is_pending = 1
    let j_pending_key = env.new_string("is_pending").map_err(|e| e.to_string())?;
    let integer_class = env
        .find_class("java/lang/Integer")
        .map_err(|e| e.to_string())?;
    let j_one = env
        .new_object(integer_class, "(I)V", &[JValue::Int(1)])
        .map_err(|e| e.to_string())?;
    env.call_method(
        &cv,
        "put",
        "(Ljava/lang/String;Ljava/lang/Integer;)V",
        &[
            JValue::Object(&j_pending_key.into()),
            JValue::Object(&j_one),
        ],
    )
    .map_err(|e| e.to_string())?;

    // MediaStore.Downloads.EXTERNAL_CONTENT_URI
    let downloads_class = env
        .find_class("android/provider/MediaStore$Downloads")
        .map_err(|e| e.to_string())?;
    let uri = env
        .get_static_field(downloads_class, "EXTERNAL_CONTENT_URI", "Landroid/net/Uri;")
        .map_err(|e| e.to_string())?
        .l()
        .map_err(|e| e.to_string())?;

    // resolver.insert(uri, cv) -> Uri
    let out_uri = env
        .call_method(
            resolver,
            "insert",
            "(Landroid/net/Uri;Landroid/content/ContentValues;)Landroid/net/Uri;",
            &[JValue::Object(&uri), JValue::Object(&cv)],
        )
        .map_err(|e| format!("insert: {e}"))?
        .l()
        .map_err(|e| e.to_string())?;
    if out_uri.is_null() {
        return Err("insert returned null uri".into());
    }

    // resolver.openOutputStream(uri) -> OutputStream
    let os = env
        .call_method(
            resolver,
            "openOutputStream",
            "(Landroid/net/Uri;)Ljava/io/OutputStream;",
            &[JValue::Object(&out_uri)],
        )
        .map_err(|e| format!("openOutputStream: {e}"))?
        .l()
        .map_err(|e| e.to_string())?;
    if os.is_null() {
        return Err("openOutputStream returned null".into());
    }

    // Stream src file bytes into OutputStream in 64KB chunks via JNI
    {
        let mut file = std::fs::File::open(src_path).map_err(|e| e.to_string())?;
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let n = std::io::Read::read(&mut file, &mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            let jarr = env
                .byte_array_from_slice(&buf[..n])
                .map_err(|e| e.to_string())?;
            let jarr_obj = JObject::from(jarr);
            env.call_method(&os, "write", "([B)V", &[JValue::Object(&jarr_obj)])
                .map_err(|e| format!("write: {e}"))?;
        }
        env.call_method(&os, "flush", "()V", &[])
            .map_err(|e| e.to_string())?;
        env.call_method(&os, "close", "()V", &[])
            .map_err(|e| e.to_string())?;
    }

    // Clear IS_PENDING = 0
    let cv_class2 = env
        .find_class("android/content/ContentValues")
        .map_err(|e| e.to_string())?;
    let cv2 = env
        .new_object(cv_class2, "()V", &[])
        .map_err(|e| e.to_string())?;
    let j_pending_key2 = env.new_string("is_pending").map_err(|e| e.to_string())?;
    let integer_class2 = env
        .find_class("java/lang/Integer")
        .map_err(|e| e.to_string())?;
    let j_zero = env
        .new_object(integer_class2, "(I)V", &[JValue::Int(0)])
        .map_err(|e| e.to_string())?;
    env.call_method(
        &cv2,
        "put",
        "(Ljava/lang/String;Ljava/lang/Integer;)V",
        &[
            JValue::Object(&j_pending_key2.into()),
            JValue::Object(&j_zero),
        ],
    )
    .map_err(|e| e.to_string())?;
    env.call_method(
        resolver,
        "update",
        "(Landroid/net/Uri;Landroid/content/ContentValues;Ljava/lang/String;[Ljava/lang/String;)I",
        &[
            JValue::Object(&out_uri),
            JValue::Object(&cv2),
            JValue::Object(&JObject::null()),
            JValue::Object(&JObject::null()),
        ],
    )
    .map_err(|e| e.to_string())?;

    // Public path: resolver queries _data or build via Environment + Download/Auralis/display_name
    // Simpler: return /storage/emulated/0/Download/Auralis/<display>
    Ok(format!(
        "/storage/emulated/0/Download/Auralis/{}",
        display_name
    ))
}

#[cfg(target_os = "android")]
fn publish_legacy(
    env: &mut JNIEnv<'_>,
    resolver: &JObject<'_>,
    ctx: &JObject<'_>,
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
            .unwrap_or(std::path::Path::new("/storage/emulated/0/Download")),
    )
    .map_err(|e| e.to_string())?;
    std::fs::copy(src_path, &dest_path).map_err(|e| e.to_string())?;

    // MediaScannerConnection.scanFile(ctx, [path], null, null)
    let scanner_class = env
        .find_class("android/media/MediaScannerConnection")
        .map_err(|e| e.to_string())?;
    let j_path = env.new_string(&dest_path).map_err(|e| e.to_string())?;
    let arr = env
        .new_object_array(1, "java/lang/String", &j_path)
        .map_err(|e| e.to_string())?;
    env.call_static_method(
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
    .map_err(|e| e.to_string())?;
    let _ = resolver;
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
    if guard.exception_check().unwrap_or(false) {
        let _ = guard.exception_clear();
    }
    match res {
        Ok(v) => Some(Ok(v)),
        Err(e) => {
            if guard.exception_check().unwrap_or(false) {
                let _ = guard.exception_clear();
            }
            Some(Err(e))
        }
    }
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
