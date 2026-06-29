# Auralis Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use compose:subagent (recommended) or compose:execute to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a cross-platform offline music app (Mac, Linux, Android) that downloads and manages audio from YouTube and Instagram Reels.

**Architecture:** Kotlin Multiplatform with Compose UI (Material Design 3). Shared business logic in common module, platform-specific implementations for audio playback and file system access.

**Tech Stack:** Kotlin, Compose Multiplatform, SQLDelight, yt-dlp, FFprobe, Gradle

## Global Constraints

- Kotlin 2.0+ required
- Compose Multiplatform 1.7+ required
- Target SDK: Android 34, macOS 13+, Linux (kernel 5.10+)
- All audio formats supported: MP3, FLAC, AAC, OGG, WAV, M4A
- Storage location: `~/.auralis/`
- Default audio format: MP3 (320kbps)

---

## File Structure

```
auralis/
├── build.gradle.kts                 # Root build config
├── settings.gradle.kts              # Project settings
├── gradle.properties                # Gradle properties
├── shared/                          # Common Kotlin module
│   ├── build.gradle.kts
│   └── src/
│       ├── commonMain/
│       │   └── kotlin/
│       │       └── com/auralis/
│       │           ├── model/        # Data classes
│       │           ├── database/     # SQLDelight schemas
│       │           ├── repository/   # Data access
│       │           ├── service/      # Business logic
│       │           └── di/           # Dependency injection
│       └── commonTest/
├── android/                         # Android app
│   ├── build.gradle.kts
│   └── src/main/
├── desktop/                         # Mac/Linux app
│   ├── build.gradle.kts
│   └── src/main/
└── docs/
    └── compose/
        ├── specs/
        └── plans/
```

---

## Task 1: Project Scaffolding

**Covers:** S3, S8

**Files:**
- Create: `auralis/build.gradle.kts`
- Create: `auralis/settings.gradle.kts`
- Create: `auralis/gradle.properties`
- Create: `auralis/shared/build.gradle.kts`
- Create: `auralis/android/build.gradle.kts`
- Create: `auralis/desktop/build.gradle.kts`

**Interfaces:**
- Produces: Multi-module Gradle project structure

- [ ] **Step 1: Create root build.gradle.kts**

```kotlin
// auralis/build.gradle.kts
plugins {
    kotlin("multiplatform") version "2.0.21" apply false
    id("org.jetbrains.compose") version "1.7.1" apply false
    id("app.cash.sqldelight") version "2.0.2" apply false
}
```

- [ ] **Step 2: Create settings.gradle.kts**

```kotlin
// auralis/settings.gradle.kts
pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
        maven("https://maven.pkg.jetbrains.space/public/p/compose/dev")
    }
}

dependencyResolution {
    repositories {
        google()
        mavenCentral()
        maven("https://maven.pkg.jetbrains.space/public/p/compose/dev")
    }
}

rootProject.name = "auralis"
include(":shared", ":android", ":desktop")
```

- [ ] **Step 3: Create gradle.properties**

```properties
# auralis/gradle.properties
kotlin.code.style=official
org.gradle.jvmargs=-Xmx2048m -Dfile.encoding=UTF-8
android.useAndroidX=true
```

- [ ] **Step 4: Create shared module build.gradle.kts**

```kotlin
// auralis/shared/build.gradle.kts
plugins {
    kotlin("multiplatform")
    id("app.cash.sqldelight")
}

kotlin {
    androidTarget {
        compilations.all {
            kotlinOptions {
                jvmTarget = "17"
            }
        }
    }

    listOf(
        iosX64(),
        iosArm64(),
        iosSimulatorArm64()
    ).forEach { iosTarget ->
        iosTarget.binaries.framework {
            baseName = "shared"
            isStatic = true
        }
    }

    jvm("desktop")

    sourceSets {
        val commonMain by getting {
            dependencies {
                implementation(compose.runtime)
                implementation(compose.foundation)
                implementation(compose.material3)
                implementation("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.9.0")
                implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.7.3")
                implementation("app.cash.sqldelight:coroutines-extensions:2.0.2")
            }
        }

        val commonTest by getting {
            dependencies {
                implementation(kotlin("test"))
            }
        }

        val androidMain by getting {
            dependencies {
                implementation("app.cash.sqldelight:android-driver:2.0.2")
            }
        }

        val desktopMain by getting {
            dependencies {
                implementation("app.cash.sqldelight:sqlite-driver:2.0.2")
            }
        }
    }
}

sqldelight {
    databases {
        create("AuralisDatabase") {
            packageName.set("com.auralis.db")
        }
    }
}
```

- [ ] **Step 5: Create Android module build.gradle.kts**

```kotlin
// auralis/android/build.gradle.kts
plugins {
    kotlin("multiplatform")
    id("com.android.application")
    id("org.jetbrains.compose")
}

kotlin {
    androidTarget {
        compilations.all {
            kotlinOptions {
                jvmTarget = "17"
            }
        }
    }
}

android {
    namespace = "com.auralis"
    compileSdk = 34

    defaultConfig {
        applicationId = "com.auralis"
        minSdk = 26
        targetSdk = 34
        versionCode = 1
        versionName = "1.0.0"
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro"
            )
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

dependencies {
    implementation(project(":shared"))
    implementation(compose.material3)
    implementation("androidx.activity:activity-compose:1.9.3")
    implementation("androidx.media3:media3-exoplayer:1.5.0")
    implementation("androidx.media3:media3-session:1.5.0")
}
```

- [ ] **Step 6: Create Desktop module build.gradle.kts**

