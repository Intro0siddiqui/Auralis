package com.auralis.desktop.service

import com.auralis.model.Track
import com.auralis.service.PlayerService
import com.auralis.service.PlayerState
import com.auralis.service.RepeatMode
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import javax.sound.sampled.AudioSystem
import javax.sound.sampled.Clip
import javax.sound.sampled.LineEvent

class DesktopPlayerService : PlayerService {
    private var clip: Clip? = null
    private val _state = MutableStateFlow(PlayerState())
    override val state: StateFlow<PlayerState> = _state.asStateFlow()

    override suspend fun play(track: Track) {
        stop()
        try {
            val audioInputStream = AudioSystem.getAudioInputStream(java.io.File(track.filePath))
            clip = AudioSystem.getClip().apply {
                open(audioInputStream)
                addLineListener { event ->
                    if (event.type == LineEvent.Type.STOP) {
                        close()
                        _state.update { it.copy(isPlaying = false) }
                    }
                }
                start()
            }
            _state.update {
                it.copy(
                    isPlaying = true,
                    currentTrack = track,
                    duration = clip?.microsecondLength ?: 0
                )
            }
        } catch (e: Exception) {
            _state.update { it.copy(isPlaying = false) }
        }
    }

    override suspend fun playQueue(tracks: List<Track>, startIndex: Int) {
        _state.update { it.copy(queue = tracks, currentIndex = startIndex) }
        tracks.getOrNull(startIndex)?.let { play(it) }
    }

    override suspend fun pause() {
        clip?.stop()
        _state.update { it.copy(isPlaying = false) }
    }

    override suspend fun resume() {
        clip?.start()
        _state.update { it.copy(isPlaying = true) }
    }

    override suspend fun stop() {
        clip?.stop()
        clip?.close()
        clip = null
        _state.update { it.copy(isPlaying = false, position = 0) }
    }

    override suspend fun seekTo(position: Long) {
        clip?.microsecondPosition = position
        _state.update { it.copy(position = position) }
    }

    override suspend fun next() {
        val currentState = _state.value
        val nextIndex = currentState.currentIndex + 1
        if (nextIndex < currentState.queue.size) {
            _state.update { it.copy(currentIndex = nextIndex) }
            play(currentState.queue[nextIndex])
        }
    }

    override suspend fun previous() {
        val currentState = _state.value
        val prevIndex = currentState.currentIndex - 1
        if (prevIndex >= 0) {
            _state.update { it.copy(currentIndex = prevIndex) }
            play(currentState.queue[prevIndex])
        }
    }

    override suspend fun setRepeatMode(mode: RepeatMode) {
        _state.update { it.copy(repeatMode = mode) }
    }

    override suspend fun toggleShuffle() {
        _state.update { it.copy(isShuffleEnabled = !it.isShuffleEnabled) }
    }

    override suspend fun addToQueue(track: Track) {
        _state.update { it.copy(queue = it.queue + track) }
    }

    override suspend fun removeFromQueue(index: Int) {
        _state.update { it.copy(queue = it.queue.toMutableList().apply { removeAt(index) }) }
    }

    override suspend fun clearQueue() {
        stop()
        _state.update { PlayerState() }
    }
}
