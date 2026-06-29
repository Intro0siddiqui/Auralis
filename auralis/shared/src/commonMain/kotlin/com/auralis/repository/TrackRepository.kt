package com.auralis.repository

import com.auralis.database.AuralisDatabase
import com.auralis.model.AudioFormat
import com.auralis.model.Track
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

class TrackRepository(private val database: AuralisDatabase) {
    private val queries = database.trackQueries

    suspend fun getAllTracks(): List<Track> = withContext(Dispatchers.Default) {
        queries.selectAll().executeAsList().map { it.toDomain() }
    }

    suspend fun getTrackById(id: Long): Track? = withContext(Dispatchers.Default) {
        queries.selectById(id).executeAsOneOrNull()?.toDomain()
    }

    suspend fun searchTracks(query: String): List<Track> = withContext(Dispatchers.Default) {
        queries.selectByTitle("%$query%").executeAsList().map { it.toDomain() }
    }

    suspend fun getTracksBySource(source: String): List<Track> = withContext(Dispatchers.Default) {
        queries.selectBySource(source).executeAsList().map { it.toDomain() }
    }

    suspend fun insertTrack(track: Track): Long = withContext(Dispatchers.Default) {
        queries.insert(
            title = track.title,
            artist = track.artist,
            album = track.album,
            genre = track.genre,
            year = track.year?.toLong(),
            track_number = track.trackNumber?.toLong(),
            duration = track.duration,
            file_path = track.filePath,
            thumbnail_path = track.thumbnailPath,
            format = track.format.extension,
            size = track.size,
            date_added = track.dateAdded,
            source = track.source,
            source_url = track.sourceUrl
        )
    }

    suspend fun updateTrack(track: Track) = withContext(Dispatchers.Default) {
        queries.update(
            title = track.title,
            artist = track.artist,
            album = track.album,
            genre = track.genre,
            year = track.year?.toLong(),
            track_number = track.trackNumber?.toLong(),
            id = track.id
        )
    }

    suspend fun deleteTrack(id: Long) = withContext(Dispatchers.Default) {
        queries.deleteById(id)
    }

    private fun com.auralis.database.Track.toDomain() = Track(
        id = id,
        title = title,
        artist = artist,
        album = album,
        genre = genre,
        year = year?.toInt(),
        trackNumber = track_number?.toInt(),
        duration = duration,
        filePath = file_path,
        thumbnailPath = thumbnail_path,
        format = AudioFormat.fromExtension(format),
        size = size,
        dateAdded = date_added,
        source = source,
        sourceUrl = source_url
    )
}
