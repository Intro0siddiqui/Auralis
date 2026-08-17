package com.auralis.v2

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.media.MediaMetadata
import android.media.session.MediaSession
import android.media.session.PlaybackState
import android.os.Build
import android.os.IBinder

/**
 * JNI bridge into the Rust audio engine (libauralis_lib.so).
 *
 * Command strings mirror the backend operations: "play" / "pause" / "next" /
 * "previous" / "seek:<seconds>". The Rust side dispatches them to the same
 * playback commands as the UI, so the app and this notification stay in sync.
 */
object NativeBridge {
    init {
        System.loadLibrary("auralis_lib")
    }

    external fun command(cmd: String): String
}

/**
 * Foreground media playback service.
 *
 * While audio is playing, this service holds a foreground notification so the
 * OS does not kill the process (which would stop the Rust/oboe audio engine).
 * The notification and the MediaSession expose play/pause/next/previous/seek
 * controls that are forwarded into Rust via [NativeBridge].
 *
 * Started from Rust via `MediaPlaybackService.start(...)` (JNI) whenever
 * playback begins; updated on every state change; stopped when playback ends.
 */
class MediaPlaybackService : Service() {

    companion object {
        const val CHANNEL_ID = "auralis_playback_channel"
        const val NOTIFICATION_ID = 101

        const val ACTION_PLAY = "com.auralis.v2.action.PLAY"
        const val ACTION_PAUSE = "com.auralis.v2.action.PAUSE"
        const val ACTION_NEXT = "com.auralis.v2.action.NEXT"
        const val ACTION_PREVIOUS = "com.auralis.v2.action.PREVIOUS"

        /** Start (or refresh) the foreground service with the current track info. */
        @JvmStatic
        fun start(
            context: Context,
            title: String,
            artist: String,
            durationSecs: Int,
            positionSecs: Int,
            isPlaying: Boolean
        ) {
            val intent = Intent(context, MediaPlaybackService::class.java).apply {
                putExtra("title", title)
                putExtra("artist", artist)
                putExtra("durationSecs", durationSecs)
                putExtra("positionSecs", positionSecs)
                putExtra("isPlaying", isPlaying)
            }
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                context.startForegroundService(intent)
            } else {
                context.startService(intent)
            }
        }

