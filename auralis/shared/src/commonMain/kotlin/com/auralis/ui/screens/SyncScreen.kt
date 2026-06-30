package com.auralis.ui.screens

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.DisposableEffect
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.Canvas
import androidx.compose.ui.graphics.Paint
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import com.auralis.sync.*
import com.auralis.ui.icons.SyncIcon
import com.auralis.ui.icons.WifiIcon

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SyncScreen(
    syncService: SyncService?,
    onNavigateToAccount: () -> Unit = {}
) {
    val state by syncService?.state?.collectAsState() ?: remember { mutableStateOf<SyncState>(SyncState.Idle) }
    val devices by syncService?.discoveredDevices?.collectAsState() ?: remember { mutableStateOf<Map<String, DiscoveredDevice>>(emptyMap()) }
    val progress by syncService?.progress?.collectAsState() ?: remember { mutableStateOf(0f) }

    var showAccountDialog by remember { mutableStateOf(false) }
    var showTempCodeDialog by remember { mutableStateOf(false) }
    var showQrScanner by remember { mutableStateOf(false) }
    var includeTracks by remember { mutableStateOf(true) }
    var includeMetadata by remember { mutableStateOf(true) }
    var includePlaylists by remember { mutableStateOf(true) }
    var includeThumbnails by remember { mutableStateOf(false) }

    LaunchedEffect(Unit) {
        syncService?.startDiscovery()
    }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(16.dp)
    ) {
        Text(
            text = "Sync Library",
            style = MaterialTheme.typography.headlineMedium
        )

        Spacer(modifier = Modifier.height(16.dp))

        when (val currentState = state) {
            is SyncState.Idle -> {
                IdleSyncView(
                    onStartHost = { syncService?.startHosting("Desktop") },
                    onConnect = { _, _ ->
                        showAccountDialog = true
                    },
                    onConnectWithTempCode = {
                        showTempCodeDialog = true
                    },
                    onScanQr = {
                        showQrScanner = true
                    },
                    devices = devices
                )
            }

            is SyncState.Hosting -> {
                HostingView(
                    port = currentState.port,
                    token = currentState.token,
                    ip = currentState.ip,
                    tempCode = currentState.tempCode,
                    onStop = { syncService?.stop() }
                )
            }

            is SyncState.Connecting -> {
                ConnectingView(host = currentState.host, port = currentState.port)
            }

            is SyncState.Authenticating -> {
                AuthenticatingView()
            }

            is SyncState.Selecting -> {
                SyncSelectionView(
                    availableTracks = currentState.availableTracks,
                    availablePlaylists = currentState.availablePlaylists,
                    includeTracks = includeTracks,
                    includeMetadata = includeMetadata,
                    includePlaylists = includePlaylists,
                    includeThumbnails = includeThumbnails,
                    onTracksChange = { includeTracks = it },
                    onMetadataChange = { includeMetadata = it },
                    onPlaylistsChange = { includePlaylists = it },
                    onThumbnailsChange = { includeThumbnails = it },
                    onStartSync = {
                        syncService?.requestSync(
                            includeTracks, includeMetadata, includePlaylists, includeThumbnails
                        )
                    },
                    onCancel = { syncService?.stop() }
                )
            }

            is SyncState.Syncing -> {
                SyncProgressView(
                    totalFiles = currentState.totalFiles,
                    completedFiles = currentState.completedFiles,
                    currentFile = currentState.currentFile,
                    bytesTransferred = currentState.bytesTransferred,
                    totalBytes = currentState.totalBytes,
                    progress = progress
                )
            }

            is SyncState.Completed -> {
                CompletedView(
                    message = currentState.message,
                    onDone = { syncService?.stop() }
                )
            }

            is SyncState.Error -> {
                ErrorView(
                    message = currentState.message,
                    onRetry = { syncService?.stop() },
                    onDismiss = { syncService?.stop() }
                )
            }
        }
    }

    if (showAccountDialog) {
        AccountDialog(
            onDismiss = { showAccountDialog = false },
            onLogin = { _, _ ->
                showAccountDialog = false
            }
        )
    }

    if (showTempCodeDialog) {
        TempCodeDialog(
            onDismiss = { showTempCodeDialog = false },
            devices = devices,
            onConnect = { device, code ->
                showTempCodeDialog = false
                syncService?.connectWithTempCode(device.host, device.port, code)
            }
        )
    }

    if (showQrScanner) {
        QrScanDialog(
            onDismiss = { showQrScanner = false },
            onQrScanned = { qrContent ->
                showQrScanner = false
                val connectionInfo = QrCodeManager.ConnectionInfo.fromJson(qrContent)
                if (connectionInfo != null) {
                    syncService?.connectWithToken(connectionInfo.host, connectionInfo.port, connectionInfo.token)
                }
            }
        )
    }
}

