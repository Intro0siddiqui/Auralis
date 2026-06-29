package com.auralis.service

import com.auralis.model.Track

data class PlayerState(
    val isPlaying: Boolean = false,
    val currentTrack: Track? = null,
    val queue: List<Track> = emptyList(),
    val currentIndex: Int = -1,
    val position: Long = 0,
    val duration: Long = 0,
    val repeatMode: RepeatMode = RepeatMode.OFF,
    val isShuffleEnabled: Boolean = false
) {
    val progress: Float
        get() = if (duration > 0) position.toFloat() / duration else 0f

    val positionFormatted: String
        get() = formatTime(position)

    val durationFormatted: String
        get() = formatTime(duration)

    private fun formatTime(ms: Long): String {
        val minutes = ms / 60000
        val seconds = (ms % 60000) / 1000
        return "%d:%02d".format(minutes, seconds)
    }
}

enum class RepeatMode {
    OFF, ONE, ALL
}
