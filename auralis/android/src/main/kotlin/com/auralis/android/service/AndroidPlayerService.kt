package com.auralis.android.service

import android.content.Context
import androidx.media3.common.MediaItem
import androidx.media3.exoplayer.ExoPlayer
import com.auralis.model.Track
import com.auralis.service.PlayerService
import com.auralis.service.PlayerState
import com.auralis.service.RepeatMode
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update

class AndroidPlayerService(private val context: Context) : PlayerService {
    private var exoPlayer: ExoPlayer? = null
    private val _state = MutableStateFlow(PlayerState())
    override val state: StateFlow<PlayerState> = _state.asStateFlow()

    init {
        exoPlayer = ExoPlayer.Builder(context).build().apply {
            playWhenReady = true
        }
    }

    override suspend fun play(track: Track) {
        exoPlayer?.apply {
            val mediaItem = MediaItem.fromUri(track.filePath)
            setMediaItem(mediaItem)
            prepare()
            play()
        }
        _state.update { it.copy(isPlaying = true, currentTrack = track) }
    }

    override suspend fun playQueue(tracks: List<Track>, startIndex: Int) {
        exoPlayer?.apply {
            val mediaItems = tracks.map { MediaItem.fromUri(it.filePath) }
            setMediaItems(mediaItems, startIndex, 0)
            prepare()
            play()
        }
        _state.update {
            it.copy(
                isPlaying = true,
                queue = tracks,
                currentIndex = startIndex,
                currentTrack = tracks.getOrNull(startIndex)
            )
        }
    }

    override suspend fun pause() {
        exoPlayer?.pause()
        _state.update { it.copy(isPlaying = false) }
    }

    override suspend fun resume() {
        exoPlayer?.play()
        _state.update { it.copy(isPlaying = true) }
    }

    override suspend fun stop() {
        exoPlayer?.stop()
        _state.update { it.copy(isPlaying = false, currentTrack = null) }
    }

    override suspend fun seekTo(position: Long) {
        exoPlayer?.seekTo(position)
        _state.update { it.copy(position = position) }
    }

    override suspend fun next() {
        val currentState = _state.value
        val nextIndex = currentState.currentIndex + 1
        if (nextIndex < currentState.queue.size) {
            val track = currentState.queue[nextIndex]
            play(track)
            _state.update { it.copy(currentIndex = nextIndex) }
        }
    }

    override suspend fun previous() {
        val currentState = _state.value
        val prevIndex = currentState.currentIndex - 1
        if (prevIndex >= 0) {
            val track = currentState.queue[prevIndex]
            play(track)
            _state.update { it.copy(currentIndex = prevIndex) }
        }
    }

    override suspend fun setRepeatMode(mode: RepeatMode) {
        exoPlayer?.repeatMode = when (mode) {
            RepeatMode.OFF -> ExoPlayer.REPEAT_MODE_OFF
            RepeatMode.ONE -> ExoPlayer.REPEAT_MODE_ONE
            RepeatMode.ALL -> ExoPlayer.REPEAT_MODE_ALL
        }
        _state.update { it.copy(repeatMode = mode) }
    }

    override suspend fun toggleShuffle() {
        val newState = !_state.value.isShuffleEnabled
        exoPlayer?.shuffleModeEnabled = newState
        _state.update { it.copy(isShuffleEnabled = newState) }
    }

    override suspend fun addToQueue(track: Track) {
        _state.update { it.copy(queue = it.queue + track) }
    }

    override suspend fun removeFromQueue(index: Int) {
        _state.update { it.copy(queue = it.queue.toMutableList().apply { removeAt(index) }) }
    }

    override suspend fun clearQueue() {
        exoPlayer?.stop()
        _state.update { PlayerState() }
    }

    fun release() {
        exoPlayer?.release()
        exoPlayer = null
    }
}