@Composable
private fun IdleSyncView(
    onStartHost: () -> Unit,
    onConnect: (String, Int) -> Unit,
    onConnectWithTempCode: () -> Unit,
    onScanQr: () -> Unit,
    devices: Map<String, DiscoveredDevice>
) {
    Column(
        modifier = Modifier.fillMaxWidth(),
        horizontalAlignment = Alignment.CenterHorizontally
    ) {
        Text(
            text = "Share your music library with other Auralis devices",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center
        )

        Spacer(modifier = Modifier.height(24.dp))

        Button(
            onClick = onStartHost,
            modifier = Modifier.fillMaxWidth()
        ) {
            Icon(SyncIcon, contentDescription = null)
            Spacer(modifier = Modifier.width(8.dp))
            Text("Host Sync Session")
        }

        Spacer(modifier = Modifier.height(12.dp))

        OutlinedButton(
            onClick = onScanQr,
            modifier = Modifier.fillMaxWidth()
        ) {
            Icon(WifiIcon, contentDescription = null)
            Spacer(modifier = Modifier.width(8.dp))
            Text("Scan QR Code to Connect")
        }

        Spacer(modifier = Modifier.height(12.dp))

        OutlinedButton(
            onClick = { onConnect("", 0) },
            modifier = Modifier.fillMaxWidth()
        ) {
            Icon(WifiIcon, contentDescription = null)
            Spacer(modifier = Modifier.width(8.dp))
            Text("Connect to Discovered Device")
        }

        Spacer(modifier = Modifier.height(12.dp))

        OutlinedButton(
            onClick = onConnectWithTempCode,
            modifier = Modifier.fillMaxWidth()
        ) {
            Icon(WifiIcon, contentDescription = null)
            Spacer(modifier = Modifier.width(8.dp))
            Text("Connect with Temp Code")
        }

        if (devices.isNotEmpty()) {
            Spacer(modifier = Modifier.height(24.dp))

            Text(
                text = "Discovered Devices",
                style = MaterialTheme.typography.titleMedium
            )

            Spacer(modifier = Modifier.height(8.dp))

            LazyColumn {
                items(devices.values.toList()) { device ->
                    DeviceCard(
                        device = device,
                        onConnect = { onConnect(device.host, device.port) }
                    )
                }
            }
        }
    }
}

