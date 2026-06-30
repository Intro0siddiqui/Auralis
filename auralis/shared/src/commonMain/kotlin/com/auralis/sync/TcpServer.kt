package com.auralis.sync

import kotlinx.coroutines.*
import java.io.DataInputStream
import java.io.DataOutputStream
import java.net.ServerSocket
import java.net.Socket
import java.util.concurrent.ConcurrentHashMap

class TcpServer(
    private val onClientConnected: (String, Int) -> Unit,
    private val onClientDisconnected: (String) -> Unit,
    private val onMessage: (String, SyncMessage) -> Unit,
    private val onError: (String) -> Unit
) {
    private var serverSocket: ServerSocket? = null
    private var serverJob: Job? = null
    private val clients = ConcurrentHashMap<String, ClientConnection>()
    private val scope = CoroutineScope(Dispatchers.IO + SupervisorJob())

    val port: Int get() = serverSocket?.localPort ?: -1
    val isRunning: Boolean get() = serverSocket?.isClosed == false

    fun start() {
        if (serverSocket?.isClosed == false) return

        serverSocket = ServerSocket(0)

        serverJob = scope.launch {
            try {
                while (isActive && serverSocket?.isClosed == false) {
                    try {
                        val socket = serverSocket!!.accept()
                        handleClient(socket)
                    } catch (e: Exception) {
                        if (isActive && serverSocket?.isClosed == false) {
                            onError("Accept error: ${e.message}")
                        }
                    }
                }
            } catch (e: Exception) {
                if (isActive) {
                    onError("Server error: ${e.message}")
                }
            }
        }
    }

    private fun handleClient(socket: Socket) {
        scope.launch {
            val clientId = "${socket.inetAddress.hostAddress}:${socket.port}"
            try {
                val input = DataInputStream(socket.getInputStream())
                val output = DataOutputStream(socket.getOutputStream())

                val client = ClientConnection(socket, input, output)
                clients[clientId] = client

                onClientConnected(clientId, socket.port)

                while (isActive && !socket.isClosed) {
                    val message = SyncProtocol.readMessage(input) ?: break
                    onMessage(clientId, message)
                }
            } catch (e: Exception) {
                if (!socket.isClosed) {
                    onError("Client $clientId error: ${e.message}")
                }
            } finally {
                clients.remove(clientId)
                try { socket.close() } catch (_: Exception) {}
                onClientDisconnected(clientId)
            }
        }
    }

    fun sendMessage(clientId: String, message: SyncMessage): Boolean {
        val client = clients[clientId] ?: return false
        return try {
            synchronized(client.output) {
                SyncProtocol.writeMessage(client.output, message)
            }
            true
        } catch (e: Exception) {
            onError("Send error to $clientId: ${e.message}")
            false
        }
    }

    fun broadcastMessage(message: SyncMessage) {
        clients.keys.forEach { sendMessage(it, message) }
    }

    fun sendFileChunk(clientId: String, fileName: String, chunkIndex: Int, totalChunks: Int, data: ByteArray, checksum: String): Boolean {
        val client = clients[clientId] ?: return false
        return try {
            val encoded = Base64Util.encode(data)
            val chunk = SyncMessage.FileChunk(fileName, chunkIndex, totalChunks, encoded, checksum)
            synchronized(client.output) {
                SyncProtocol.writeMessage(client.output, chunk)
            }
            true
        } catch (e: Exception) {
            onError("File chunk error to $clientId: ${e.message}")
            false
        }
    }

    fun stop() {
        serverJob?.cancel()
        clients.values.forEach { client ->
            try { client.socket.close() } catch (_: Exception) {}
        }
        clients.clear()
        try { serverSocket?.close() } catch (_: Exception) {}
        serverSocket = null
        scope.cancel()
    }

    private data class ClientConnection(
        val socket: Socket,
        val input: DataInputStream,
        val output: DataOutputStream
    )
}
