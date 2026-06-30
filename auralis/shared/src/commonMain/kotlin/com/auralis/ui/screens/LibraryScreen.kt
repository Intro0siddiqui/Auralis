package com.auralis.ui.screens

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.auralis.model.Track
import com.auralis.service.MusicScanner
import com.auralis.service.ScanResult
import com.auralis.ui.components.TrackItem
import com.auralis.ui.icons.FolderOpenIcon
import com.auralis.ui.icons.ScanIcon

@Composable
fun LibraryScreen(
    tracks: List<Track> = emptyList(),
    onTrackClick: (Track) -> Unit = {},
    onPickFolder: () -> Unit = {},
    onScanSystem: () -> Unit = {},
    scanState: MusicScanner.ScanState = MusicScanner.ScanState.Idle,
    scanResult: ScanResult? = null
) {
    var showScanResult by remember { mutableStateOf(false) }

    LaunchedEffect(scanResult) {
        if (scanResult != null) showScanResult = true
    }

    Column(modifier = Modifier.fillMaxSize()) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 8.dp),
            horizontalArrangement = Arrangement.spacedBy(8.dp)
        ) {
            OutlinedButton(
                onClick = onPickFolder,
                modifier = Modifier.weight(1f),
                enabled = scanState !is MusicScanner.ScanState.Scanning
            ) {
                Icon(FolderOpenIcon, contentDescription = null, modifier = Modifier.size(18.dp))
                Spacer(modifier = Modifier.width(6.dp))
                Text("Import Folder")
            }
            OutlinedButton(
                onClick = onScanSystem,
                modifier = Modifier.weight(1f),
                enabled = scanState !is MusicScanner.ScanState.Scanning
            ) {
                Icon(ScanIcon, contentDescription = null, modifier = Modifier.size(18.dp))
                Spacer(modifier = Modifier.width(6.dp))
                Text("Scan System")
            }
        }

        when (scanState) {
            is MusicScanner.ScanState.Scanning -> {
                Card(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 16.dp, vertical = 4.dp),
                    colors = CardDefaults.cardColors(
                        containerColor = MaterialTheme.colorScheme.primaryContainer
                    )
                ) {
                    Row(
                        modifier = Modifier.padding(12.dp),
                        verticalAlignment = Alignment.CenterVertically
                    ) {
                        CircularProgressIndicator(
                            modifier = Modifier.size(20.dp),
                            strokeWidth = 2.dp
                        )
                        Spacer(modifier = Modifier.width(12.dp))
                        Column {
                            Text(
                                text = "Scanning... ${scanState.found} files found",
                                style = MaterialTheme.typography.bodyMedium
                            )
                            if (scanState.currentPath.isNotEmpty()) {
                                Text(
                                    text = scanState.currentPath,
                                    style = MaterialTheme.typography.bodySmall,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                    maxLines = 1
                                )
                            }
                        }
                    }
                }
            }
            else -> {}
        }

        if (tracks.isEmpty() && scanState !is MusicScanner.ScanState.Scanning) {
            Box(
                modifier = Modifier.fillMaxSize(),
                contentAlignment = Alignment.Center
            ) {
                Column(horizontalAlignment = Alignment.CenterHorizontally) {
                    Text(
                        text = "No music yet",
                        style = MaterialTheme.typography.headlineSmall
                    )
                    Spacer(modifier = Modifier.height(8.dp))
                    Text(
                        text = "Import a folder or scan your system for music",
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                }
            }
        } else {
            LazyColumn {
                items(tracks) { track ->
                    TrackItem(
                        track = track,
                        onClick = { onTrackClick(track) }
                    )
                }
            }
        }
    }

    if (showScanResult && scanResult != null) {
        AlertDialog(
            onDismissRequest = { showScanResult = false },
            title = { Text("Scan Complete") },
            text = {
                Column {
                    Text("Found: ${scanResult.totalFound} audio files")
                    Text("Added: ${scanResult.added} new tracks")
                    Text("Skipped: ${scanResult.skipped} (already in library)")
                    if (scanResult.errors.isNotEmpty()) {
                        Spacer(modifier = Modifier.height(8.dp))
                        Text(
                            "Errors: ${scanResult.errors.size}",
                            color = MaterialTheme.colorScheme.error
                        )
                        scanResult.errors.take(3).forEach { error ->
                            Text(
                                text = error,
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.error
                            )
                        }
                    }
                }
            },
            confirmButton = {
                TextButton(onClick = { showScanResult = false }) {
                    Text("OK")
                }
            }
        )
    }
}
