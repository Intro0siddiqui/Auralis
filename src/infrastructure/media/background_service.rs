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
    objects::{JObject, JString, JValue},
    sys::jstring,
    JNIEnv, JavaVM,
};

/// Kotlin `MediaPlaybackService` (kept in the foreground via `startForeground`).
#[cfg(target_os = "android")]
const SERVICE_CLASS: &str = "com/auralis/v2/MediaPlaybackService";

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
        with_attached_env(|env| {
            let class = env.find_class(SERVICE_CLASS)?;
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

#[cfg(target_os = "android")]
fn notify(track: &Track, position: Duration, is_playing: bool) {
    let Some(ctx) = service_context() else { return };
    with_attached_env(|env| {
        let class = env.find_class(SERVICE_CLASS)?;
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
fn with_attached_env<T>(f: impl FnOnce(&mut JNIEnv<'_>) -> jni::errors::Result<T>) -> Option<T> {
    let vm = cached_vm()?;
    let mut guard = vm.attach_current_thread().ok()?;
    match f(&mut guard) {
        Ok(v) => Some(v),
        Err(e) => {
            // Clear any pending JNI exception (e.g. ForegroundServiceStartNotAllowedException
            // when startForegroundService is throttled in the background) so future JNI
            // calls are not poisoned.
            if guard.exception_check().unwrap_or(false) {
                let _ = guard.exception_clear();
            }
            warn!(error = %e, "JNI call failed");
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
