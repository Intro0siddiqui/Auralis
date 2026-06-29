package com.auralis.service

sealed class DownloadResult {
    data class Success(val filePath: String, val metadata: MediaMetadata) : DownloadResult()
    data class Error(val message: String, val exception: Exception? = null) : DownloadResult()
    data class Progress(val percent: Float, val speed: String? = null) : DownloadResult()
}

data class MediaMetadata(
    val title: String,
    val artist: String? = null,
    val thumbnailUrl: String? = null,
    val duration: Long? = null,
    val source: String,
    val sourceUrl: String
)
