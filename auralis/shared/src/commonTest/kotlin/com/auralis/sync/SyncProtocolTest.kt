package com.auralis.sync

import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream
import java.io.DataInputStream
import java.io.DataOutputStream
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue
import kotlin.test.assertFalse

class SyncProtocolTest {

    @Test
    fun testEncodeDecodeAuthRequest() {
        val message = SyncMessage.AuthRequest("testuser", "hash123")
        val encoded = SyncProtocol.encode(message)
        val decoded = SyncProtocol.decode(encoded) as SyncMessage.AuthRequest

        assertEquals("testuser", decoded.username)
        assertEquals("hash123", decoded.passwordHash)
    }

    @Test
    fun testEncodeDecodeAuthResponse() {
        val message = SyncMessage.AuthResponse(true, "Success")
        val encoded = SyncProtocol.encode(message)
        val decoded = SyncProtocol.decode(encoded) as SyncMessage.AuthResponse

        assertTrue(decoded.success)
        assertEquals("Success", decoded.message)
    }

    @Test
    fun testEncodeDecodeSyncRequest() {
        val message = SyncMessage.SyncRequest(
            includeTracks = true,
            includeMetadata = false,
            includePlaylists = true,
            includeThumbnails = false
        )
        val encoded = SyncProtocol.encode(message)
        val decoded = SyncProtocol.decode(encoded) as SyncMessage.SyncRequest

        assertTrue(decoded.includeTracks)
        assertFalse(decoded.includeMetadata)
        assertTrue(decoded.includePlaylists)
        assertFalse(decoded.includeThumbnails)
    }

    @Test
    fun testEncodeDecodeTrackList() {
        val tracks = listOf(
            SyncTrack(
                id = 1,
                title = "Test Song",
                artist = "Test Artist",
                album = "Test Album",
                genre = "Rock",
                year = 2024,
                trackNumber = 1,
                duration = 180000,
                filePath = "/path/to/song.mp3",
                format = "mp3",
                size = 5000000
            )
        )
        val message = SyncMessage.TrackList(tracks)
        val encoded = SyncProtocol.encode(message)
        val decoded = SyncProtocol.decode(encoded) as SyncMessage.TrackList

        assertEquals(1, decoded.tracks.size)
        assertEquals("Test Song", decoded.tracks[0].title)
        assertEquals("Test Artist", decoded.tracks[0].artist)
    }

    @Test
    fun testEncodeDecodeFileChunk() {
        val data = "test data".toByteArray()
        val encoded = Base64Util.encode(data)
        val message = SyncMessage.FileChunk(
            fileName = "test.mp3",
            chunkIndex = 0,
            totalChunks = 10,
            data = encoded,
            checksum = "abc123"
        )
        val msgEncoded = SyncProtocol.encode(message)
        val decoded = SyncProtocol.decode(msgEncoded) as SyncMessage.FileChunk

        assertEquals("test.mp3", decoded.fileName)
        assertEquals(0, decoded.chunkIndex)
        assertEquals(10, decoded.totalChunks)
        assertEquals(encoded, decoded.data)
        assertEquals("abc123", decoded.checksum)
    }

    @Test
    fun testWriteReadMessage() {
        val baos = ByteArrayOutputStream()
        val output = DataOutputStream(baos)

        val message = SyncMessage.DeviceInfo("Desktop", 100, 5)
        SyncProtocol.writeMessage(output, message)

        val bais = ByteArrayInputStream(baos.toByteArray())
        val input = DataInputStream(bais)

        val decoded = SyncProtocol.readMessage(input) as SyncMessage.DeviceInfo
        assertEquals("Desktop", decoded.deviceName)
        assertEquals(100, decoded.trackCount)
        assertEquals(5, decoded.playlistCount)
    }

    @Test
    fun testConnectionInfoJson() {
        val info = QrCodeManager.ConnectionInfo("192.168.1.100", 54321, "abc123")
        val json = info.toJson()

        val parsed = QrCodeManager.ConnectionInfo.fromJson(json)
        assertEquals("192.168.1.100", parsed?.host)
        assertEquals(54321, parsed?.port)
        assertEquals("abc123", parsed?.token)
    }

    @Test
    fun testQrMatrixGeneration() {
        val content = "test content"
        val matrix = QrCodeManager.generateQrMatrix(content)

        assertEquals(25, matrix.size)
        assertEquals(25, matrix[0].size)

        assertTrue(matrix[0][0])
        assertTrue(matrix[0][1])
        assertTrue(matrix[1][0])
    }
}
