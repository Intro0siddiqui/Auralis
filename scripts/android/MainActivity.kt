package com.auralis.v2

import android.Manifest
import android.app.Activity
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.webkit.WebView
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat
import java.lang.ref.WeakReference

class MainActivity : TauriActivity() {

    companion object {
        const val PERMISSION_REQUEST_CODE = 1001
        private var currentActivityRef: WeakReference<MainActivity>? = null
        // Set when Rust/background code asked for permissions while no live
        // Activity existed (Application context can't request). Retried on
        // next onResume when the Activity is foregrounded again.
        @Volatile
        private var pendingPermissionRequest = false

        @JvmStatic
        @JvmOverloads
        fun requestRuntimePermissions(context: Any? = null) {
            val act = (context as? MainActivity) ?: currentActivityRef?.get()
            if (act == null) {
                pendingPermissionRequest = true
                return
            }
            act.runOnUiThread {
                act.checkAndRequestPermissions()
            }
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        currentActivityRef = WeakReference(this)
        WebView.setWebContentsDebuggingEnabled(true)

        // Ensure playback notification channel exists early on app startup
        MediaPlaybackService.createNotificationChannel(this)
        
        // Post permission check after window and webview layout attach
        Handler(Looper.getMainLooper()).postDelayed({
            if (!isFinishing && !isDestroyed) {
                checkAndRequestPermissions()
            }
        }, 500)
    }

    override fun onResume() {
        super.onResume()
        currentActivityRef = WeakReference(this)
        if (pendingPermissionRequest) {
            pendingPermissionRequest = false
            checkAndRequestPermissions()
        }
        // Our CSS handles light/dark explicitly via [data-theme]; opt the
        // WebView out of Android's algorithmic darkening (Android 10+/16
        // re-inverts light-theme pixels when the OS is in dark mode, making
        // the theme button look dead). Tauri owns the WebView, so find it by
        // traversal once the view hierarchy is attached.
        Handler(Looper.getMainLooper()).postDelayed({
            if (!isFinishing && !isDestroyed) {
                disableWebViewForceDark()
            }
        }, 500)
    }

    override fun onDestroy() {
        if (currentActivityRef?.get() === this) {
            currentActivityRef = null
        }
        super.onDestroy()
    }

    fun checkAndRequestPermissions() {
        val permissionsToRequest = mutableListOf<String>()

        // Notification permission for foreground media playback (Android 13+ / API 33+)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            if (ContextCompat.checkSelfPermission(this, Manifest.permission.POST_NOTIFICATIONS)
                != PackageManager.PERMISSION_GRANTED
            ) {
                permissionsToRequest.add(Manifest.permission.POST_NOTIFICATIONS)
            }
            // Audio Media permission for MediaStore system-wide scanning (Android 13+)
            if (ContextCompat.checkSelfPermission(this, Manifest.permission.READ_MEDIA_AUDIO)
                != PackageManager.PERMISSION_GRANTED
            ) {
                permissionsToRequest.add(Manifest.permission.READ_MEDIA_AUDIO)
            }
        } else {
            // Legacy external storage permission (Android 12 and below)
            if (ContextCompat.checkSelfPermission(this, Manifest.permission.READ_EXTERNAL_STORAGE)
                != PackageManager.PERMISSION_GRANTED
            ) {
                permissionsToRequest.add(Manifest.permission.READ_EXTERNAL_STORAGE)
            }
        }

        if (permissionsToRequest.isNotEmpty()) {
            ActivityCompat.requestPermissions(
                this,
                permissionsToRequest.toTypedArray(),
                PERMISSION_REQUEST_CODE
            )
        }
    }

    /**
     * Opt every WebView in the hierarchy out of force/algorithmic darkening.
     * Uses the platform API on TIRAMISU+ (compileSdk 36) and the deprecated
     * forceDark flag on Q–S via suppression; all guarded so older WebViews
     * simply keep default behavior.
     */
    private fun disableWebViewForceDark() {
        try {
            val root = window?.decorView?.rootView as? android.view.ViewGroup ?: return
            val queue: ArrayDeque<android.view.View> = ArrayDeque()
            queue.add(root)
            while (queue.isNotEmpty()) {
                val v = queue.removeFirst()
                if (v is WebView) {
                    try {
                        val s = v.settings
                        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                            try {
                                s.isAlgorithmicDarkeningAllowed = false
                            } catch (_: Exception) { }
                        } else if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                            @Suppress("DEPRECATION")
                            try {
                                s.forceDark = WebView.FORCE_DARK_OFF
                            } catch (_: Exception) { }
                        }
                    } catch (_: Exception) { }
                } else if (v is android.view.ViewGroup) {
                    for (i in 0 until v.childCount) {
                        try {
                            v.getChildAt(i)?.let { queue.add(it) }
                        } catch (_: Exception) { }
                    }
                }
            }
        } catch (_: Exception) { }
    }
}