```kotlin
// auralis/desktop/build.gradle.kts
plugins {
    kotlin("multiplatform")
    id("org.jetbrains.compose")
    id("application")
}

kotlin {
    jvm("desktop")
}

val desktopMain by getting {
    dependencies {
        implementation(compose.desktop.common)
        implementation(compose.material3)
        implementation(project(":shared"))
    }
}

application {
    mainClass.set("com.auralis.MainKt")
}

tasks.named<JavaExec>("run") {
    dependsOn(tasks.named<Jar>("jar"))
    classpath(tasks.named<Jar>("jar").flatMap { it.archiveFile })
}
```

- [ ] **Step 7: Verify project compiles**

Run: `cd auralis && ./gradlew build`
Expected: BUILD SUCCESSFUL

- [ ] **Step 8: Commit**

```bash
git add auralis/
git commit -m "feat: initialize Kotlin Multiplatform project structure"
```

---

## Task 2: Database Schema

**Covers:** S6, S7

**Files:**
- Create: `auralis/shared/src/commonMain/sqldelight/com/auralis/db/Track.sq`
- Create: `auralis/shared/src/commonMain/sqldelight/com/auralis/db/Playlist.sq`
- Create: `auralis/shared/src/commonMain/sqldelight/com/auralis/db/PlaylistTrack.sq`

**Interfaces:**
- Produces: Type-safe database queries for Track, Playlist, PlaylistTrack

- [ ] **Step 1: Create Track.sq**

```sql
-- auralis/shared/src/commonMain/sqldelight/com/auralis/db/Track.sq
CREATE TABLE Track (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    title TEXT NOT NULL,
    artist TEXT,
    album TEXT,
    genre TEXT,
    year INTEGER,
    track_number INTEGER,
    duration INTEGER NOT NULL DEFAULT 0,
    file_path TEXT NOT NULL UNIQUE,
    thumbnail_path TEXT,
    format TEXT NOT NULL DEFAULT 'mp3',
    size INTEGER NOT NULL DEFAULT 0,
    date_added INTEGER NOT NULL DEFAULT 0,
    source TEXT,
    source_url TEXT
);

createIndex: CREATE INDEX Track_title ON Track(title);
createIndex: CREATE INDEX Track_artist ON Track(artist);
createIndex: CREATE INDEX Track_album ON Track(album);

selectAll:
SELECT * FROM Track ORDER BY date_added DESC;

selectById:
SELECT * FROM Track WHERE id = ?;

selectByTitle:
SELECT * FROM Track WHERE title LIKE ?;

selectBySource:
SELECT * FROM Track WHERE source = ?;

insert:
INSERT INTO Track (title, artist, album, genre, year, track_number, duration, file_path, thumbnail_path, format, size, date_added, source, source_url)
VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?);

update:
UPDATE Track
SET title = ?, artist = ?, album = ?, genre = ?, year = ?, track_number = ?
WHERE id = ?;

deleteById:
DELETE FROM Track WHERE id = ?;
```

- [ ] **Step 2: Create Playlist.sq**

```sql
-- auralis/shared/src/commonMain/sqldelight/com/auralis/db/Playlist.sq
CREATE TABLE Playlist (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    description TEXT,
    thumbnail_path TEXT,
    date_created INTEGER NOT NULL DEFAULT 0,
    date_modified INTEGER NOT NULL DEFAULT 0
);

selectAll:
SELECT * FROM Playlist ORDER BY date_created DESC;

selectById:
SELECT * FROM Playlist WHERE id = ?;

insert:
INSERT INTO Playlist (name, description, thumbnail_path, date_created, date_modified)
VALUES (?, ?, ?, ?, ?);

update:
UPDATE Playlist
SET name = ?, description = ?, thumbnail_path = ?, date_modified = ?
WHERE id = ?;

deleteById:
DELETE FROM Playlist WHERE id = ?;
```

- [ ] **Step 3: Create PlaylistTrack.sq**

```sql
-- auralis/shared/src/commonMain/sqldelight/com/auralis/db/PlaylistTrack.sq
CREATE TABLE PlaylistTrack (
    playlist_id INTEGER NOT NULL,
    track_id INTEGER NOT NULL,
    position INTEGER NOT NULL DEFAULT 0,
    date_added INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (playlist_id, track_id),
    FOREIGN KEY (playlist_id) REFERENCES Playlist(id) ON DELETE CASCADE,
    FOREIGN KEY (track_id) REFERENCES Track(id) ON DELETE CASCADE
);

selectByPlaylist:
SELECT * FROM PlaylistTrack WHERE playlist_id = ? ORDER BY position;

insert:
INSERT INTO PlaylistTrack (playlist_id, track_id, position, date_added)
VALUES (?, ?, ?, ?);

updatePosition:
UPDATE PlaylistTrack SET position = ? WHERE playlist_id = ? AND track_id = ?;

deleteFromPlaylist:
DELETE FROM PlaylistTrack WHERE playlist_id = ? AND track_id = ?;

deleteAllFromPlaylist:
DELETE FROM PlaylistTrack WHERE playlist_id = ?;
```

- [ ] **Step 4: Verify SQLDelight generates code**

Run: `cd auralis && ./gradlew :shared:generateMainAuralisDatabaseInterface`
Expected: Generated Kotlin code in `shared/build/generated/sqldelight/`

- [ ] **Step 5: Commit**

```bash
git add auralis/shared/src/commonMain/sqldelight/
git commit -m "feat: add SQLDelight database schema for tracks and playlists"
```

---

## Task 3: Data Models

**Covers:** S6

