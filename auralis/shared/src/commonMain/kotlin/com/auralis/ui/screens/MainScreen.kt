package com.auralis.ui.screens

import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import com.auralis.database.AuralisDatabase
import com.auralis.model.Track
import com.auralis.repository.TrackRepository
import com.auralis.service.FolderPicker
import com.auralis.service.MusicScanner
import com.auralis.service.ScanResult
import com.auralis.ui.components.BottomPlayer
import com.auralis.ui.icons.DownloadIcon
import com.auralis.ui.icons.LibraryMusicIcon
import com.auralis.ui.icons.QueueMusicIcon
import com.auralis.ui.icons.SearchIcon
import com.auralis.ui.icons.SettingsIcon
import com.auralis.ui.icons.SyncIcon
import com.auralis.sync.SyncService
import kotlinx.coroutines.launch

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun MainScreen(
    database: AuralisDatabase,
    storageDir: java.io.File,
    folderPicker: FolderPicker = FolderPicker()
) {
    var selectedTab by remember { mutableIntStateOf(0) }
    var libraryInitialized by remember { mutableStateOf(false) }
    var playlistsInitialized by remember { mutableStateOf(false) }
    var downloadInitialized by remember { mutableStateOf(false) }
    var settingsInitialized by remember { mutableStateOf(false) }
    var syncInitialized by remember { mutableStateOf(false) }

    val scope = rememberCoroutineScope()

    val trackRepository = remember { TrackRepository(database) }
    val musicScanner = remember { MusicScanner(trackRepository) }
    val syncService = remember { SyncService(database, storageDir) }

    var tracks by remember { mutableStateOf<List<Track>>(emptyList()) }
    var scanState by remember { mutableStateOf<MusicScanner.ScanState>(MusicScanner.ScanState.Idle) }
    var scanResult by remember { mutableStateOf<ScanResult?>(null) }

    LaunchedEffect(Unit) {
        tracks = trackRepository.getAllTracks()
    }

    LaunchedEffect(selectedTab) {
        when (selectedTab) {
            0 -> libraryInitialized = true
            1 -> playlistsInitialized = true
            2 -> downloadInitialized = true
            3 -> settingsInitialized = true
            4 -> syncInitialized = true
        }
    }

    LaunchedEffect(musicScanner.scanState.value) {
        scanState = musicScanner.scanState.value
        if (scanState is MusicScanner.ScanState.Complete) {
            tracks = trackRepository.getAllTracks()
            scanResult = (scanState as MusicScanner.ScanState.Complete).result
        }
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Auralis") },
                actions = {
                    IconButton(onClick = { /* Search */ }) {
                        Icon(SearchIcon, contentDescription = "Search")
                    }
                }
            )
        },
        bottomBar = {
            Column {
                BottomPlayer()
                NavigationBar {
                    NavigationBarItem(
                        icon = { Icon(LibraryMusicIcon, contentDescription = null) },
                        label = { Text("Library") },
                        selected = selectedTab == 0,
                        onClick = { selectedTab = 0 }
                    )
                    NavigationBarItem(
                        icon = { Icon(QueueMusicIcon, contentDescription = null) },
                        label = { Text("Playlists") },
                        selected = selectedTab == 1,
                        onClick = { selectedTab = 1 }
                    )
                    NavigationBarItem(
                        icon = { Icon(DownloadIcon, contentDescription = null) },
                        label = { Text("Downloads") },
                        selected = selectedTab == 2,
                        onClick = { selectedTab = 2 }
                    )
                    NavigationBarItem(
                        icon = { Icon(SyncIcon, contentDescription = null) },
                        label = { Text("Sync") },
                        selected = selectedTab == 4,
                        onClick = { selectedTab = 4 }
                    )
                    NavigationBarItem(
                        icon = { Icon(SettingsIcon, contentDescription = null) },
                        label = { Text("Settings") },
                        selected = selectedTab == 3,
                        onClick = { selectedTab = 3 }
                    )
                }
            }
        }
    ) { paddingValues ->
        Box(modifier = Modifier.padding(paddingValues)) {
            when (selectedTab) {
                0 -> if (libraryInitialized) LibraryScreen(
                    tracks = tracks,
                    onTrackClick = { /* play track */ },
                    onPickFolder = {
                        scope.launch {
                            val path = folderPicker.pickFolder()
                            if (path != null) {
                                val dir = java.io.File(path)
                                if (dir.exists() && dir.isDirectory) {
                                    musicScanner.scanDirectory(dir)
                                }
                            }
                        }
                    },
                    onScanSystem = {
                        scope.launch {
                            val dirs = MusicScanner.defaultScanDirectories()
                            musicScanner.scanDirectories(dirs)
                        }
                    },
                    scanState = scanState,
                    scanResult = scanResult
                )
                1 -> if (playlistsInitialized) PlaylistsScreen()
                2 -> if (downloadInitialized) DownloadScreen()
                3 -> if (settingsInitialized) SettingsScreen()
                4 -> if (syncInitialized) SyncScreen(syncService = syncService)
            }
        }
    }
}
