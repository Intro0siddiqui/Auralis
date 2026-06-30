package com.auralis.service

import com.auralis.model.AudioFormat
import com.auralis.model.Track
import com.auralis.repository.TrackRepository
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.withContext
import java.io.File

data class ScanResult(
    val totalFound: Int = 0,
    val added: Int = 0,
    val skipped: Int = 0,
    val errors: List<String> = emptyList()
)

class MusicScanner(private val trackRepository: TrackRepository) {
    private val _scanState = MutableStateFlow<ScanState>(ScanState.Idle)
    val scanState: StateFlow<ScanState> = _scanState.asStateFlow()

    sealed class ScanState {
        data object Idle : ScanState()
        data class Scanning(val currentPath: String = "", val found: Int = 0) : ScanState()
        data class Complete(val result: ScanResult) : ScanState()
        data class Error(val message: String) : ScanState()
    }

    companion object {
        val SUPPORTED_EXTENSIONS = setOf("mp3", "flac", "aac", "ogg", "wav", "m4a")

        fun defaultScanDirectories(): List<File> {
            val home = System.getProperty("user.home") ?: return emptyList()
            val homeDir = File(home)
            return listOf(
                File(homeDir, "Music"),
                File(homeDir, "Downloads"),
                File(homeDir, "Documents"),
            ).filter { it.exists() && it.isDirectory }
        }
    }

    suspend fun scanDirectories(directories: List<File>): ScanResult = withContext(Dispatchers.IO) {
        _scanState.value = ScanState.Scanning()
        val existingPaths = trackRepository.getAllTracks().map { it.filePath }.toSet()
        var totalFound = 0
        var added = 0
        var skipped = 0
        val errors = mutableListOf<String>()

        for (dir in directories) {
            try {
                scanDirectory(dir, existingPaths) { path, count ->
                    _scanState.value = ScanState.Scanning(path, count)
                }.let { (found, newAdded, newSkipped, newErrors) ->
                    totalFound += found
                    added += newAdded
                    skipped += newSkipped
                    errors.addAll(newErrors)
                }
            } catch (e: Exception) {
                errors.add("Failed to scan ${dir.path}: ${e.message}")
            }
        }

        val result = ScanResult(totalFound, added, skipped, errors)
        _scanState.value = ScanState.Complete(result)
        result
    }

    suspend fun scanDirectory(directory: File): ScanResult = withContext(Dispatchers.IO) {
        _scanState.value = ScanState.Scanning()
        val existingPaths = trackRepository.getAllTracks().map { it.filePath }.toSet()
        val (found, newAdded, newSkipped, newErrors) = scanDirectory(directory, existingPaths) { path, count ->
            _scanState.value = ScanState.Scanning(path, count)
        }
        val result = ScanResult(found, newAdded, newSkipped, newErrors)
        _scanState.value = ScanState.Complete(result)
        result
    }

    private suspend fun scanDirectory(
        directory: File,
        existingPaths: Set<String>,
        onProgress: (String, Int) -> Unit
    ): ScanDirectoryResult {
        var totalFound = 0
        var added = 0
        var skipped = 0
        val errors = mutableListOf<String>()

        try {
            val files = directory.listFiles() ?: return ScanDirectoryResult(0, 0, 0, emptyList())

            for (file in files) {
                if (file.isDirectory) {
                    val subResult = scanDirectory(file, existingPaths, onProgress)
                    totalFound += subResult.found
                    added += subResult.added
                    skipped += subResult.skipped
                    errors.addAll(subResult.errors)
                } else if (isAudioFile(file)) {
                    totalFound++
                    onProgress(file.path, totalFound)

                    if (existingPaths.contains(file.absolutePath)) {
                        skipped++
                        continue
                    }

                    try {
                        val track = fileToTrack(file)
                        trackRepository.insertTrack(track)
                        added++
                    } catch (e: Exception) {
                        errors.add("Failed to add ${file.name}: ${e.message}")
                    }
                }
            }
        } catch (e: Exception) {
            errors.add("Error scanning ${directory.path}: ${e.message}")
        }

        return ScanDirectoryResult(totalFound, added, skipped, errors)
    }

    private fun isAudioFile(file: File): Boolean {
        val ext = file.extension.lowercase()
        return ext in SUPPORTED_EXTENSIONS
    }

    private fun fileToTrack(file: File): Track {
        val name = file.nameWithoutExtension
        val parts = name.split(" - ", limit = 2)
        val title = if (parts.size > 1) parts[1].trim() else name
        val artist = if (parts.size > 1) parts[0].trim() else null

        return Track(
            title = title,
            artist = artist,
            filePath = file.absolutePath,
            format = AudioFormat.fromExtension(file.extension),
            size = file.length(),
            dateAdded = System.currentTimeMillis(),
            source = "local"
        )
    }

    private data class ScanDirectoryResult(
        val found: Int,
        val added: Int,
        val skipped: Int,
        val errors: List<String>
    )
}