**Files:**
- Create: `auralis/shared/src/commonMain/kotlin/com/auralis/model/Track.kt`
- Create: `auralis/shared/src/commonMain/kotlin/com/auralis/model/Playlist.kt`
- Create: `auralis/shared/src/commonMain/kotlin/com/auralis/model/AudioFormat.kt`

**Interfaces:**
- Produces: Domain models used across the app

- [ ] **Step 1: Create AudioFormat.kt**

```kotlin
// auralis/shared/src/commonMain/kotlin/com/auralis/model/AudioFormat.kt
package com.auralis.model

enum class AudioFormat(val extension: String, val mimeType: String) {
    MP3("mp3", "audio/mpeg"),
    FLAC("flac", "audio/flac"),
    AAC("aac", "audio/aac"),
    OGG("ogg", "audio/ogg"),
    WAV("wav", "audio/wav"),
    M4A("m4a", "audio/mp4");

    companion object {
        fun fromExtension(ext: String): AudioFormat {
            return entries.find { it.extension.equals(ext, ignoreCase = true) } ?: MP3
        }
    }
}
```

- [ ] **Step 2: Create Track.kt**

```kotlin
// auralis/shared/src/commonMain/kotlin/com/auralis/model/Track.kt
package com.auralis.model

data class Track(
    val id: Long = 0,
    val title: String,
    val artist: String? = null,
    val album: String? = null,
    val genre: String? = null,
    val year: Int? = null,
    val trackNumber: Int? = null,
    val duration: Long = 0,
    val filePath: String,
    val thumbnailPath: String? = null,
    val format: AudioFormat = AudioFormat.MP3,
    val size: Long = 0,
    val dateAdded: Long = System.currentTimeMillis(),
    val source: String? = null,
    val sourceUrl: String? = null
) {
    val durationFormatted: String
        get() {
            val minutes = duration / 60000
            val seconds = (duration % 60000) / 1000
            return "%d:%02d".format(minutes, seconds)
        }

    val sizeFormatted: String
        get() = when {
            size < 1024 -> "$size B"
            size < 1024 * 1024 -> "${size / 1024} KB"
            else -> "%.1f MB".format(size / (1024.0 * 1024.0))
        }
}
```

- [ ] **Step 3: Create Playlist.kt**

```kotlin
// auralis/shared/src/commonMain/kotlin/com/auralis/model/Playlist.kt
package com.auralis.model

data class Playlist(
    val id: Long = 0,
    val name: String,
    val description: String? = null,
    val thumbnailPath: String? = null,
    val dateCreated: Long = System.currentTimeMillis(),
    val dateModified: Long = System.currentTimeMillis(),
    val trackCount: Int = 0
)
```

- [ ] **Step 4: Commit**

```bash
git add auralis/shared/src/commonMain/kotlin/com/auralis/model/
git commit -m "feat: add domain models for Track, Playlist, and AudioFormat"
```

---

## Task 4: Repository Layer

**Covers:** S6, S7

**Files:**
- Create: `auralis/shared/src/commonMain/kotlin/com/auralis/repository/TrackRepository.kt`
- Create: `auralis/shared/src/commonMain/kotlin/com/auralis/repository/PlaylistRepository.kt`

**Interfaces:**
- Consumes: SQLDelight generated queries, domain models
- Produces: Repository interfaces for data access

- [ ] **Step 1: Create TrackRepository.kt**

```kotlin
// auralis/shared/src/commonMain/kotlin/com/auralis/repository/TrackRepository.kt
package com.auralis.repository

import com.auralis.db.AuralisDatabase
import com.auralis.model.AudioFormat
import com.auralis.model.Track
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

class TrackRepository(private val database: AuralisDatabase) {
    private val queries = database.trackQueries

    suspend fun getAllTracks(): List<Track> = withContext(Dispatchers.Default) {
        queries.selectAll().executeAsList().map { it.toDomain() }
    }

    suspend fun getTrackById(id: Long): Track? = withContext(Dispatchers.Default) {
        queries.selectById(id).executeAsOneOrNull()?.toDomain()
    }

    suspend fun searchTracks(query: String): List<Track> = withContext(Dispatchers.Default) {
        queries.selectByTitle("%$query%").executeAsList().map { it.toDomain() }
    }

    suspend fun getTracksBySource(source: String): List<Track> = withContext(Dispatchers.Default) {
        queries.selectBySource(source).executeAsList().map { it.toDomain() }
    }

    suspend fun insertTrack(track: Track): Long = withContext(Dispatchers.Default) {
        queries.insert(
            title = track.title,
            artist = track.artist,
            album = track.album,
            genre = track.genre,
            year = track.year?.toLong(),
            track_number = track.trackNumber?.toLong(),
            duration = track.duration,
            file_path = track.filePath,
            thumbnail_path = track.thumbnailPath,
            format = track.format.extension,
            size = track.size,
            date_added = track.dateAdded,
            source = track.source,
            source_url = track.sourceUrl
        )
    }

    suspend fun updateTrack(track: Track) = withContext(Dispatchers.Default) {
        queries.update(
            title = track.title,
            artist = track.artist,
            album = track.album,
            genre = track.genre,
            year = track.year?.toLong(),
            track_number = track.trackNumber?.toLong(),
            id = track.id
        )
    }

    suspend fun deleteTrack(id: Long) = withContext(Dispatchers.Default) {
        queries.deleteById(id)
    }

    private fun com.auralis.db.Track.toDomain() = Track(
        id = id,
        title = title,
        artist = artist,
        album = album,
        genre = genre,
        year = year?.toInt(),
        track_number = track_number?.toInt(),
        duration = duration,
        file_path = file_path,
        thumbnail_path = thumbnail_path,
        format = AudioFormat.fromExtension(format),
        size = size,
        date_added = date_added,
        source = source,
        source_url = source_url
    )
}
```

