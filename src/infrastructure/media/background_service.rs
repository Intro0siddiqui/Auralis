//! Background playback service bridge (Android only).
//!
//! Keeps the Rust audio engine alive while the app is backgrounded by
//! driving a Kotlin foreground `MediaPlaybackService` (notification +
//! MediaSession) over JNI. Notification / lockscreen media buttons are
//! routed back into Rust through the exported JNI entry point
//! `Java_com_auralis_v2_NativeBridge_command`, which dispatches to the same
//! playback operations as the UI — so the frontend stays in sync through the
//! regular `playback:*` events.
//!
//! On non-Android targets every public function is a no-op.

use std::sync::Arc;
#[cfg(target_os = "android")]
use std::sync::OnceLock;
use std::time::Duration;
use tauri::AppHandle;
use tracing::debug;
#[cfg(target_os = "android")]
use tracing::warn;

use super::AudioPlayer;
use crate::domain::models::Track;

#[cfg(target_os = "android")]
use jni::{
    objects::{GlobalRef, JObject, JString, JValue},
    sys::jstring,
    JNIEnv, JavaVM,
};

/// Kotlin `MediaPlaybackService` (kept in the foreground via `startForeground`).
#[cfg(target_os = "android")]
const SERVICE_CLASS: &str = "com/auralis/v2/MediaPlaybackService";

/// Kotlin `MainActivity` (runtime permission requests).
#[cfg(target_os = "android")]
const ACTIVITY_CLASS: &str = "com/auralis/v2/MainActivity";

/// Cached global refs for app classes.
///
/// `JNIEnv::find_class` resolves through the system class loader when called
/// from an attached native thread (tokio workers), which cannot see app
/// classes — every lookup therefore goes through [`app_class`], which caches
/// a global ref and falls back to the app `ClassLoader.loadClass`.
#[cfg(target_os = "android")]
static SERVICE_CLASS_REF: OnceLock<GlobalRef> = OnceLock::new();
#[cfg(target_os = "android")]
static ACTIVITY_CLASS_REF: OnceLock<GlobalRef> = OnceLock::new();

/// The live `JavaVM`, reconstructed from the pointer captured in `JNI_OnLoad`.
#[cfg(target_os = "android")]
static VM: OnceLock<Option<JavaVM>> = OnceLock::new();

/// The audio player and app handle used by the JNI command dispatcher.
#[cfg(target_os = "android")]
static PLAYER: OnceLock<Arc<AudioPlayer>> = OnceLock::new();
#[cfg(target_os = "android")]
static APP: OnceLock<AppHandle> = OnceLock::new();

/// Attach the bridge to the live player + app handle (called once from setup).
#[cfg_attr(not(target_os = "android"), allow(unused_variables))]
pub fn attach(player: Arc<AudioPlayer>, app: AppHandle) {
    #[cfg(target_os = "android")]
    {
        let _ = PLAYER.set(player);
        let _ = APP.set(app);
        cached_vm();
        debug!("Background service bridge attached");
    }
}

/// Push the current playback state to the background service.
///
/// No current track ⇒ stops the service (queue exhausted / app stopped).
/// Otherwise refreshes the notification + MediaSession with the playing or
/// paused state.
pub async fn push_now_playing(player: &AudioPlayer) {
    let Some(track) = player.get_current_track().await else {
        stop_service();
        return;
    };
    let position = player.current_position().await;
    if player.is_playing().await {
        notify_playing(&track, position);
    } else {
        notify_paused(&track, position);
    }
}

/// Refresh the notification as "playing".
#[cfg_attr(not(target_os = "android"), allow(unused_variables))]
pub fn notify_playing(track: &Track, position: Duration) {
    #[cfg(target_os = "android")]
    notify(track, position, true);
}

/// Refresh the notification as "paused".
#[cfg_attr(not(target_os = "android"), allow(unused_variables))]
pub fn notify_paused(track: &Track, position: Duration) {
    #[cfg(target_os = "android")]
    notify(track, position, false);
}

/// Stop the foreground service (queue exhausted / explicit stop).
pub fn stop_service() {
    #[cfg(target_os = "android")]
    {
        let Some(ctx) = service_context() else { return };
        with_attached_env("svc-stop", |env| {
            let class = app_class(env, &SERVICE_CLASS_REF, SERVICE_CLASS)?;
            env.call_static_method(
                class,
                "stop",
                "(Landroid/content/Context;)V",
                &[JValue::Object(&ctx)],
            )?;
            Ok(())
        });
    }
    debug!("Background service stopped");
}

/// Request runtime notification permissions on Android (Android 13+ / API 33+).
pub fn request_notification_permission() {
    #[cfg(target_os = "android")]
    {
        let Some(ctx) = service_context() else { return };
        with_attached_env("perm-request", |env| {
            let class = app_class(env, &ACTIVITY_CLASS_REF, ACTIVITY_CLASS)?;
            // Kotlin `requestRuntimePermissions(context: Any?)` erases to
            // `(Ljava/lang/Object;)V` — NOT `(Landroid/content/Context;)V`.
            // The Context descriptor throws NoSuchMethodError on every call.
            env.call_static_method(
                class,
                "requestRuntimePermissions",
                "(Ljava/lang/Object;)V",
                &[JValue::Object(&ctx)],
            )?;
            Ok(())
        });
    }
}

