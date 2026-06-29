package com.auralis.service

import com.auralis.model.Track
import kotlinx.coroutines.flow.StateFlow

interface PlayerService {
    val state: StateFlow<PlayerState>

    suspend fun play(track: Track)
    suspend fun playQueue(tracks: List<Track>, startIndex: Int = 0)
    suspend fun pause()
    suspend fun resume()
    suspend fun stop()
    suspend fun seekTo(position: Long)
    suspend fun next()
    suspend fun previous()
    suspend fun setRepeatMode(mode: RepeatMode)
    suspend fun toggleShuffle()
    suspend fun addToQueue(track: Track)
    suspend fun removeFromQueue(index: Int)
    suspend fun clearQueue()
}