- [ ] **Step 2: Create PlaylistRepository.kt**

```kotlin
// auralis/shared/src/commonMain/kotlin/com/auralis/repository/PlaylistRepository.kt
package com.auralis.repository

import com.auralis.db.AuralisDatabase
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
            // Would need TrackRepository to resolve track IDs
            // Simplified for now
            Track(id = it.track_id, title = "Track ${it.track_id}", filePath = "")
        }
    }

    private fun com.auralis.db.Playlist.toDomain() = Playlist(
        id = id,
        name = name,
        description = description,
        thumbnail_path = thumbnail_path,
        date_created = date_created,
        date_modified = date_modified
    )
}
```

- [ ] **Step 3: Commit**

```bash
git add auralis/shared/src/commonMain/kotlin/com/auralis/repository/
git commit -m "feat: add repository layer for tracks and playlists"
```

---

## Task 5: Downloader Service

**Covers:** S4.1

**Files:**
- Create: `auralis/shared/src/commonMain/kotlin/com/auralis/service/DownloaderService.kt`
- Create: `auralis/shared/src/commonMain/kotlin/com/auralis/service/DownloadResult.kt`

**Interfaces:**
- Consumes: Repository layer, platform file system APIs
- Produces: Download functionality for YouTube/Instagram

- [ ] **Step 1: Create DownloadResult.kt**

```kotlin
// auralis/shared/src/commonMain/kotlin/com/auralis/service/DownloadResult.kt
package com.auralis.service

sealed class DownloadResult {
    data class Success(val filePath: String, val metadata: MediaMetadata) : DownloadResult()
    data class Error(val message: String, val exception: Exception? = null) : DownloadResult()
    data class Progress(val percent: Float, val speed: String? = null) : DownloadResult()
}

data class MediaMetadata(
    val title: String,
    val artist: String? = null,
    val thumbnailUrl: String? = null,
    val duration: Long? = null,
    val source: String,
    val sourceUrl: String
)
```

- [ ] **Step 2: Create DownloaderService.kt**

```kotlin
// auralis/shared/src/commonMain/kotlin/com/auralis/service/DownloaderService.kt
package com.auralis.service

import com.auralis.model.AudioFormat
import kotlinx.coroutines.flow.Flow

interface DownloaderService {
    suspend fun downloadAudio(
        url: String,
        format: AudioFormat = AudioFormat.MP3,
        quality: Int = 320
    ): Flow<DownloadResult>

    suspend fun getMetadata(url: String): MediaMetadata?

    suspend fun isSupportedUrl(url: String): Boolean
}

// Platform-specific implementation will be provided in android/desktop modules
```

- [ ] **Step 3: Commit**

```bash
git add auralis/shared/src/commonMain/kotlin/com/auralis/service/
git commit -m "feat: add downloader service interface and download result types"
```

---

## Task 6: Player Service

**Covers:** S4.2

**Files:**
- Create: `auralis/shared/src/commonMain/kotlin/com/auralis/service/PlayerService.kt`
- Create: `auralis/shared/src/commonMain/kotlin/com/auralis/service/PlayerState.kt`

**Interfaces:**
- Consumes: Track model, repository
- Produces: Audio playback functionality

- [ ] **Step 1: Create PlayerState.kt**

```kotlin
// auralis/shared/src/commonMain/kotlin/com/auralis/service/PlayerState.kt
package com.auralis.service

import com.auralis.model.Track

data class PlayerState(
    val isPlaying: Boolean = false,
    val currentTrack: Track? = null,
    val queue: List<Track> = emptyList(),
    val currentIndex: Int = -1,
    val position: Long = 0,
    val duration: Long = 0,
    val repeatMode: RepeatMode = RepeatMode.OFF,
    val isShuffleEnabled: Boolean = false
) {
    val progress: Float
        get() = if (duration > 0) position.toFloat() / duration else 0f

    val positionFormatted: String
        get() = formatTime(position)

    val durationFormatted: String
        get() = formatTime(duration)

    private fun formatTime(ms: Long): String {
        val minutes = ms / 60000
        val seconds = (ms % 60000) / 1000
        return "%d:%02d".format(minutes, seconds)
    }
}

enum class RepeatMode {
    OFF, ONE, ALL
}
```

- [ ] **Step 2: Create PlayerService.kt**

```kotlin
// auralis/shared/src/commonMain/kotlin/com/auralis/service/PlayerService.kt
package com.auralis.service

import com.auralis.model.Track
import kotlinx.coroutines.flow.StateFlow

interface PlayerService {
    val state: StateFlow<PlayerState>

    suspend fun play(track: Track)
    suspend fun playQueue(tracks: List<Track>, startIndex: Int = 0)
    suspend fun pause()
    suspend fun resume()
    suspend fun stop()
    suspend fun seekTo(position: Long)
    suspend fun next()
    suspend fun previous()
    suspend fun setRepeatMode(mode: RepeatMode)
    suspend fun toggleShuffle()
    suspend fun addToQueue(track: Track)
    suspend fun removeFromQueue(index: Int)
    suspend fun clearQueue()
}

// Platform-specific implementation will be provided in android/desktop modules
```

- [ ] **Step 3: Commit**

```bash
git add auralis/shared/src/commonMain/kotlin/com/auralis/service/
git commit -m "feat: add player service interface and player state"
```

---

## Task 7: Android Player Implementation

**Covers:** S4.2, S8

**Files:**
- Create: `auralis/android/src/main/kotlin/com/auralis/service/AndroidPlayerService.kt`
- Modify: `auralis/android/src/main/AndroidManifest.xml`