#[cfg(target_os = "android")]
fn notify(track: &Track, position: Duration, is_playing: bool) {
    let Some(ctx) = service_context() else { return };
    with_attached_env("svc-start", |env| {
        let class = app_class(env, &SERVICE_CLASS_REF, SERVICE_CLASS)?;
        let title = env.new_string(track.title.as_str())?;
        let artist = env.new_string(track.artist.clone().unwrap_or_default().as_str())?;
        let art_path = env.new_string(track.album_art_path.clone().unwrap_or_default().as_str())?;
        env.call_static_method(
            class,
            "start",
            "(Landroid/content/Context;Ljava/lang/String;Ljava/lang/String;IIZLjava/lang/String;)V",
            &[
                JValue::Object(&ctx),
                JValue::Object(&title),
                JValue::Object(&artist),
                JValue::Int(track.duration_secs as i32),
                JValue::Int(position.as_secs().min(i32::MAX as u64) as i32),
                JValue::Bool(is_playing as u8),
                JValue::Object(&art_path),
            ],
        )?;
        Ok(())
    });
    debug!(track_id = %track.id, ?position, is_playing, "Background service notified");
}

/// Resolve an app class to a cached global ref, from any thread.
///
/// `find_class` on an attached native thread (tokio workers) resolves through
/// the system class loader, which cannot see app classes
/// (`ClassNotFoundException`), so the fast path is tried first and the app
/// `ClassLoader.loadClass` fallback second. Either way the result is cached,
/// so later calls never touch the class loader again.
#[cfg(target_os = "android")]
fn app_class(
    env: &mut JNIEnv<'_>,
    slot: &'static OnceLock<GlobalRef>,
    slashed: &str,
) -> jni::errors::Result<&'static GlobalRef> {
    if let Some(cached) = slot.get() {
        return Ok(cached);
    }
    let global = match env
        .find_class(slashed)
        .and_then(|class| env.new_global_ref(class))
    {
        Ok(global) => global,
        Err(_) => {
            // A pending ClassNotFoundException would poison every later JNI
            // call on this env — clear it before trying the fallback.
            let _ = env.exception_clear();
            class_via_app_loader(env, slashed)?
        }
    };
    Ok(slot.get_or_init(move || global))
}

/// Resolve `slashed` (e.g. `com/auralis/v2/MediaPlaybackService`) through the
/// live context's `ClassLoader`, which works from any attached thread.
#[cfg(target_os = "android")]
fn class_via_app_loader(env: &mut JNIEnv<'_>, slashed: &str) -> jni::errors::Result<GlobalRef> {
    let ctx = service_context().ok_or(jni::errors::Error::NullPtr("service context"))?;
    let loader = env
        .call_method(ctx, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])?
        .l()?;
    let dotted = env.new_string(slashed.replace('/', "."))?;
    let class = env
        .call_method(
            loader,
            "loadClass",
            "(Ljava/lang/String;)Ljava/lang/Class;",
            &[JValue::Object(&dotted)],
        )?
        .l()?;
    env.new_global_ref(class)
}

/// Best-effort pre-cache of app class refs while `JNI_OnLoad` still runs under
/// the app class loader. Failures are fine — the classloader fallback in
/// [`app_class`] covers them — but a warm cache avoids the fallback entirely.
#[cfg(target_os = "android")]
pub(crate) fn precache_classes(vm: &jni::JavaVM) {
    let Ok(mut env) = vm.attach_current_thread() else {
        return;
    };
    for (slot, name) in [
        (&SERVICE_CLASS_REF, SERVICE_CLASS),
        (&ACTIVITY_CLASS_REF, ACTIVITY_CLASS),
    ] {
        if slot.get().is_none() {
            match env.find_class(name).and_then(|c| env.new_global_ref(c)) {
                Ok(global) => {
                    let _ = slot.set(global);
                }
                Err(_) => {
                    let _ = env.exception_clear();
                }
            }
        }
    }
}

/// Best-effort `android.util.Log.e`.
///
/// Framework classes resolve from any thread, so bridge failures become
/// visible in logcat even though the Rust `tracing` subscriber is absent on
/// release Android builds (where `warn!` goes nowhere).
#[cfg(target_os = "android")]
fn logcat_error(tag: &str, msg: &str) {
    let Some(vm) = cached_vm() else { return };
    let Ok(mut guard) = vm.attach_current_thread() else {
        return;
    };
    let env: &mut JNIEnv<'_> = &mut guard;
    if logcat_emit(env, tag, msg).is_err() {
        let _ = env.exception_clear();
    }
}

