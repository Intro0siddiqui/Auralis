package com.auralis.v2

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.Uri
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.drawable.Icon
import android.util.Log
import android.media.AudioAttributes
import android.media.AudioFocusRequest
import android.media.AudioManager
import android.media.MediaMetadata
import android.media.session.MediaSession
import android.media.session.PlaybackState
import android.os.Build
import android.os.IBinder
import android.os.PowerManager

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
        // v2 channel id: v1 shipped with IMPORTANCE_LOW on some installs and
        // Android keeps channel settings sticky across upgrades (create is a
        // no-op once the channel exists). Bumping the id forces a fresh
        // IMPORTANCE_DEFAULT channel so notifications are visible again.
        const val CHANNEL_ID = "auralis_playback_channel_v2"
        const val NOTIFICATION_ID = 101

        const val ACTION_PLAY = "com.auralis.v2.action.PLAY"
        const val ACTION_PAUSE = "com.auralis.v2.action.PAUSE"
        const val ACTION_NEXT = "com.auralis.v2.action.NEXT"
        const val ACTION_PREVIOUS = "com.auralis.v2.action.PREVIOUS"

        @JvmStatic
        fun createNotificationChannel(context: Context) {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                val channel = NotificationChannel(
                    CHANNEL_ID,
                    "Audio Playback",
                    NotificationManager.IMPORTANCE_DEFAULT
                ).apply {
                    description = "Keeps audio playback active in background"
                    setSound(null, null)
                    enableVibration(false)
                    lockscreenVisibility = Notification.VISIBILITY_PUBLIC
                }
                context.getSystemService(NotificationManager::class.java)?.createNotificationChannel(channel)
            }
        }

        /** Start (or refresh) the foreground service with the current track info. */
        @JvmStatic
        fun start(
            context: Context,
            title: String,
            artist: String,
            durationSecs: Int,
            positionSecs: Int,
            isPlaying: Boolean,
            artPath: String
        ) {
            createNotificationChannel(context)

            // If notification permission is missing on Android 13+, trigger request
            if (isPlaying && Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                if (context.checkSelfPermission(android.Manifest.permission.POST_NOTIFICATIONS)
                    != android.content.pm.PackageManager.PERMISSION_GRANTED
                ) {
                    MainActivity.requestRuntimePermissions(context)
                }
            }

            val intent = Intent(context, MediaPlaybackService::class.java).apply {
                putExtra("title", title)
                putExtra("artist", artist)
                putExtra("durationSecs", durationSecs)
                putExtra("positionSecs", positionSecs)
                putExtra("isPlaying", isPlaying)
                putExtra("artPath", artPath)
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
    private var audioManager: AudioManager? = null
    private var notificationManager: NotificationManager? = null
    private var audioFocusRequest: AudioFocusRequest? = null
    private var pausedByFocusLoss: Boolean = false
    private var wakeLock: PowerManager.WakeLock? = null

    override fun onCreate() {
        super.onCreate()
        audioManager = getSystemService(Context.AUDIO_SERVICE) as? AudioManager
        notificationManager = getSystemService(Context.NOTIFICATION_SERVICE) as? NotificationManager
        createNotificationChannel()
        setupMediaSession()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        // Handle media-button intents forwarded via PendingIntent (no track extras).
        val action = intent?.action
        if (action != null) {
            when (action) {
                ACTION_PLAY -> { pausedByFocusLoss = false; NativeBridge.command("play") }
                ACTION_PAUSE -> { pausedByFocusLoss = false; NativeBridge.command("pause") }
                ACTION_NEXT -> NativeBridge.command("next")
                ACTION_PREVIOUS -> NativeBridge.command("previous")
            }
            // Don't rebuild notification with default title/isPlaying=true; Rust will
            // push the corrected state asynchronously via start().
            return START_NOT_STICKY
        }
        if (intent == null) {
            // System restarted service after process death without Tauri runtime — cannot
            // resume audio; avoid ghost notification with stale defaults.
            return START_NOT_STICKY
        }

        val title = intent.getStringExtra("title") ?: "Auralis"
        val artist = intent.getStringExtra("artist").orEmpty()
        val durationSecs = intent.getIntExtra("durationSecs", 0)
        val positionSecs = intent.getIntExtra("positionSecs", 0)
        val isPlaying = intent.getBooleanExtra("isPlaying", true)
        val artPath = intent.getStringExtra("artPath").orEmpty()

        if (isPlaying) {
            requestAudioFocus()
            acquireWakeLock()
        } else {
            releaseWakeLock()
        }

        val art = loadArtBitmap(artPath)
        updateMediaSession(title, artist, durationSecs, positionSecs, isPlaying, art)
        val notification = buildNotification(title, artist, isPlaying, art)
        try {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                startForeground(
                    NOTIFICATION_ID,
                    notification,
                    ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK
                )
            } else {
                startForeground(NOTIFICATION_ID, notification)
            }
        } catch (e: Exception) {
            Log.e("AuralisMedia", "Failed to start foreground service", e)
            // Android 14+ (API 34, incl. 16): a background startForegroundService
            // throws ForegroundServiceStartNotAllowedException when the app has
            // no visible Activity (e.g. Rust watcher / auto-advance fires while
            // backgrounded). Fall back to a plain media notification so controls
            // are still visible instead of nothing at all.
            try {
                notificationManager?.notify(NOTIFICATION_ID, notification)
            } catch (notifyErr: Exception) {
                Log.e("AuralisMedia", "Failed fallback notification notify", notifyErr)
            }
            return START_NOT_STICKY
        }
        notificationManager?.notify(NOTIFICATION_ID, notification)
        return START_STICKY
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onDestroy() {
        releaseWakeLock()
        abandonAudioFocus()
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.N) {
            stopForeground(STOP_FOREGROUND_REMOVE)
        } else {
            @Suppress("DEPRECATION")
            stopForeground(true)
        }
        mediaSession?.isActive = false
        mediaSession?.release()
        mediaSession = null
        super.onDestroy()
    }

    private fun requestAudioFocus() {
        val am = audioManager ?: return
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val playbackAttributes = AudioAttributes.Builder()
                .setUsage(AudioAttributes.USAGE_MEDIA)
                .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC)
                .build()
            val req = AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN)
                .setAudioAttributes(playbackAttributes)
                .setAcceptsDelayedFocusGain(true)
                .setOnAudioFocusChangeListener { focusChange ->
                    when (focusChange) {
                        AudioManager.AUDIOFOCUS_LOSS,
                        AudioManager.AUDIOFOCUS_LOSS_TRANSIENT -> {
                            pausedByFocusLoss = true
                            NativeBridge.command("pause")
                        }
                        AudioManager.AUDIOFOCUS_LOSS_TRANSIENT_CAN_DUCK -> {
                            // Duck rather than pause — keep playing at lower volume.
                            // Rust/rodio volume ducking would require player affinity; for
                            // minimal fix we simply avoid pausing (audio continues).
                        }
                        AudioManager.AUDIOFOCUS_GAIN -> {
                            if (pausedByFocusLoss) {
                                pausedByFocusLoss = false
                                NativeBridge.command("play")
                            }
                        }
                    }
                }
                .build()
            audioFocusRequest = req
            am.requestAudioFocus(req)
        } else {
            @Suppress("DEPRECATION")
            am.requestAudioFocus(
                { focusChange ->
                    when (focusChange) {
                        AudioManager.AUDIOFOCUS_LOSS,
                        AudioManager.AUDIOFOCUS_LOSS_TRANSIENT -> {
                            pausedByFocusLoss = true
                            NativeBridge.command("pause")
                        }
                        AudioManager.AUDIOFOCUS_LOSS_TRANSIENT_CAN_DUCK -> {
                            // Duck — do not pause.
                        }
                        AudioManager.AUDIOFOCUS_GAIN -> {
                            if (pausedByFocusLoss) {
                                pausedByFocusLoss = false
                                NativeBridge.command("play")
                            }
                        }
                    }
                },
                AudioManager.STREAM_MUSIC,
                AudioManager.AUDIOFOCUS_GAIN
            )
        }
    }

    private fun abandonAudioFocus() {
        val am = audioManager ?: return
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            audioFocusRequest?.let { am.abandonAudioFocusRequest(it) }
            audioFocusRequest = null
        } else {
            @Suppress("DEPRECATION")
            am.abandonAudioFocus(null)
        }
    }

    private fun createNotificationChannel() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val channel = NotificationChannel(
                CHANNEL_ID,
                "Audio Playback",
                NotificationManager.IMPORTANCE_DEFAULT
            ).apply {
                description = "Keeps audio playback active in background"
                setSound(null, null)
                enableVibration(false)
                lockscreenVisibility = Notification.VISIBILITY_PUBLIC
            }
            getSystemService(NotificationManager::class.java)?.createNotificationChannel(channel)
        }
    }

    private fun setupMediaSession() {
        val session = MediaSession(this, "AuralisPlayback")
        val mediaButtonIntent = Intent(Intent.ACTION_MEDIA_BUTTON).apply {
            setClass(this@MediaPlaybackService, MediaPlaybackService::class.java)
        }
        val pendingMediaButton = PendingIntent.getService(
            this,
            0,
            mediaButtonIntent,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        )
        session.setMediaButtonReceiver(pendingMediaButton)
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
        session.isActive = true
        mediaSession = session
    }

    private fun loadArtBitmap(path: String): Bitmap? {
        if (path.isEmpty()) return null
        return try {
            if (path.startsWith("content://")) {
                val uri = Uri.parse(path)
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                    contentResolver.loadThumbnail(uri, android.util.Size(512, 512), null)
                } else {
                    contentResolver.openInputStream(uri)?.use { stream ->
                        val opts = BitmapFactory.Options().apply { inSampleSize = 2 }
                        BitmapFactory.decodeStream(stream, null, opts)
                    }
                }
            } else {
                val f = java.io.File(path)
                if (!f.exists()) return null
                // Downsample to avoid ANR/OOM decoding large art on main thread.
                val opts = BitmapFactory.Options().apply { inSampleSize = 2 }
                BitmapFactory.decodeFile(path, opts)
            }
        } catch (_: Exception) {
            null
        } catch (_: OutOfMemoryError) {
            null
        }
    }

    private fun acquireWakeLock() {
        if (wakeLock?.isHeld == true) return
        val pm = getSystemService(Context.POWER_SERVICE) as? PowerManager ?: return
        val wl = pm.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "Auralis:PlaybackWakeLock")
        wl.acquire() // Keep wake lock held indefinitely while playing to prevent background pause
        wakeLock = wl
    }

    private fun releaseWakeLock() {
        wakeLock?.let { if (it.isHeld) it.release() }
        wakeLock = null
    }

    private fun updateMediaSession(
        title: String,
        artist: String,
        durationSecs: Int,
        positionSecs: Int,
        isPlaying: Boolean,
        art: Bitmap? = null
    ) {
        val session = mediaSession ?: return
        val metadataBuilder = MediaMetadata.Builder()
            .putString(MediaMetadata.METADATA_KEY_TITLE, title)
            .putString(MediaMetadata.METADATA_KEY_ARTIST, artist.ifEmpty { "Auralis" })
            .putLong(MediaMetadata.METADATA_KEY_DURATION, (durationSecs.toLong() * 1000L).coerceAtLeast(0L))
        if (art != null) {
            metadataBuilder.putBitmap(MediaMetadata.METADATA_KEY_ALBUM_ART, art)
            metadataBuilder.putBitmap(MediaMetadata.METADATA_KEY_ART, art)
        }
        session.setMetadata(metadataBuilder.build())

        val state = if (isPlaying) PlaybackState.STATE_PLAYING else PlaybackState.STATE_PAUSED
        val actions = PlaybackState.ACTION_PLAY or
            PlaybackState.ACTION_PAUSE or
            PlaybackState.ACTION_PLAY_PAUSE or
            PlaybackState.ACTION_SKIP_TO_NEXT or
            PlaybackState.ACTION_SKIP_TO_PREVIOUS or
            PlaybackState.ACTION_SEEK_TO or
            PlaybackState.ACTION_STOP
        val playbackState = PlaybackState.Builder()
            .setActions(actions)
            .setState(state, (positionSecs.toLong() * 1000L).coerceAtLeast(0L), 1.0f)
            .build()
        session.setPlaybackState(playbackState)
        session.isActive = true
    }

    private fun buildNotification(title: String, artist: String, isPlaying: Boolean, art: Bitmap? = null): Notification {
        val builder = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            Notification.Builder(this, CHANNEL_ID)
        } else {
            @Suppress("DEPRECATION")
            Notification.Builder(this)
        }

        // Android 13+ (API 33-36) SystemUI requires framework icons to explicitly specify
        // package "android" via Icon.createWithResource("android", resId). Passing raw int
        // resource IDs causes SystemUI to look up system icons in com.auralis.v2 package,
        // throwing Resources$NotFoundException and suppressing the notification entirely.
        val smallIcon = Icon.createWithResource("android", android.R.drawable.ic_media_play)
        val pauseIcon = Icon.createWithResource("android", android.R.drawable.ic_media_pause)
        val playIcon = Icon.createWithResource("android", android.R.drawable.ic_media_play)
        val prevIcon = Icon.createWithResource("android", android.R.drawable.ic_media_previous)
        val nextIcon = Icon.createWithResource("android", android.R.drawable.ic_media_next)

        val playPauseAction = if (isPlaying) {
            Notification.Action.Builder(
                pauseIcon,
                "Pause",
                pendingServiceIntent(ACTION_PAUSE)
            ).build()
        } else {
            Notification.Action.Builder(
                playIcon,
                "Play",
                pendingServiceIntent(ACTION_PLAY)
            ).build()
        }

        val prevAction = Notification.Action.Builder(
            prevIcon,
            "Previous",
            pendingServiceIntent(ACTION_PREVIOUS)
        ).build()

        val nextAction = Notification.Action.Builder(
            nextIcon,
            "Next",
            pendingServiceIntent(ACTION_NEXT)
        ).build()

        if (art != null) {
            builder.setLargeIcon(art)
        }
        val mediaStyle = Notification.MediaStyle()
        mediaSession?.sessionToken?.let { token ->
            mediaStyle.setMediaSession(token)
        }
        mediaStyle.setShowActionsInCompactView(0, 1, 2)

        return builder
            .setContentTitle(title)
            .setContentText(artist.ifEmpty { "Auralis Music Player" })
            .setSmallIcon(smallIcon)
            .setCategory(Notification.CATEGORY_TRANSPORT)
            .setVisibility(Notification.VISIBILITY_PUBLIC)
            .setShowWhen(false)
            .setOngoing(isPlaying)
            .setStyle(mediaStyle)
            .addAction(prevAction)
            .addAction(playPauseAction)
            .addAction(nextAction)
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