**Interfaces:**
- Consumes: PlayerService interface, Track model
- Produces: Android-specific audio playback using Media3/ExoPlayer

- [ ] **Step 1: Create AndroidPlayerService.kt**

```kotlin
// auralis/android/src/main/kotlin/com/auralis/service/AndroidPlayerService.kt
package com.auralis.service

import android.content.Context
import androidx.media3.common.MediaItem
import androidx.media3.exoplayer.ExoPlayer
import com.auralis.model.Track
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update

class AndroidPlayerService(private val context: Context) : PlayerService {
    private var exoPlayer: ExoPlayer? = null
    private val _state = MutableStateFlow(PlayerState())
    override val state: StateFlow<PlayerState> = _state.asStateFlow()

    init {
        exoPlayer = ExoPlayer.Builder(context).build().apply {
            playWhenReady = true
        }
    }

    override suspend fun play(track: Track) {
        exoPlayer?.apply {
            val mediaItem = MediaItem.fromUri(track.filePath)
            setMediaItem(mediaItem)
            prepare()
            play()
        }
        _state.update { it.copy(isPlaying = true, currentTrack = track) }
    }

    override suspend fun playQueue(tracks: List<Track>, startIndex: Int) {
        exoPlayer?.apply {
            val mediaItems = tracks.map { MediaItem.fromUri(it.filePath) }
            setMediaItems(mediaItems, startIndex, 0)
            prepare()
            play()
        }
        _state.update {
            it.copy(
                isPlaying = true,
                queue = tracks,
                currentIndex = startIndex,
                currentTrack = tracks.getOrNull(startIndex)
            )
        }
    }

    override suspend fun pause() {
        exoPlayer?.pause()
        _state.update { it.copy(isPlaying = false) }
    }

    override suspend fun resume() {
        exoPlayer?.play()
        _state.update { it.copy(isPlaying = true) }
    }

    override suspend fun stop() {
        exoPlayer?.stop()
        _state.update { it.copy(isPlaying = false, currentTrack = null) }
    }

    override suspend fun seekTo(position: Long) {
        exoPlayer?.seekTo(position)
        _state.update { it.copy(position = position) }
    }

    override suspend fun next() {
        val currentState = _state.value
        val nextIndex = currentState.currentIndex + 1
        if (nextIndex < currentState.queue.size) {
            val track = currentState.queue[nextIndex]
            play(track)
            _state.update { it.copy(currentIndex = nextIndex) }
        }
    }

    override suspend fun previous() {
        val currentState = _state.value
        val prevIndex = currentState.currentIndex - 1
        if (prevIndex >= 0) {
            val track = currentState.queue[prevIndex]
            play(track)
            _state.update { it.copy(currentIndex = prevIndex) }
        }
    }

    override suspend fun setRepeatMode(mode: RepeatMode) {
        exoPlayer?.repeatMode = when (mode) {
            RepeatMode.OFF -> ExoPlayer.REPEAT_MODE_OFF
            RepeatMode.ONE -> ExoPlayer.REPEAT_MODE_ONE
            RepeatMode.ALL -> ExoPlayer.REPEAT_MODE_ALL
        }
        _state.update { it.copy(repeatMode = mode) }
    }

    override suspend fun toggleShuffle() {
        val newState = !_state.value.isShuffleEnabled
        exoPlayer?.shuffleModeEnabled = newState
        _state.update { it.copy(isShuffleEnabled = newState) }
    }

    override suspend fun addToQueue(track: Track) {
        _state.update { it.copy(queue = it.queue + track) }
    }

    override suspend fun removeFromQueue(index: Int) {
        _state.update { it.copy(queue = it.queue.toMutableList().apply { removeAt(index) }) }
    }

    override suspend fun clearQueue() {
        exoPlayer?.stop()
        _state.update { PlayerState() }
    }

    fun release() {
        exoPlayer?.release()
        exoPlayer = null
    }
}
```

- [ ] **Step 2: Update AndroidManifest.xml**

```xml
<!-- auralis/android/src/main/AndroidManifest.xml -->
<?xml version="1.0" encoding="utf-8"?>
<manifest xmlns:android="http://schemas.android.com/apk/res/android">

    <uses-permission android:name="android.permission.INTERNET" />
    <uses-permission android:name="android.permission.WRITE_EXTERNAL_STORAGE"
        android:maxSdkVersion="28" />
    <uses-permission android:name="android.permission.READ_EXTERNAL_STORAGE"
        android:maxSdkVersion="32" />
    <uses-permission android:name="android.permission.FOREGROUND_SERVICE" />
    <uses-permission android:name="android.permission.FOREGROUND_SERVICE_MEDIA_PLAYBACK" />

    <application
        android:allowBackup="true"
        android:icon="@mipmap/ic_launcher"
        android:label="Auralis"
        android:supportsRtl="true"
        android:theme="@style/Theme.Auralis">
        <activity
            android:name=".MainActivity"
            android:exported="true"
            android:theme="@style/Theme.Auralis">
            <intent-filter>
                <action android:name="android.intent.action.MAIN" />
                <category android:name="android.intent.category.LAUNCHER" />
            </intent-filter>
        </activity>
    </application>

</manifest>
```

- [ ] **Step 3: Commit**

```bash
git add auralis/android/
git commit -m "feat: add Android player service implementation with ExoPlayer"
```

---

## Task 8: Desktop Player Implementation

**Covers:** S4.2, S8

**Files:**
- Create: `auralis/desktop/src/main/kotlin/com/auralis/service/DesktopPlayerService.kt`

