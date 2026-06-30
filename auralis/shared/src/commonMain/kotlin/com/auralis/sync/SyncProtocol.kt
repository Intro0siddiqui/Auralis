package com.auralis.sync

import kotlinx.serialization.Serializable
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import java.io.DataInputStream
import java.io.DataOutputStream
import java.io.IOException

private val json = Json { ignoreUnknownKeys = true; prettyPrint = false }

@Serializable
sealed class SyncMessage {
    @Serializable
    data class AuthRequest(val username: String, val passwordHash: String) : SyncMessage()

    @Serializable
    data class TempCodeAuth(val tempCode: String) : SyncMessage()

    @Serializable
    data class AuthResponse(val success: Boolean, val message: String) : SyncMessage()

    @Serializable
    data class SyncRequest(
        val includeTracks: Boolean,
        val includeMetadata: Boolean,
        val includePlaylists: Boolean,
        val includeThumbnails: Boolean
    ) : SyncMessage()

    @Serializable
    data class TrackList(val tracks: List<SyncTrack>) : SyncMessage()

    @Serializable
    data class PlaylistList(val playlists: List<SyncPlaylist>) : SyncMessage()

    @Serializable
    data class FileChunk(
        val fileName: String,
        val chunkIndex: Int,
        val totalChunks: Int,
        val data: String,
        val checksum: String
    ) : SyncMessage()

    @Serializable
    data class FileRequest(val fileName: String) : SyncMessage()

    @Serializable
    data class SyncProgress(
        val current: Int,
        val total: Int,
        val fileName: String
    ) : SyncMessage()

    @Serializable
    data class SyncComplete(val message: String) : SyncMessage()

    @Serializable
    data class Error(val message: String) : SyncMessage()

    @Serializable
    data class DeviceInfo(
        val deviceName: String,
        val trackCount: Int,
        val playlistCount: Int
    ) : SyncMessage()
}

@Serializable
data class SyncTrack(
    val id: Long,
    val title: String,
    val artist: String?,
    val album: String?,
    val genre: String?,
    val year: Int?,
    val trackNumber: Int?,
    val duration: Long,
    val filePath: String,
    val format: String,
    val size: Long
)

@Serializable
data class SyncPlaylist(
    val id: Long,
    val name: String,
    val description: String?,
    val trackIds: List<Long>
)

object SyncProtocol {
    const val CHUNK_SIZE = 65536

    fun encode(message: SyncMessage): ByteArray {
        val jsonStr = json.encodeToString<SyncMessage>(message)
        return jsonStr.toByteArray(Charsets.UTF_8)
    }

    fun decode(data: ByteArray): SyncMessage {
        val jsonStr = String(data, Charsets.UTF_8)
        return json.decodeFromString<SyncMessage>(jsonStr)
    }

    fun writeMessage(output: DataOutputStream, message: SyncMessage) {
        val bytes = encode(message)
        output.writeInt(bytes.size)
        output.write(bytes)
        output.flush()
    }

    fun readMessage(input: DataInputStream): SyncMessage? {
        return try {
            val length = input.readInt()
            if (length <= 0 || length > 10 * 1024 * 1024) return null
            val bytes = ByteArray(length)
            input.readFully(bytes)
            decode(bytes)
        } catch (e: IOException) {
            null
        }
    }
}