        /** Stop the foreground service (queue exhausted / explicit stop). */
        @JvmStatic
        fun stop(context: Context) {
            context.stopService(Intent(context, MediaPlaybackService::class.java))
        }
    }

    private var mediaSession: MediaSession? = null

    override fun onCreate() {
        super.onCreate()
        createNotificationChannel()
        setupMediaSession()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_PLAY -> NativeBridge.command("play")
            ACTION_PAUSE -> NativeBridge.command("pause")
            ACTION_NEXT -> NativeBridge.command("next")
            ACTION_PREVIOUS -> NativeBridge.command("previous")
        }

        val title = intent?.getStringExtra("title") ?: "Auralis"
        val artist = intent?.getStringExtra("artist").orEmpty()
        val durationSecs = intent?.getIntExtra("durationSecs", 0) ?: 0
        val positionSecs = intent?.getIntExtra("positionSecs", 0) ?: 0
        val isPlaying = intent?.getBooleanExtra("isPlaying", true) ?: true

        startForeground(NOTIFICATION_ID, buildNotification(title, artist, isPlaying))
        updateMediaSession(title, artist, durationSecs, positionSecs, isPlaying)
        return START_STICKY
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onDestroy() {
        mediaSession?.isActive = false
        mediaSession?.release()
        mediaSession = null
        super.onDestroy()
    }

    private fun createNotificationChannel() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val channel = NotificationChannel(
                CHANNEL_ID,
                "Audio Playback",
                NotificationManager.IMPORTANCE_LOW
            ).apply {
                description = "Keeps audio playback active in background"
            }
            getSystemService(NotificationManager::class.java)?.createNotificationChannel(channel)
        }
    }

    private fun setupMediaSession() {
        val session = MediaSession(this, "AuralisPlayback")
        session.setCallback(object : MediaSession.Callback() {
            override fun onPlay() {
                NativeBridge.command("play")
            }
            override fun onPause() {
                NativeBridge.command("pause")
            }
            override fun onSkipToNext() {
                NativeBridge.command("next")
            }
            override fun onSkipToPrevious() {
                NativeBridge.command("previous")
            }
            override fun onSeekTo(pos: Long) {
                NativeBridge.command("seek:${pos / 1000}")
            }
        })
        session.setFlags(
            MediaSession.FLAG_HANDLES_MEDIA_BUTTONS or MediaSession.FLAG_HANDLES_TRANSPORT_CONTROLS
        )
        mediaSession = session
    }

    private fun updateMediaSession(
        title: String,
        artist: String,
        durationSecs: Int,
        positionSecs: Int,
        isPlaying: Boolean
    ) {
        val session = mediaSession ?: return
        val metadata = MediaMetadata.Builder()
            .putString(MediaMetadata.METADATA_KEY_TITLE, title)
            .putString(MediaMetadata.METADATA_KEY_ARTIST, artist)
            .putLong(MediaMetadata.METADATA_KEY_DURATION, durationSecs * 1000L)
            .build()
        val actions = PlaybackState.ACTION_PLAY or PlaybackState.ACTION_PAUSE or
            PlaybackState.ACTION_PLAY_PAUSE or PlaybackState.ACTION_SKIP_TO_NEXT or
            PlaybackState.ACTION_SKIP_TO_PREVIOUS or PlaybackState.ACTION_SEEK_TO
        val state = PlaybackState.Builder()
            .setActions(actions)
            .setState(
                if (isPlaying) PlaybackState.STATE_PLAYING else PlaybackState.STATE_PAUSED,
                positionSecs * 1000L,
                1f
            )
            .build()
        session.setMetadata(metadata)
        session.setPlaybackState(state)
        session.isActive = true
    }

    private fun buildNotification(title: String, artist: String, isPlaying: Boolean): Notification {
        val builder = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            Notification.Builder(this, CHANNEL_ID)
        } else {
            @Suppress("DEPRECATION")
            Notification.Builder(this)
        }
        val playPause = if (isPlaying) {
            Notification.Action.Builder(
                android.R.drawable.ic_media_pause,
                "Pause",
                pendingServiceIntent(ACTION_PAUSE)
            ).build()
        } else {
            Notification.Action.Builder(
                android.R.drawable.ic_media_play,
                "Play",
                pendingServiceIntent(ACTION_PLAY)
            ).build()
        }
        return builder
            .setContentTitle(title)
            .setContentText(artist.ifEmpty { "Auralis Music Player" })
            .setSmallIcon(android.R.drawable.ic_media_play)
            .setCategory(Notification.CATEGORY_TRANSPORT)
            .setVisibility(Notification.VISIBILITY_PUBLIC)
            .setShowWhen(false)
            .setOngoing(true)
            .addAction(
                android.R.drawable.ic_media_previous,
                "Previous",
                pendingServiceIntent(ACTION_PREVIOUS)
            )
            .addAction(playPause)
            .addAction(android.R.drawable.ic_media_next, "Next", pendingServiceIntent(ACTION_NEXT))
            .setContentIntent(pendingActivityIntent())
            .build()
    }

    private fun pendingServiceIntent(action: String): PendingIntent {
        val intent = Intent(this, MediaPlaybackService::class.java).setAction(action)
        val flags = PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        return PendingIntent.getService(this, action.hashCode(), intent, flags)
    }

    /** Tapping the notification brings the app back to the foreground. */
    private fun pendingActivityIntent(): PendingIntent {
        val intent = Intent(this, MainActivity::class.java).apply {
            flags = Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP or
                Intent.FLAG_ACTIVITY_NEW_TASK
        }
        val flags = PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        return PendingIntent.getActivity(this, 0, intent, flags)
    }
}