**Interfaces:**
- Consumes: PlayerService interface, Track model
- Produces: Desktop audio playback (Mac/Linux)

- [ ] **Step 1: Create DesktopPlayerService.kt**

```kotlin
// auralis/desktop/src/main/kotlin/com/auralis/service/DesktopPlayerService.kt
package com.auralis.service

import com.auralis.model.Track
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import javax.sound.sampled.AudioSystem
import javax.sound.sampled.Clip
import javax.sound.sampled.LineEvent

class DesktopPlayerService : PlayerService {
    private var clip: Clip? = null
    private val _state = MutableStateFlow(PlayerState())
    override val state: StateFlow<PlayerState> = _state.asStateFlow()

    override suspend fun play(track: Track) {
        stop()
        try {
            val audioInputStream = AudioSystem.getAudioInputStream(java.io.File(track.filePath))
            clip = AudioSystem.getClip().apply {
                open(audioInputStream)
                addLineListener { event ->
                    if (event.type == LineEvent.Type.STOP) {
                        close()
                        _state.update { it.copy(isPlaying = false) }
                    }
                }
                start()
            }
            _state.update { it.copy(isPlaying = true, currentTrack = track, duration = clip?.microsecondLength ?: 0) }
        } catch (e: Exception) {
            _state.update { it.copy(isPlaying = false) }
        }
    }

    override suspend fun playQueue(tracks: List<Track>, startIndex: Int) {
        _state.update { it.copy(queue = tracks, currentIndex = startIndex) }
        tracks.getOrNull(startIndex)?.let { play(it) }
    }

    override suspend fun pause() {
        clip?.stop()
        _state.update { it.copy(isPlaying = false) }
    }

    override suspend fun resume() {
        clip?.start()
        _state.update { it.copy(isPlaying = true) }
    }

    override suspend fun stop() {
        clip?.stop()
        clip?.close()
        clip = null
        _state.update { it.copy(isPlaying = false, position = 0) }
    }

    override suspend fun seekTo(position: Long) {
        clip?.microsecondPosition = position
        _state.update { it.copy(position = position) }
    }

    override suspend fun next() {
        val currentState = _state.value
        val nextIndex = currentState.currentIndex + 1
        if (nextIndex < currentState.queue.size) {
            _state.update { it.copy(currentIndex = nextIndex) }
            play(currentState.queue[nextIndex])
        }
    }

    override suspend fun previous() {
        val currentState = _state.value
        val prevIndex = currentState.currentIndex - 1
        if (prevIndex >= 0) {
            _state.update { it.copy(currentIndex = prevIndex) }
            play(currentState.queue[prevIndex])
        }
    }

    override suspend fun setRepeatMode(mode: RepeatMode) {
        _state.update { it.copy(repeatMode = mode) }
    }

    override suspend fun toggleShuffle() {
        _state.update { it.copy(isShuffleEnabled = !it.isShuffleEnabled) }
    }

    override suspend fun addToQueue(track: Track) {
        _state.update { it.copy(queue = it.queue + track) }
    }

    override suspend fun removeFromQueue(index: Int) {
        _state.update { it.copy(queue = it.queue.toMutableList().apply { removeAt(index) }) }
    }

    override suspend fun clearQueue() {
        stop()
        _state.update { PlayerState() }
    }
}
```

- [ ] **Step 2: Commit**

```bash
git add auralis/desktop/
git commit -m "feat: add desktop player service implementation"
```

---

## Task 9: Compose UI - Main Screen

**Covers:** S4.3, S8

**Files:**
- Create: `auralis/shared/src/commonMain/kotlin/com/auralis/ui/screens/MainScreen.kt`
- Create: `auralis/shared/src/commonMain/kotlin/com/auralis/ui/components/BottomPlayer.kt`

**Interfaces:**
- Consumes: PlayerState, Track model
- Produces: Main navigation screen with bottom player

- [ ] **Step 1: Create MainScreen.kt**

```kotlin
// auralis/shared/src/commonMain/kotlin/com/auralis/ui/screens/MainScreen.kt
package com.auralis.ui.screens

import androidx.compose.foundation.layout.*
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import com.auralis.ui.components.BottomPlayer

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun MainScreen() {
    var selectedTab by remember { mutableStateOf(0) }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Auralis") },
                actions = {
                    IconButton(onClick = { /* Search */ }) {
                        Icon(Icons.Default.Search, contentDescription = "Search")
                    }
                }
            )
        },
        bottomBar = {
            Column {
                BottomPlayer()
                NavigationBar {
                    NavigationBarItem(
                        icon = { Icon(Icons.Default.LibraryMusic, contentDescription = null) },
                        label = { Text("Library") },
                        selected = selectedTab == 0,
                        onClick = { selectedTab = 0 }
                    )
                    NavigationBarItem(
                        icon = { Icon(Icons.Default.QueueMusic, contentDescription = null) },
                        label = { Text("Playlists") },
                        selected = selectedTab == 1,
                        onClick = { selectedTab = 1 }
                    )
                    NavigationBarItem(
                        icon = { Icon(Icons.Default.Download, contentDescription = null) },
                        label = { Text("Downloads") },
                        selected = selectedTab == 2,
                        onClick = { selectedTab = 2 }
                    )
                    NavigationBarItem(
                        icon = { Icon(Icons.Default.Settings, contentDescription = null) },
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
                0 -> LibraryScreen()
                1 -> PlaylistsScreen()
                2 -> DownloadsScreen()
                3 -> SettingsScreen()
            }
        }
    }
}
```

- [ ] **Step 2: Create BottomPlayer.kt**

