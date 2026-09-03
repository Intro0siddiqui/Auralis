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

        @JvmStatic
        @JvmOverloads
        fun requestRuntimePermissions(context: Any? = null) {
            val act = (context as? MainActivity) ?: currentActivityRef?.get() ?: return
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
}