/// Emit one `android.util.Log.e` line. Called only by [`logcat_error`].
#[cfg(target_os = "android")]
fn logcat_emit(env: &mut JNIEnv<'_>, tag: &str, msg: &str) -> jni::errors::Result<()> {
    let class = env.find_class("android/util/Log")?;
    let tag = env.new_string(tag)?;
    let msg = env.new_string(msg)?;
    env.call_static_method(
        class,
        "e",
        "(Ljava/lang/String;Ljava/lang/String;)I",
        &[JValue::Object(&tag), JValue::Object(&msg)],
    )?;
    Ok(())
}

/// Reconstruct the cached `JavaVM` from the pointer captured in `JNI_OnLoad`.
#[cfg(target_os = "android")]
fn cached_vm() -> Option<&'static JavaVM> {
    VM.get_or_init(|| {
        let ptr = crate::android_jni::INITIAL_VM.load(std::sync::atomic::Ordering::SeqCst);
        if ptr.is_null() {
            warn!("Android JavaVM not captured in JNI_OnLoad; background service bridge disabled");
            return None;
        }
        // SAFETY: `ptr` was captured from the live JavaVM in `JNI_OnLoad` (lib.rs).
        unsafe { JavaVM::from_raw(ptr as *mut jni::sys::JavaVM) }.ok()
    })
    .as_ref()
}

/// Run `f` with a JNI environment attached to the current thread.
///
/// Attaching an already-attached thread is a no-op per the JNI spec, so this
/// is safe from both tokio worker threads and the main thread.
#[cfg(target_os = "android")]
fn with_attached_env<T>(
    op: &'static str,
    f: impl FnOnce(&mut JNIEnv<'_>) -> jni::errors::Result<T>,
) -> Option<T> {
    let vm = cached_vm()?;
    let mut guard = vm.attach_current_thread().ok()?;
    match f(&mut guard) {
        Ok(v) => Some(v),
        Err(e) => {
            // Dump the full Java stack trace to logcat (tag `System.err`)
            // BEFORE clearing — the one-line `e` alone cannot identify throws
            // from inside Kotlin (e.g. which statement in `start()` failed).
            if guard.exception_check().unwrap_or(false) {
                let _ = guard.exception_describe();
                let _ = guard.exception_clear();
            }
            // `tracing` has no subscriber on release Android builds, so mirror
            // the failure into logcat where it can actually be diagnosed.
            logcat_error("AuralisBridge", &format!("JNI {op} failed: {e}"));
            warn!(op, error = %e, "JNI call failed");
            None
        }
    }
}

/// The global `Context` (the Android `Activity`) registered by `JNI_OnLoad`.
#[cfg(target_os = "android")]
fn service_context() -> Option<JObject<'static>> {
    let ctx = ndk_context::android_context().context();
    if ctx.is_null() {
        warn!("Android context unavailable; background service bridge disabled");
        logcat_error(
            "AuralisBridge",
            "Android context unavailable; background service bridge disabled",
        );
        return None;
    }
    // SAFETY: `ctx` is the live global JNI reference seeded by JNI_OnLoad.
    Some(unsafe { JObject::from_raw(ctx as jni::sys::jobject) })
}

/// JNI entry point called by `com.auralis.v2.NativeBridge` when a notification
/// or lockscreen media control is pressed. Runs on the Android main thread and
/// must return immediately — the actual dispatch happens on the tokio runtime.
#[cfg(target_os = "android")]
#[no_mangle]
#[allow(non_snake_case)]
pub extern "system" fn Java_com_auralis_v2_NativeBridge_command(
    mut env: JNIEnv<'_>,
    _obj: JObject<'_>,
    cmd: JString<'_>,
) -> jstring {
    let command: String = env.get_string(&cmd).map(|s| s.into()).unwrap_or_default();
    dispatch(&command);
    match env.new_string("ok") {
        Ok(s) => s.into_raw(),
        Err(e) => {
            warn!(error = %e, "Failed to build JNI reply");
            std::ptr::null_mut()
        }
    }
}

/// Parse a media command and act on the player, then refresh the frontend and
/// the notification state so every surface stays in sync.
#[cfg(target_os = "android")]
fn dispatch(command: &str) {
    let Some(player) = PLAYER.get().cloned() else {
        return;
    };
    let Some(app) = APP.get().cloned() else {
        return;
    };
    debug!(command, "Notification media command received");
    let command = command.to_owned();
    tauri::async_runtime::spawn(async move {
        match command.as_str() {
            "play" => {
                let _ = player.resume().await;
            }
            "pause" => {
                let _ = player.pause().await;
            }
            "next" => {
                let _ = player.next().await;
                crate::commands::playback::emit_track_changed(&app, &player).await;
            }
            "previous" => {
                let _ = player.previous().await;
                crate::commands::playback::emit_track_changed(&app, &player).await;
            }
            c if c.starts_with("seek:") => {
                if let Ok(secs) = c[5..].parse::<u64>() {
                    let _ = player.seek(Duration::from_secs(secs)).await;
                }
            }
            _ => {
                warn!(command, "Unknown notification command");
            }
        }
        crate::commands::playback::emit_state_changed(&app, &player).await;
        push_now_playing(&player).await;
    });
}
