package com.auralis.sync

import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import java.net.DatagramPacket
import java.net.DatagramSocket
import java.net.InetAddress
import java.net.MulticastSocket
import java.net.NetworkInterface
import java.util.concurrent.ConcurrentHashMap

data class DiscoveredDevice(
    val id: String,
    val name: String,
    val host: String,
    val port: Int,
    val trackCount: Int = 0,
    val playlistCount: Int = 0,
    val lastSeen: Long = System.currentTimeMillis()
)

class DeviceDiscovery {
    private val _devices = MutableStateFlow<Map<String, DiscoveredDevice>>(emptyMap())
    val devices: StateFlow<Map<String, DiscoveredDevice>> = _devices.asStateFlow()

    private var multicastSocket: MulticastSocket? = null
    private var discoveryJob: Job? = null
    private val scope = CoroutineScope(Dispatchers.IO + SupervisorJob())

    companion object {
        private const val MULTICAST_GROUP = "239.255.0.1"
        private const val DISCOVERY_PORT = 51234
        private const val SERVICE_TYPE = "_auralis._tcp."
        private const val DEVICE_TIMEOUT = 30000L
    }

    fun startHosting(deviceName: String, port: Int, trackCount: Int, playlistCount: Int) {
        discoveryJob?.cancel()
        discoveryJob = scope.launch {
            try {
                val socket = DatagramSocket()
                socket.soTimeout = 1000

                val heartbeat = launch {
                    while (isActive) {
                        try {
                            val response = "$SERVICE_TYPE|$deviceName|$port|$trackCount|$playlistCount"
                            val data = response.toByteArray()
                            val packet = DatagramPacket(
                                data, data.size,
                                InetAddress.getByName(MULTICAST_GROUP),
                                DISCOVERY_PORT
                            )
                            socket.send(packet)
                        } catch (_: Exception) {}
                        delay(2000)
                    }
                }

                val cleanup = launch {
                    while (isActive) {
                        delay(DEVICE_TIMEOUT / 2)
                        val now = System.currentTimeMillis()
                        _devices.value = _devices.value.filter {
                            now - it.value.lastSeen < DEVICE_TIMEOUT
                        }
                    }
                }

                val listenBuffer = ByteArray(1024)
                while (isActive) {
                    try {
                        val packet = DatagramPacket(listenBuffer, listenBuffer.size)
                        socket.receive(packet)
                        val message = String(packet.data, 0, packet.length)
                        val parts = message.split("|")
                        if (parts.size >= 5 && parts[0] == SERVICE_TYPE) {
                            val device = DiscoveredDevice(
                                id = "${packet.address.hostAddress}:${parts[2]}",
                                name = parts[1],
                                host = packet.address.hostAddress,
                                port = parts[2].toIntOrNull() ?: continue,
                                trackCount = parts[3].toIntOrNull() ?: 0,
                                playlistCount = parts[4].toIntOrNull() ?: 0
                            )
                            _devices.value = _devices.value + (device.id to device)
                        }
                    } catch (_: java.net.SocketTimeoutException) {}
                }

                heartbeat.join()
                cleanup.join()
            } catch (e: Exception) {
                // Discovery failed
            }
        }
    }

    fun startScanning() {
        discoveryJob?.cancel()
        discoveryJob = scope.launch {
            try {
                val group = InetAddress.getByName(MULTICAST_GROUP)
                val socket = MulticastSocket(DISCOVERY_PORT)
                socket.joinGroup(group)
                socket.soTimeout = 1000

                multicastSocket = socket

                val cleanup = launch {
                    while (isActive) {
                        delay(DEVICE_TIMEOUT / 2)
                        val now = System.currentTimeMillis()
                        _devices.value = _devices.value.filter {
                            now - it.value.lastSeen < DEVICE_TIMEOUT
                        }
                    }
                }

                val buffer = ByteArray(1024)
                while (isActive) {
                    try {
                        val packet = DatagramPacket(buffer, buffer.size)
                        socket.receive(packet)
                        val message = String(packet.data, 0, packet.length)
                        val parts = message.split("|")
                        if (parts.size >= 5 && parts[0] == SERVICE_TYPE) {
                            val device = DiscoveredDevice(
                                id = "${packet.address.hostAddress}:${parts[2]}",
                                name = parts[1],
                                host = packet.address.hostAddress,
                                port = parts[2].toIntOrNull() ?: continue,
                                trackCount = parts[3].toIntOrNull() ?: 0,
                                playlistCount = parts[4].toIntOrNull() ?: 0
                            )
                            _devices.value = _devices.value + (device.id to device)
                        }
                    } catch (_: java.net.SocketTimeoutException) {}
                }

                cleanup.join()
            } catch (e: Exception) {
                // Scan failed
            }
        }
    }

    fun stop() {
        discoveryJob?.cancel()
        discoveryJob = null
        try { multicastSocket?.leaveGroup(InetAddress.getByName(MULTICAST_GROUP)) } catch (_: Exception) {}
        try { multicastSocket?.close() } catch (_: Exception) {}
        multicastSocket = null
        _devices.value = emptyMap()
    }

    fun destroy() {
        stop()
        scope.cancel()
    }

    fun getLocalIpAddress(): String? {
        return try {
            NetworkInterface.getNetworkInterfaces().toList()
                .flatMap { it.inetAddresses.toList() }
                .firstOrNull { !it.isLoopbackAddress && it is java.net.Inet4Address }
                ?.hostAddress
        } catch (_: Exception) {
            null
        }
    }
}
