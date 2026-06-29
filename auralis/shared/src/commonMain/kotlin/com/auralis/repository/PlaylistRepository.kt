package com.auralis.repository

import com.auralis.database.AuralisDatabase
import com.auralis.model.Playlist
import com.auralis.model.Track
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

class PlaylistRepository(private val database: AuralisDatabase) {
    private val playlistQueries = database.playlistQueries
    private val playlistTrackQueries = database.playlistTrackQueries

    suspend fun getAllPlaylists(): List<Playlist> = withContext(Dispatchers.Default) {
        playlistQueries.selectAll().executeAsList().map { it.toDomain() }
    }

    suspend fun getPlaylistById(id: Long): Playlist? = withContext(Dispatchers.Default) {
        playlistQueries.selectById(id).executeAsOneOrNull()?.toDomain()
    }

    suspend fun insertPlaylist(playlist: Playlist): Long = withContext(Dispatchers.Default) {
        playlistQueries.insert(
            name = playlist.name,
            description = playlist.description,
            thumbnail_path = playlist.thumbnailPath,
            date_created = playlist.dateCreated,
            date_modified = playlist.dateModified
        )
    }

    suspend fun updatePlaylist(playlist: Playlist) = withContext(Dispatchers.Default) {
        playlistQueries.update(
            name = playlist.name,
            description = playlist.description,
            thumbnail_path = playlist.thumbnailPath,
            date_modified = System.currentTimeMillis(),
            id = playlist.id
        )
    }

    suspend fun deletePlaylist(id: Long) = withContext(Dispatchers.Default) {
        playlistTrackQueries.deleteAllFromPlaylist(id)
        playlistQueries.deleteById(id)
    }

    suspend fun addTrackToPlaylist(playlistId: Long, trackId: Long, position: Int) =
        withContext(Dispatchers.Default) {
            playlistTrackQueries.insert(
                playlist_id = playlistId,
                track_id = trackId,
                position = position,
                date_added = System.currentTimeMillis()
            )
        }

    suspend fun removeTrackFromPlaylist(playlistId: Long, trackId: Long) =
        withContext(Dispatchers.Default) {
            playlistTrackQueries.deleteFromPlaylist(playlistId, trackId)
        }

    suspend fun getPlaylistTracks(playlistId: Long): List<Track> = withContext(Dispatchers.Default) {
        playlistTrackQueries.selectByPlaylist(playlistId).executeAsList().map {
            // Simplified — full implementation would join with Track table
            Track(id = it.track_id, title = "Track ${it.track_id}", filePath = "")
        }
    }

    private fun com.auralis.database.Playlist.toDomain() = Playlist(
        id = id,
        name = name,
        description = description,
        thumbnailPath = thumbnail_path,
        dateCreated = date_created,
        dateModified = date_modified
    )
}
