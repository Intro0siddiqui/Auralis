package com.auralis.sync

import kotlinx.coroutines.*
import java.io.DataInputStream
import java.io.DataOutputStream
import java.net.Socket
import java.util.concurrent.atomic.AtomicBoolean

class TcpClient(
    private val onMessage: (SyncMessage) -> Unit,
    private val onDisconnected: () -> Unit,
    private val onError: (String) -> Unit
) {
    private var socket: Socket? = null
    private var input: DataInputStream? = null
    private var output: DataOutputStream? = null
    private var readJob: Job? = null
    private val scope = CoroutineScope(Dispatchers.IO + SupervisorJob())
    private val connected = AtomicBoolean(false)

    val isConnected: Boolean get() = connected.get()

    suspend fun connect(host: String, port: Int): Boolean {
        return try {
            socket = Socket(host, port)
            input = DataInputStream(socket!!.getInputStream())
            output = DataOutputStream(socket!!.getOutputStream())
            connected.set(true)

            readJob = scope.launch {
                try {
                    while (isActive && connected.get()) {
                        val message = SyncProtocol.readMessage(input!!) ?: break
                        onMessage(message)
                    }
                } catch (e: Exception) {
                    if (connected.get()) {
                        onError("Read error: ${e.message}")
                    }
                } finally {
                    disconnect()
                }
            }

            true
        } catch (e: Exception) {
            onError("Connection failed: ${e.message}")
            false
        }
    }

    fun sendMessage(message: SyncMessage): Boolean {
        if (!connected.get()) return false
        val out = output ?: return false
        return try {
            synchronized(out) {
                SyncProtocol.writeMessage(out, message)
            }
            true
        } catch (e: Exception) {
            onError("Send error: ${e.message}")
            false
        }
    }

    fun sendFileChunk(fileName: String, chunkIndex: Int, totalChunks: Int, data: ByteArray, checksum: String): Boolean {
        if (!connected.get()) return false
        val out = output ?: return false
        return try {
            val encoded = Base64Util.encode(data)
            val chunk = SyncMessage.FileChunk(fileName, chunkIndex, totalChunks, encoded, checksum)
            synchronized(out) {
                SyncProtocol.writeMessage(out, chunk)
            }
            true
        } catch (e: Exception) {
            onError("File chunk error: ${e.message}")
            false
        }
    }

    fun disconnect() {
        if (!connected.compareAndSet(true, false)) return
        readJob?.cancel()
        try { socket?.close() } catch (_: Exception) {}
        socket = null
        input = null
        output = null
        onDisconnected()
    }

    fun destroy() {
        disconnect()
        scope.cancel()
    }
}