@Composable
private fun DeviceCard(
    device: DiscoveredDevice,
    onConnect: () -> Unit
) {
    Card(
        modifier = Modifier
            .fillMaxWidth()
            .padding(vertical = 4.dp),
        onClick = onConnect
    ) {
        Row(
            modifier = Modifier
                .padding(16.dp)
                .fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically
        ) {
            Column(modifier = Modifier.weight(1f)) {
                Text(
                    text = device.name,
                    style = MaterialTheme.typography.bodyLarge
                )
                Text(
                    text = "${device.trackCount} tracks, ${device.playlistCount} playlists",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
            }

            Icon(
                WifiIcon,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.primary
            )
        }
    }
}

@Composable
private fun HostingView(
    port: Int,
    token: String,
    ip: String,
    tempCode: String,
    onStop: () -> Unit
) {
    Column(
        modifier = Modifier.fillMaxWidth(),
        horizontalAlignment = Alignment.CenterHorizontally
    ) {
        Text(
            text = "Sync session active",
            style = MaterialTheme.typography.titleLarge,
            color = MaterialTheme.colorScheme.primary
        )

        Spacer(modifier = Modifier.height(16.dp))

        Card(
            modifier = Modifier.fillMaxWidth()
        ) {
            Column(
                modifier = Modifier.padding(16.dp)
            ) {
                Text(
                    text = "Connection Info",
                    style = MaterialTheme.typography.titleMedium
                )

                Spacer(modifier = Modifier.height(8.dp))

                Text(
                    text = "IP: $ip",
                    style = MaterialTheme.typography.bodyMedium
                )
                Text(
                    text = "Port: $port",
                    style = MaterialTheme.typography.bodyMedium
                )
                Text(
                    text = "Token: $token",
                    style = MaterialTheme.typography.bodyMedium
                )

                Spacer(modifier = Modifier.height(12.dp))

                val qrContent = QrCodeManager.ConnectionInfo(ip, port, token).toJson()
                val qrMatrix = remember(qrContent) { QrCodeManager.generateQrMatrix(qrContent) }

                Text(
                    text = "Scan this QR code to connect:",
                    style = MaterialTheme.typography.bodyMedium
                )

                Spacer(modifier = Modifier.height(8.dp))

                QrCodeDisplay(matrix = qrMatrix, size = 200)

                Spacer(modifier = Modifier.height(16.dp))

                Divider()

                Spacer(modifier = Modifier.height(16.dp))

                Text(
                    text = "Or use temp code (for devices without camera):",
                    style = MaterialTheme.typography.bodyMedium
                )

                Spacer(modifier = Modifier.height(8.dp))

                Text(
                    text = tempCode,
                    style = MaterialTheme.typography.headlineLarge,
                    color = MaterialTheme.colorScheme.primary
                )

                Spacer(modifier = Modifier.height(4.dp))

                Text(
                    text = "Enter this code on the other device",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
            }
        }

        Spacer(modifier = Modifier.height(16.dp))

        Text(
            text = "Waiting for devices to connect...",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant
        )

        Spacer(modifier = Modifier.height(24.dp))

        OutlinedButton(
            onClick = onStop,
            modifier = Modifier.fillMaxWidth(),
            colors = ButtonDefaults.outlinedButtonColors(
                contentColor = MaterialTheme.colorScheme.error
            )
        ) {
            Text("Stop Hosting")
        }
    }
}

@Composable
private fun QrCodeDisplay(matrix: List<List<Boolean>>, size: Int) {
    val bitmap = remember(matrix) {
        val moduleSize = size / matrix.size
        val imageBitmap = ImageBitmap(size, size)
        val canvas = Canvas(imageBitmap)

        val bgPaint = Paint().apply { color = Color.White }
        val fgPaint = Paint().apply { color = Color.Black }

        canvas.drawRect(Rect(0f, 0f, size.toFloat(), size.toFloat()), bgPaint)

        for (row in matrix.indices) {
            for (col in matrix[row].indices) {
                if (matrix[row][col]) {
                    canvas.drawRect(
                        Rect(
                            col * moduleSize.toFloat(),
                            row * moduleSize.toFloat(),
                            (col + 1) * moduleSize.toFloat(),
                            (row + 1) * moduleSize.toFloat()
                        ),
                        fgPaint
                    )
                }
            }
        }

        imageBitmap
    }

    Image(
        bitmap = bitmap,
        contentDescription = "QR Code",
        modifier = Modifier.size(size.dp)
    )
}

@Composable
private fun ConnectingView(host: String, port: Int) {
    Column(
        modifier = Modifier.fillMaxWidth(),
        horizontalAlignment = Alignment.CenterHorizontally
    ) {
        CircularProgressIndicator()

        Spacer(modifier = Modifier.height(16.dp))

        Text(
            text = "Connecting to $host:$port...",
            style = MaterialTheme.typography.bodyLarge
        )
    }
}

@Composable
private fun AuthenticatingView() {
    Column(
        modifier = Modifier.fillMaxWidth(),
        horizontalAlignment = Alignment.CenterHorizontally
    ) {
        CircularProgressIndicator()

        Spacer(modifier = Modifier.height(16.dp))

        Text(
            text = "Authenticating...",
            style = MaterialTheme.typography.bodyLarge
        )
    }
}

@Composable
private fun SyncSelectionView(
    availableTracks: Int,
    availablePlaylists: Int,
    includeTracks: Boolean,
    includeMetadata: Boolean,
    includePlaylists: Boolean,
    includeThumbnails: Boolean,
    onTracksChange: (Boolean) -> Unit,
    onMetadataChange: (Boolean) -> Unit,
    onPlaylistsChange: (Boolean) -> Unit,
    onThumbnailsChange: (Boolean) -> Unit,
    onStartSync: () -> Unit,
    onCancel: () -> Unit
) {
    Column(
        modifier = Modifier.fillMaxWidth()
    ) {
        Text(
            text = "Select what to sync",
            style = MaterialTheme.typography.titleMedium
        )

        Spacer(modifier = Modifier.height(16.dp))

        Text(
            text = "Available: $availableTracks tracks, $availablePlaylists playlists",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant
        )

        Spacer(modifier = Modifier.height(16.dp))

        CheckboxItem(
            checked = includeTracks,
            onCheckedChange = onTracksChange,
            label = "Audio Files",
            description = "Transfer actual music files"
        )

        CheckboxItem(
            checked = includeMetadata,
            onCheckedChange = onMetadataChange,
            label = "Metadata",
            description = "Track info, artist, album, etc."
        )

        CheckboxItem(
            checked = includePlaylists,
            onCheckedChange = onPlaylistsChange,
            label = "Playlists",
            description = "Playlist structure and associations"
        )

        CheckboxItem(
            checked = includeThumbnails,
            onCheckedChange = onThumbnailsChange,
            label = "Thumbnails",
            description = "Album art and track thumbnails"
        )

        Spacer(modifier = Modifier.height(24.dp))

        Button(
            onClick = onStartSync,
            modifier = Modifier.fillMaxWidth(),
            enabled = includeTracks || includeMetadata || includePlaylists || includeThumbnails
        ) {
            Icon(SyncIcon, contentDescription = null)
            Spacer(modifier = Modifier.width(8.dp))
            Text("Start Sync")
        }

        Spacer(modifier = Modifier.height(8.dp))

        OutlinedButton(
            onClick = onCancel,
            modifier = Modifier.fillMaxWidth()
        ) {
            Text("Cancel")
        }
    }
}

@Composable
private fun CheckboxItem(
    checked: Boolean,
    onCheckedChange: (Boolean) -> Unit,
    label: String,
    description: String
) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(vertical = 4.dp),
        verticalAlignment = Alignment.CenterVertically
    ) {
        Checkbox(
            checked = checked,
            onCheckedChange = onCheckedChange
        )

        Column(modifier = Modifier.padding(start = 8.dp)) {
            Text(
                text = label,
                style = MaterialTheme.typography.bodyLarge
            )
            Text(
                text = description,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
        }
    }
}

@Composable
private fun SyncProgressView(
    totalFiles: Int,
    completedFiles: Int,
    currentFile: String,
    bytesTransferred: Long,
    totalBytes: Long,
    progress: Float
) {
    Column(
        modifier = Modifier.fillMaxWidth(),
        horizontalAlignment = Alignment.CenterHorizontally
    ) {
        LinearProgressIndicator(
            progress = { progress },
            modifier = Modifier.fillMaxWidth()
        )

        Spacer(modifier = Modifier.height(16.dp))

        Text(
            text = "Syncing: $completedFiles / $totalFiles files",
            style = MaterialTheme.typography.titleMedium
        )

        Spacer(modifier = Modifier.height(8.dp))

        Text(
            text = currentFile,
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant
        )

        Spacer(modifier = Modifier.height(8.dp))

        Text(
            text = "${formatBytes(bytesTransferred)} / ${formatBytes(totalBytes)}",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant
        )
    }
}

@Composable
private fun CompletedView(
    message: String,
    onDone: () -> Unit
) {
    Column(
        modifier = Modifier.fillMaxWidth(),
        horizontalAlignment = Alignment.CenterHorizontally
    ) {
        Text(
            text = "Sync Complete",
            style = MaterialTheme.typography.headlineMedium,
            color = MaterialTheme.colorScheme.primary
        )

        Spacer(modifier = Modifier.height(16.dp))

        Text(
            text = message,
            style = MaterialTheme.typography.bodyLarge,
            textAlign = TextAlign.Center
        )

        Spacer(modifier = Modifier.height(24.dp))

        Button(
            onClick = onDone,
            modifier = Modifier.fillMaxWidth()
        ) {
            Text("Done")
        }
    }
}

@Composable
private fun ErrorView(
    message: String,
    onRetry: () -> Unit,
    onDismiss: () -> Unit
) {
    Column(
        modifier = Modifier.fillMaxWidth(),
        horizontalAlignment = Alignment.CenterHorizontally
    ) {
        Text(
            text = "Sync Error",
            style = MaterialTheme.typography.headlineMedium,
            color = MaterialTheme.colorScheme.error
        )

        Spacer(modifier = Modifier.height(16.dp))

        Text(
            text = message,
            style = MaterialTheme.typography.bodyLarge,
            textAlign = TextAlign.Center
        )

        Spacer(modifier = Modifier.height(24.dp))

        Button(
            onClick = onRetry,
            modifier = Modifier.fillMaxWidth()
        ) {
            Text("Try Again")
        }

        Spacer(modifier = Modifier.height(8.dp))

        OutlinedButton(
            onClick = onDismiss,
            modifier = Modifier.fillMaxWidth()
        ) {
            Text("Dismiss")
        }
    }
}

@Composable
private fun AccountDialog(
    onDismiss: () -> Unit,
    onLogin: (String, String) -> Unit
) {
    var username by remember { mutableStateOf("") }
    var password by remember { mutableStateOf("") }

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Login to Sync") },
        text = {
            Column {
                OutlinedTextField(
                    value = username,
                    onValueChange = { username = it },
                    label = { Text("Username") },
                    modifier = Modifier.fillMaxWidth()
                )

                Spacer(modifier = Modifier.height(8.dp))

                OutlinedTextField(
                    value = password,
                    onValueChange = { password = it },
                    label = { Text("Password") },
                    modifier = Modifier.fillMaxWidth(),
                    visualTransformation = androidx.compose.ui.text.input.PasswordVisualTransformation()
                )
            }
        },
        confirmButton = {
            TextButton(
                onClick = { onLogin(username, password) },
                enabled = username.isNotBlank() && password.isNotBlank()
            ) {
                Text("Connect")
            }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) {
                Text("Cancel")
            }
        }
    )
}

