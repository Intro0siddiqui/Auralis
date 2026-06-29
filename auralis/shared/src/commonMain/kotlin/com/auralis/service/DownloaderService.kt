package com.auralis.service

import com.auralis.model.AudioFormat
import kotlinx.coroutines.flow.Flow

interface DownloaderService {
    suspend fun downloadAudio(
        url: String,
        format: AudioFormat = AudioFormat.MP3,
        quality: Int = 320
    ): Flow<DownloadResult>

    suspend fun getMetadata(url: String): MediaMetadata?

    suspend fun isSupportedUrl(url: String): Boolean
}