```kotlin
// auralis/shared/src/commonMain/kotlin/com/auralis/ui/components/BottomPlayer.kt
package com.auralis.ui.components

import androidx.compose.foundation.layout.*
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp

@Composable
fun BottomPlayer(
    title: String = "No track playing",
    artist: String? = null,
    isPlaying: Boolean = false,
    progress: Float = 0f,
    onPlayPause: () -> Unit = {},
    onNext: () -> Unit = {},
    onPrevious: () -> Unit = {}
) {
    Surface(
        tonalElevation = 3.dp
    ) {
        Column {
            LinearProgressIndicator(
                progress = { progress },
                modifier = Modifier
                    .fillMaxWidth()
                    .height(2.dp)
            )
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp, vertical = 8.dp),
                verticalAlignment = Alignment.CenterVertically
            ) {
                Column(modifier = Modifier.weight(1f)) {
                    Text(
                        text = title,
                        style = MaterialTheme.typography.bodyMedium,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis
                    )
                    if (artist != null) {
                        Text(
                            text = artist,
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis
                        )
                    }
                }
                Row {
                    IconButton(onClick = onPrevious) {
                        Icon(Icons.Default.SkipPrevious, contentDescription = "Previous")
                    }
                    IconButton(onClick = onPlayPause) {
                        Icon(
                            if (isPlaying) Icons.Default.Pause else Icons.Default.PlayArrow,
                            contentDescription = if (isPlaying) "Pause" else "Play"
                        )
                    }
                    IconButton(onClick = onNext) {
                        Icon(Icons.Default.SkipNext, contentDescription = "Next")
                    }
                }
            }
        }
    }
}
```

- [ ] **Step 3: Commit**

```bash
git add auralis/shared/src/commonMain/kotlin/com/auralis/ui/
git commit -m "feat: add main screen and bottom player UI components"
```

---

## Task 10: Compose UI - Library Screen

**Covers:** S4.3

**Files:**
- Create: `auralis/shared/src/commonMain/kotlin/com/auralis/ui/screens/LibraryScreen.kt`
- Create: `auralis/shared/src/commonMain/kotlin/com/auralis/ui/components/TrackItem.kt`

**Interfaces:**
- Consumes: Track list, player service
- Produces: Library browsing UI

- [ ] **Step 1: Create TrackItem.kt**

```kotlin
// auralis/shared/src/commonMain/kotlin/com/auralis/ui/components/TrackItem.kt
package com.auralis.ui.components

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.MusicNote
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.auralis.model.Track

@Composable
fun TrackItem(
    track: Track,
    onClick: () -> Unit,
    modifier: Modifier = Modifier
) {
    Row(
        modifier = modifier
            .fillMaxWidth()
            .clickable(onClick = onClick)
            .padding(horizontal = 16.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically
    ) {
        Surface(
            modifier = Modifier.size(48.dp),
            shape = MaterialTheme.shapes.small,
            color = MaterialTheme.colorScheme.surfaceVariant
        ) {
            Icon(
                Icons.Default.MusicNote,
                contentDescription = null,
                modifier = Modifier.padding(12.dp),
                tint = MaterialTheme.colorScheme.onSurfaceVariant
            )
        }
        Spacer(modifier = Modifier.width(16.dp))
        Column(modifier = Modifier.weight(1f)) {
            Text(
                text = track.title,
                style = MaterialTheme.typography.bodyLarge,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis
            )
            if (track.artist != null) {
                Text(
                    text = track.artist,
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis
                )
            }
        }
        Text(
            text = track.durationFormatted,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant
        )
    }
}
```

- [ ] **Step 2: Create LibraryScreen.kt**

```kotlin
// auralis/shared/src/commonMain/kotlin/com/auralis/ui/screens/LibraryScreen.kt
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
import com.auralis.ui.components.TrackItem

@Composable
fun LibraryScreen(
    tracks: List<Track> = emptyList(),
    onTrackClick: (Track) -> Unit = {}
) {
    if (tracks.isEmpty()) {
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
                    text = "Download music or import from your device",
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
```

- [ ] **Step 3: Commit**

```bash
git add auralis/shared/src/commonMain/kotlin/com/auralis/ui/
git commit -m "feat: add library screen and track item UI components"
```

---

## Task 11: Compose UI - Download Screen

**Covers:** S4.1

**Files:**
- Create: `auralis/shared/src/commonMain/kotlin/com/auralis/ui/screens/DownloadScreen.kt`

**Interfaces:**
- Consumes: Downloader service, download state
- Produces: Download URL input and progress UI

- [ ] **Step 1: Create DownloadScreen.kt**