@Composable
private fun TempCodeDialog(
    onDismiss: () -> Unit,
    devices: Map<String, DiscoveredDevice>,
    onConnect: (DiscoveredDevice, String) -> Unit
) {
    var tempCode by remember { mutableStateOf("") }
    var selectedDevice by remember { mutableStateOf<DiscoveredDevice?>(null) }

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Connect with Temp Code") },
        text = {
            Column {
                Text(
                    text = "Select the host device and enter the 6-digit code shown on it:",
                    style = MaterialTheme.typography.bodyMedium
                )

                Spacer(modifier = Modifier.height(16.dp))

                if (devices.isNotEmpty()) {
                    Text(
                        text = "Available Devices",
                        style = MaterialTheme.typography.titleSmall
                    )
                    Spacer(modifier = Modifier.height(8.dp))

                    devices.values.forEach { device ->
                        Row(
                            modifier = Modifier
                                .fillMaxWidth()
                                .padding(vertical = 4.dp),
                            verticalAlignment = Alignment.CenterVertically
                        ) {
                            RadioButton(
                                selected = selectedDevice?.id == device.id,
                                onClick = { selectedDevice = device }
                            )
                            Spacer(modifier = Modifier.width(8.dp))
                            Column {
                                Text(
                                    text = device.name,
                                    style = MaterialTheme.typography.bodyMedium
                                )
                                Text(
                                    text = "${device.trackCount} tracks",
                                    style = MaterialTheme.typography.bodySmall,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant
                                )
                            }
                        }
                    }
                } else {
                    Text(
                        text = "No devices found. Make sure the host is broadcasting.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                }

                Spacer(modifier = Modifier.height(16.dp))

                OutlinedTextField(
                    value = tempCode,
                    onValueChange = { if (it.length <= 6) tempCode = it.uppercase() },
                    label = { Text("6-Digit Temp Code") },
                    modifier = Modifier.fillMaxWidth(),
                    singleLine = true
                )
            }
        },
        confirmButton = {
            TextButton(
                onClick = {
                    selectedDevice?.let { device ->
                        onConnect(device, tempCode)
                    }
                },
                enabled = tempCode.length == 6 && selectedDevice != null
            ) {
                Text("Connect")
            }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) {
                Text("Cancel")
            }
        }
    )
}

@Composable
private fun QrScanDialog(
    onDismiss: () -> Unit,
    onQrScanned: (String) -> Unit
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Scan QR Code") },
        text = {
            QrScannerView(
                onQrScanned = onQrScanned,
                onDismiss = onDismiss
            )
        },
        confirmButton = {},
        dismissButton = {
            TextButton(onClick = onDismiss) {
                Text("Cancel")
            }
        }
    )
}

private fun formatBytes(bytes: Long): String {
    return when {
        bytes < 1024 -> "$bytes B"
        bytes < 1024 * 1024 -> "${bytes / 1024} KB"
        bytes < 1024 * 1024 * 1024 -> "%.1f MB".format(bytes / (1024.0 * 1024.0))
        else -> "%.2f GB".format(bytes / (1024.0 * 1024.0 * 1024.0))
    }
}
