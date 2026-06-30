package com.auralis.sync

import com.auralis.database.AuralisDatabase
import com.auralis.model.Track
import com.auralis.model.Playlist
import com.auralis.repository.TrackRepository
import com.auralis.repository.PlaylistRepository
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import java.io.File
import java.io.FileInputStream
import java.io.FileOutputStream
import java.security.MessageDigest
import java.util.UUID

sealed class SyncState {
    data object Idle : SyncState()
    data class Hosting(val port: Int, val token: String, val ip: String, val tempCode: String) : SyncState()
    data class Connecting(val host: String, val port: Int) : SyncState()
    data class Authenticating(val deviceId: String) : SyncState()
    data class Selecting(
        val availableTracks: Int,
        val availablePlaylists: Int
    ) : SyncState()
    data class Syncing(
        val totalFiles: Int,
        val completedFiles: Int,
        val currentFile: String,
        val bytesTransferred: Long,
        val totalBytes: Long
    ) : SyncState()
    data class Completed(val message: String) : SyncState()
    data class Error(val message: String) : SyncState()
}

class SyncService(
    private val database: AuralisDatabase,
    private val storageDir: File
) {
    private val trackRepository = TrackRepository(database)
    private val playlistRepository = PlaylistRepository(database)
    private val accountManager = AccountManager(database)
    private val discovery = DeviceDiscovery()

    private var server: TcpServer? = null
    private var client: TcpClient? = null
    private var currentToken: String = ""
    private var currentTempCode: String = ""
    private var syncScope: CoroutineScope? = null

    private val _state = MutableStateFlow<SyncState>(SyncState.Idle)
    val state: StateFlow<SyncState> = _state.asStateFlow()

    private val _progress = MutableStateFlow(0f)
    val progress: StateFlow<Float> = _progress.asStateFlow()

    val discoveredDevices: StateFlow<Map<String, DiscoveredDevice>> = discovery.devices

    fun startHosting(username: String) {
        syncScope?.cancel()
        syncScope = CoroutineScope(Dispatchers.IO + SupervisorJob())

        currentToken = UUID.randomUUID().toString().take(16)
        currentTempCode = QrCodeManager.generateTempCode()

        val trackCount = runBlocking { trackRepository.getAllTracks().size }
        val playlistCount = runBlocking { playlistRepository.getAllPlaylists().size }

        val ip = discovery.getLocalIpAddress() ?: "0.0.0.0"

        server = TcpServer(
            onClientConnected = { clientId, _ ->
                _state.value = SyncState.Authenticating(clientId)
            },
            onClientDisconnected = { clientId ->
                if (_state.value is SyncState.Authenticating ||
                    _state.value is SyncState.Selecting) {
                    _state.value = SyncState.Hosting(
                        port = server?.port ?: 0,
                        token = currentToken,
                        ip = ip,
                        tempCode = currentTempCode
                    )
                }
            },
            onMessage = { clientId, message -> handleServerMessage(clientId, message) },
            onError = { error ->
                _state.value = SyncState.Error(error)
            }
        )

        server?.start()

        val port = server?.port ?: 0
        _state.value = SyncState.Hosting(port = port, token = currentToken, ip = ip, tempCode = currentTempCode)

        discovery.startHosting(username, port, trackCount, playlistCount)
    }

    fun startDiscovery() {
        discovery.startScanning()
    }

    private fun handleServerMessage(clientId: String, message: SyncMessage) {
        when (message) {
            is SyncMessage.AuthRequest -> {
                val valid = runBlocking {
                    val account = accountManager.getAccount(message.username)
                    account != null && account.passwordHash == message.passwordHash
                }
                val response = SyncMessage.AuthResponse(
                    success = valid,
                    message = if (valid) "Authenticated" else "Invalid credentials"
                )
                server?.sendMessage(clientId, response)

                if (valid) {
                    val trackCount = runBlocking { trackRepository.getAllTracks().size }
                    val playlistCount = runBlocking { playlistRepository.getAllPlaylists().size }
                    server?.sendMessage(clientId, SyncMessage.DeviceInfo(
                        deviceName = System.getProperty("user.name") ?: "Desktop",
                        trackCount = trackCount,
                        playlistCount = playlistCount
                    ))
                    _state.value = SyncState.Selecting(trackCount, playlistCount)
                }
            }

            is SyncMessage.TempCodeAuth -> {
                val valid = message.tempCode == currentTempCode
                val response = SyncMessage.AuthResponse(
                    success = valid,
                    message = if (valid) "Authenticated" else "Invalid temp code"
                )
                server?.sendMessage(clientId, response)

                if (valid) {
                    val trackCount = runBlocking { trackRepository.getAllTracks().size }
                    val playlistCount = runBlocking { playlistRepository.getAllPlaylists().size }
                    server?.sendMessage(clientId, SyncMessage.DeviceInfo(
                        deviceName = System.getProperty("user.name") ?: "Desktop",
                        trackCount = trackCount,
                        playlistCount = playlistCount
                    ))
                    _state.value = SyncState.Selecting(trackCount, playlistCount)
                }
            }

            is SyncMessage.SyncRequest -> {
                syncScope?.launch {
                    performServerSync(clientId, message)
                }
            }

            is SyncMessage.FileRequest -> {
                syncScope?.launch {
                    sendRequestedFile(clientId, message.fileName)
                }
            }

            else -> {}
        }
    }

    private suspend fun performServerSync(clientId: String, request: SyncMessage.SyncRequest) {
        try {
            if (request.includeTracks || request.includeMetadata) {
                val tracks = trackRepository.getAllTracks()
                val syncTracks = tracks.map { track ->
                    SyncTrack(
                        id = track.id,
                        title = track.title,
                        artist = track.artist,
                        album = track.album,
                        genre = track.genre,
                        year = track.year,
                        trackNumber = track.trackNumber,
                        duration = track.duration,
                        filePath = track.filePath,
                        format = track.format.extension,
                        size = track.size
                    )
                }
                server?.sendMessage(clientId, SyncMessage.TrackList(syncTracks))
            }

            if (request.includePlaylists) {
                val playlists = playlistRepository.getAllPlaylists()
                val syncPlaylists = playlists.map { playlist ->
                    SyncPlaylist(
                        id = playlist.id,
                        name = playlist.name,
                        description = playlist.description,
                        trackIds = emptyList()
                    )
                }
                server?.sendMessage(clientId, SyncMessage.PlaylistList(syncPlaylists))
            }

            if (request.includeTracks) {
                val tracks = trackRepository.getAllTracks()
                val totalBytes = tracks.sumOf { it.size }
                var transferred = 0L
                var fileIndex = 0

                for (track in tracks) {
                    fileIndex++
                    _state.value = SyncState.Syncing(
                        totalFiles = tracks.size,
                        completedFiles = fileIndex - 1,
                        currentFile = track.title,
                        bytesTransferred = transferred,
                        totalBytes = totalBytes
                    )

                    sendFileToClient(clientId, File(track.filePath), track.title)
                    transferred += track.size
                    _progress.value = if (totalBytes > 0) transferred.toFloat() / totalBytes.toFloat() else 0f
                }
            }

            server?.sendMessage(clientId, SyncMessage.SyncComplete("Sync completed"))
            _state.value = SyncState.Completed("Sync completed successfully")
        } catch (e: Exception) {
            _state.value = SyncState.Error("Sync failed: ${e.message}")
        }
    }

    private suspend fun sendFileToClient(clientId: String, file: File, displayName: String) {
        if (!file.exists()) return

        val totalChunks = ((file.length() + SyncProtocol.CHUNK_SIZE - 1) / SyncProtocol.CHUNK_SIZE).toInt()
        val buffer = ByteArray(SyncProtocol.CHUNK_SIZE)
        val digest = MessageDigest.getInstance("SHA-256")
        var chunkIndex = 0

        withContext(Dispatchers.IO) {
            FileInputStream(file).use { fis ->
                while (chunkIndex < totalChunks) {
                    val bytesRead = fis.read(buffer)
                    if (bytesRead <= 0) break

                    val chunkData = if (bytesRead == buffer.size) buffer else buffer.copyOf(bytesRead)
                    digest.update(chunkData)
                    val checksum = Base64Util.encode(digest.digest())

                    server?.sendFileChunk(clientId, displayName, chunkIndex, totalChunks, chunkData, checksum)
                    chunkIndex++
                    yield()
                }
            }
        }
    }

    private fun sendRequestedFile(clientId: String, fileName: String) {
        syncScope?.launch {
            val tracks = trackRepository.getAllTracks()
            val track = tracks.find { it.title == fileName || it.filePath.endsWith(fileName) }
            if (track != null) {
                sendFileToClient(clientId, File(track.filePath), fileName)
            }
        }
    }

    fun connectToHost(host: String, port: Int, username: String, password: String) {
        syncScope?.cancel()
        syncScope = CoroutineScope(Dispatchers.IO + SupervisorJob())

        client = TcpClient(
            onMessage = { message -> handleClientMessage(message) },
            onDisconnected = {
                if (_state.value !is SyncState.Completed) {
                    _state.value = SyncState.Error("Disconnected from host")
                }
            },
            onError = { error ->
                _state.value = SyncState.Error(error)
            }
        )

        _state.value = SyncState.Connecting(host, port)

        syncScope?.launch {
            val connected = client!!.connect(host, port)
            if (connected) {
                val passwordHash = hashPasswordForAuth(password)
                client?.sendMessage(SyncMessage.AuthRequest(username, passwordHash))
            } else {
                _state.value = SyncState.Error("Failed to connect")
            }
        }
    }

    fun connectWithTempCode(host: String, port: Int, tempCode: String) {
        syncScope?.cancel()
        syncScope = CoroutineScope(Dispatchers.IO + SupervisorJob())

        client = TcpClient(
            onMessage = { message -> handleClientMessage(message) },
            onDisconnected = {
                if (_state.value !is SyncState.Completed) {
                    _state.value = SyncState.Error("Disconnected from host")
                }
            },
            onError = { error ->
                _state.value = SyncState.Error(error)
            }
        )

        _state.value = SyncState.Connecting(host, port)

        syncScope?.launch {
            val connected = client!!.connect(host, port)
            if (connected) {
                client?.sendMessage(SyncMessage.TempCodeAuth(tempCode))
            } else {
                _state.value = SyncState.Error("Failed to connect")
            }
        }
    }

    fun connectWithToken(host: String, port: Int, token: String) {
        syncScope?.cancel()
        syncScope = CoroutineScope(Dispatchers.IO + SupervisorJob())

        client = TcpClient(
            onMessage = { message -> handleClientMessage(message) },
            onDisconnected = {
                if (_state.value !is SyncState.Completed) {
                    _state.value = SyncState.Error("Disconnected from host")
                }
            },
            onError = { error ->
                _state.value = SyncState.Error(error)
            }
        )

        _state.value = SyncState.Connecting(host, port)

        syncScope?.launch {
            val connected = client!!.connect(host, port)
            if (connected) {
                val passwordHash = hashPasswordForAuth(token)
                client?.sendMessage(SyncMessage.AuthRequest("qr_user", passwordHash))
            } else {
                _state.value = SyncState.Error("Failed to connect")
            }
        }
    }

    private fun handleClientMessage(message: SyncMessage) {
        when (message) {
            is SyncMessage.AuthResponse -> {
                if (!message.success) {
                    _state.value = SyncState.Error(message.message)
                    client?.disconnect()
                }
            }

            is SyncMessage.DeviceInfo -> {
                _state.value = SyncState.Selecting(message.trackCount, message.playlistCount)
            }

            is SyncMessage.TrackList -> {
                syncScope?.launch {
                    receiveTrackList(message.tracks)
                }
            }

            is SyncMessage.PlaylistList -> {
                syncScope?.launch {
                    receivePlaylistList(message.playlists)
                }
            }

            is SyncMessage.FileChunk -> {
                syncScope?.launch {
                    receiveFileChunk(message)
                }
            }

            is SyncMessage.SyncComplete -> {
                _state.value = SyncState.Completed(message.message)
            }

            is SyncMessage.SyncProgress -> {
                _progress.value = if (message.total > 0) message.current.toFloat() / message.total.toFloat() else 0f
            }

            else -> {}
        }
    }

    fun requestSync(includeTracks: Boolean, includeMetadata: Boolean, includePlaylists: Boolean, includeThumbnails: Boolean) {
        client?.sendMessage(SyncMessage.SyncRequest(
            includeTracks = includeTracks,
            includeMetadata = includeMetadata,
            includePlaylists = includePlaylists,
            includeThumbnails = includeThumbnails
        ))
        _state.value = SyncState.Syncing(0, 0, "Starting...", 0, 0)
    }

    private suspend fun receiveTrackList(tracks: List<SyncTrack>) {
        for (syncTrack in tracks) {
            val existing = trackRepository.getTrackById(syncTrack.id)
            if (existing == null) {
                trackRepository.insertTrack(Track(
                    id = syncTrack.id,
                    title = syncTrack.title,
                    artist = syncTrack.artist,
                    album = syncTrack.album,
                    genre = syncTrack.genre,
                    year = syncTrack.year,
                    trackNumber = syncTrack.trackNumber,
                    duration = syncTrack.duration,
                    filePath = syncTrack.filePath,
                    format = com.auralis.model.AudioFormat.fromExtension(syncTrack.format),
                    size = syncTrack.size
                ))
            }
        }
    }

    private suspend fun receivePlaylistList(playlists: List<SyncPlaylist>) {
        for (syncPlaylist in playlists) {
            val existing = playlistRepository.getPlaylistById(syncPlaylist.id)
            if (existing == null) {
                playlistRepository.insertPlaylist(Playlist(
                    id = syncPlaylist.id,
                    name = syncPlaylist.name,
                    description = syncPlaylist.description
                ))
            }
        }
    }

    private suspend fun receiveFileChunk(chunk: SyncMessage.FileChunk) {
        withContext(Dispatchers.IO) {
            val dir = File(storageDir, "sync")
            dir.mkdirs()

            val file = File(dir, chunk.fileName)
            if (chunk.chunkIndex == 0) {
                file.delete()
            }

            val data = Base64Util.decode(chunk.data)
            FileOutputStream(file, true).use { fos ->
                fos.write(data)
            }

            if (chunk.chunkIndex == chunk.totalChunks - 1) {
                val digest = MessageDigest.getInstance("SHA-256")
                val fileBytes = file.readBytes()
                val checksum = Base64Util.encode(digest.digest(fileBytes))

                if (checksum == chunk.checksum) {
                    val destFile = File(storageDir, "music/${chunk.fileName}")
                    destFile.parentFile?.mkdirs()
                    file.renameTo(destFile)
                }
            }
        }
    }

    fun createAccount(username: String, password: String) {
        syncScope?.launch {
            accountManager.createAccount(username, password)
        }
    }

    fun login(username: String, password: String) {
        syncScope?.launch {
            accountManager.login(username, password)
        }
    }

    fun stop() {
        discovery.stop()
        server?.stop()
        server = null
        client?.destroy()
        client = null
        syncScope?.cancel()
        syncScope = null
        _state.value = SyncState.Idle
        _progress.value = 0f
    }

    private fun hashPasswordForAuth(password: String): String {
        val salt = "sync_auth_salt"
        val saltBytes = salt.toByteArray(Charsets.UTF_8)
        val passwordBytes = password.toByteArray(Charsets.UTF_8)
        val combined = saltBytes + passwordBytes
        val digest = MessageDigest.getInstance("SHA-256")
        return Base64Util.encode(digest.digest(combined))
    }
}