```kotlin
// auralis/shared/src/commonMain/kotlin/com/auralis/ui/screens/DownloadScreen.kt
package com.auralis.ui.screens

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.auralis.service.DownloadResult
import com.auralis.service.MediaMetadata

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun DownloadScreen(
    downloads: List<DownloadResult> = emptyList(),
    onDownload: (String) -> Unit = {}
) {
    var url by remember { mutableStateOf("") }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(16.dp)
    ) {
        Text(
            text = "Download Music",
            style = MaterialTheme.typography.headlineMedium
        )
        Spacer(modifier = Modifier.height(16.dp))

        OutlinedTextField(
            value = url,
            onValueChange = { url = it },
            label = { Text("Paste URL from YouTube or Instagram") },
            modifier = Modifier.fillMaxWidth(),
            leadingIcon = { Icon(Icons.Default.Link, contentDescription = null) },
            trailingIcon = {
                if (url.isNotEmpty()) {
                    IconButton(onClick = { url = "" }) {
                        Icon(Icons.Default.Clear, contentDescription = "Clear")
                    }
                }
            }
        )

        Spacer(modifier = Modifier.height(16.dp))

        Button(
            onClick = { onDownload(url) },
            modifier = Modifier.fillMaxWidth(),
            enabled = url.isNotBlank()
        ) {
            Icon(Icons.Default.Download, contentDescription = null)
            Spacer(modifier = Modifier.width(8.dp))
            Text("Download")
        }

        Spacer(modifier = Modifier.height(24.dp))

        if (downloads.isNotEmpty()) {
            Text(
                text = "Recent Downloads",
                style = MaterialTheme.typography.titleMedium
            )
            Spacer(modifier = Modifier.height(8.dp))

            LazyColumn {
                items(downloads) { result ->
                    DownloadResultItem(result)
                }
            }
        }
    }
}

@Composable
fun DownloadResultItem(result: DownloadResult) {
    Card(
        modifier = Modifier
            .fillMaxWidth()
            .padding(vertical = 4.dp)
    ) {
        Row(
            modifier = Modifier
                .padding(16.dp)
                .fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically
        ) {
            when (result) {
                is DownloadResult.Success -> {
                    Icon(
                        Icons.Default.CheckCircle,
                        contentDescription = null,
                        tint = MaterialTheme.colorScheme.primary
                    )
                    Spacer(modifier = Modifier.width(12.dp))
                    Column {
                        Text(result.metadata.title, style = MaterialTheme.typography.bodyLarge)
                        Text(
                            "Downloaded successfully",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                    }
                }
                is DownloadResult.Error -> {
                    Icon(
                        Icons.Default.Error,
                        contentDescription = null,
                        tint = MaterialTheme.colorScheme.error
                    )
                    Spacer(modifier = Modifier.width(12.dp))
                    Column {
                        Text("Download failed", style = MaterialTheme.typography.bodyLarge)
                        Text(
                            result.message,
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.error
                        )
                    }
                }
                is DownloadResult.Progress -> {
                    CircularProgressIndicator(modifier = Modifier.size(24.dp))
                    Spacer(modifier = Modifier.width(12.dp))
                    Column {
                        Text("Downloading...", style = MaterialTheme.typography.bodyLarge)
                        Text(
                            "${(result.percent * 100).toInt()}%",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                    }
                }
            }
        }
    }
}
```

- [ ] **Step 2: Commit**

```bash
git add auralis/shared/src/commonMain/kotlin/com/auralis/ui/screens/DownloadScreen.kt
git commit -m "feat: add download screen with URL input and progress UI"
```

---

## Task 12: App Entry Points

**Covers:** S8

**Files:**
- Create: `auralis/android/src/main/kotlin/com/auralis/MainActivity.kt`
- Create: `auralis/desktop/src/main/kotlin/com/auralis/Main.kt`

**Interfaces:**
- Consumes: All UI screens, services
- Produces: Platform-specific app entry points

- [ ] **Step 1: Create Android MainActivity.kt**

```kotlin
// auralis/android/src/main/kotlin/com/auralis/MainActivity.kt
package com.auralis

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.material3.MaterialTheme
import com.auralis.ui.screens.MainScreen

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent {
            MaterialTheme {
                MainScreen()
            }
        }
    }
}
```

- [ ] **Step 2: Create Desktop Main.kt**

```kotlin
// auralis/desktop/src/main/kotlin/com/auralis/Main.kt
package com.auralis

import androidx.compose.ui.window.Window
import androidx.compose.ui.window.application
import androidx.compose.ui.window.rememberWindowState
import androidx.compose.material3.MaterialTheme
import com.auralis.ui.screens.MainScreen

fun main() = application {
    Window(
        onCloseRequest = ::exitApplication,
        title = "Auralis",
        state = rememberWindowState()
    ) {
        MaterialTheme {
            MainScreen()
        }
    }
}
```

- [ ] **Step 3: Commit**

```bash
git add auralis/android/src/main/kotlin/com/auralis/MainActivity.kt
git add auralis/desktop/src/main/kotlin/com/auralis/Main.kt
git commit -m "feat: add platform entry points for Android and Desktop"
```

---

## Task 13: Build and Verify

**Covers:** S8

**Files:**
- Modify: Various build files if needed

**Interfaces:**
- Produces: Working build for all platforms

- [ ] **Step 1: Build Android APK**

Run: `cd auralis && ./gradlew :android:assembleDebug`
Expected: APK generated at `android/build/outputs/apk/debug/`

- [ ] **Step 2: Build Desktop JAR**

Run: `cd auralis && ./gradlew :desktop:jar`
Expected: JAR generated at `desktop/build/libs/`

- [ ] **Step 3: Run Desktop App**

Run: `cd auralis && ./gradlew :desktop:run`
Expected: App window opens

- [ ] **Step 4: Run Tests**

Run: `cd auralis && ./gradlew test`
Expected: All tests pass

- [ ] **Step 5: Final Commit**

```bash
git add -A
git commit -m "feat: complete initial implementation of Auralis music app"
```

---

## Summary

| Task | Description | Covers |
|------|-------------|--------|
| 1 | Project Scaffolding | S3, S8 |
| 2 | Database Schema | S6, S7 |
| 3 | Data Models | S6 |
| 4 | Repository Layer | S6, S7 |
| 5 | Downloader Service | S4.1 |
| 6 | Player Service | S4.2 |
| 7 | Android Player | S4.2, S8 |
| 8 | Desktop Player | S4.2, S8 |
| 9 | Main Screen UI | S4.3, S8 |
| 10 | Library Screen UI | S4.3 |
| 11 | Download Screen UI | S4.1 |
| 12 | App Entry Points | S8 |
| 13 | Build and Verify | S8 |
