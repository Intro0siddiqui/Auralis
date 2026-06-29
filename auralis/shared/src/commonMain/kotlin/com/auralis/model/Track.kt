package com.auralis.model

data class Track(
    val id: Long = 0,
    val title: String,
    val artist: String? = null,
    val album: String? = null,
    val genre: String? = null,
    val year: Int? = null,
    val trackNumber: Int? = null,
    val duration: Long = 0,
    val filePath: String,
    val thumbnailPath: String? = null,
    val format: AudioFormat = AudioFormat.MP3,
    val size: Long = 0,
    val dateAdded: Long = System.currentTimeMillis(),
    val source: String? = null,
    val sourceUrl: String? = null
) {
    val durationFormatted: String
        get() {
            val minutes = duration / 60000
            val seconds = (duration % 60000) / 1000
            return "%d:%02d".format(minutes, seconds)
        }

    val sizeFormatted: String
        get() = when {
            size < 1024 -> "$size B"
            size < 1024 * 1024 -> "${size / 1024} KB"
            else -> "%.1f MB".format(size / (1024.0 * 1024.0))
        }
}
