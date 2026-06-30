# Auralis — Project Status & Context

> **Single source of truth for project state.** Read this first to resume work.
> Last updated: 2026-06-30

---

## TL;DR

Auralis is a cross-platform offline music app (Kotlin Multiplatform + Compose Multiplatform, Material 3) that downloads audio from YouTube and Instagram Reels. **All 13 implementation tasks are complete.** Desktop compiles. Android now assembles a debug APK with SDK 34 installed at `/home/Intro/.local/android-sdk`. Desktop `:desktop:run` still fails due to missing Skiko Linux native AWT runtime library on this machine.

- **Repo root:** `/home/Intro/spectre-enviroment/aureal`
- **Build root:** `/home/Intro/spectre-enviroment/aureal/auralis`
- **Git:** initialized, 2 commits on `main`
- **JDK:** Java 17 at `/usr/lib/jvm/java-17-openjdk` (Java 26 is default but Gradle rejects it)
- **Android SDK:** `/home/Intro/.local/android-sdk` (cmdline-tools, platform-tools, platform 34, build-tools 34)
- **Working build commands:**
  - Desktop compile: `JAVA_HOME=/usr/lib/jvm/java-17-openjdk ./gradlew :desktop:compileKotlin`
  - Android APK: from `auralis/` root with `local.properties` present

---

## 1. What's Done (verified)

| # | Task | Files | Status |
|---|------|-------|--------|
| 1 | Project Scaffolding | 6 build files + gradle wrapper 8.10 | ✅ Done (pre-existing) |
| 2 | Database Schema | 3 `.sq` files (Track, Playlist, PlaylistTrack) | ✅ Code generation confirmed |
| 3 | Data Models | AudioFormat, Track, Playlist | ✅ Compiles |
| 4 | Repository Layer | TrackRepository, PlaylistRepository | ✅ Compiles |
| 5 | Downloader Service | DownloaderService interface + DownloadResult | ✅ Compiles |
| 6 | Player Service | PlayerService interface + PlayerState + RepeatMode | ✅ Compiles |
| 7 | Android Player | AndroidPlayerService (ExoPlayer) + AndroidManifest.xml | ✅ Builds (SDK 34) |
| 8 | Desktop Player | DesktopPlayerService (javax.sound.sampled) | ✅ Compiles |
| 9 | Main Screen UI | MainScreen + BottomPlayer | ✅ Compiles |
| 10 | Library Screen UI | LibraryScreen + TrackItem | ✅ Compiles |
| 11 | Download Screen UI | DownloadScreen + DownloadResultItem | ✅ Compiles |
| 12 | Entry Points | MainActivity (Android) + Main.kt (Desktop) | ✅ APK builds |
| 13 | Build & Verify | `:desktop:compileKotlin` + `:android:assembleDebug` | ✅ Both compile |

**Additional files created beyond the plan:** `PlaylistsScreen.kt`, `SettingsScreen.kt` (stub screens referenced by MainScreen but missing from the original plan), `.gitignore`, `proguard-rules.pro`, `android/local.properties`, `android/src/main/res/` (mipmap-*/ic_launcher.xml, values/styles.xml, values/colors.xml).

---

## 2. What's Left To Do

### 🔴 Blocked (environment)
1. **Desktop Skiko native library** — `:desktop:run` fails with `LibraryLoadException: Cannot find libskiko-linux-x64.so`. `skiko-awt-runtime-linux-x64:0.8.21` added but native .so still not loading at runtime. Needs either a working Skiko version for this JVM or a Wayland-native Compose fix.

### 🟡 Functional gaps (in scope but not in plan)
2. **Downloader implementation** — `DownloaderService` is an interface only. No platform implementation exists. Plan deferred this to a "sidecar or native binding" of yt-dlp. Needs:
   - Desktop: process-bridge to `yt-dlp` + `ffmpeg` binaries
   - Android: bundled yt-dlp or alternative extractor
   - Progress parsing from yt-dlp stdout → `DownloadResult.Progress`
3. **SQLDelight driver wiring** — Schema generates correctly, but no `expect/actual` factory creates `AuralisDatabase` instances at runtime. Needs:
   - `commonMain`: `expect fun createDatabase(...): AuralisDatabase`
   - `androidMain`: `AndroidSqliteDriver`
   - `desktopMain`: `JdbcSqliteDriver`
4. **PlaylistTrack resolution** — `PlaylistRepository.getPlaylistTracks()` returns placeholder tracks (`title = "Track {id}"`). Needs a JOIN query to fetch full Track rows.
5. **UI ↔ Service wiring** — Screens take default empty data. No ViewModel/DI layer connects repositories & services to composables. The architecture diagram (S5) shows a ViewModel layer (PlayerVM, LibraryVM, DownloadVM) that doesn't exist yet.

### 🟢 Polish / future work
6. **Background playback** (Android foreground service with Media3 session) — manifest has permissions, but no `MediaSessionService` is registered.
7. **Metadata editor** (S4.4), **local folder import** (S4.5) — not implemented.
8. **Tests** — `commonTest/` is empty. Plan S11 calls for unit/integration/UI tests.
9. **`kotlinOptions` deprecation** — `shared/build.gradle.kts:13` uses deprecated `kotlinOptions` DSL; migrate to `compilerOptions`.
10. **`QueueMusic` icon deprecation** — `MainScreen.kt:38` should use `Icons.AutoMirrored.Filled.QueueMusic`.
11. **Desktop player limitation** — `javax.sound.sampled.Clip` loads entire file into memory. Replace with streaming approach for large files.
12. **App icons** — `icon.png`/`icon.icns`/`icon.ico` referenced in desktop build but don't exist.

---

## 3. Key Build Facts (must know)

- **JDK:** Use **17** (`/usr/lib/jvm/java-17-openjdk`). Java 26 is the system default but Gradle 8.10 rejects it.
- **Build prefix:** Always run gradle as `JAVA_HOME=/usr/lib/jvm/java-17-openjdk ./gradlew <task>`
- **Android SDK:** `/home/Intro/.local/android-sdk` (cmdline-tools, platform-tools, platform 34, build-tools 34). Set via `auralis/local.properties` (`sdk.dir=/home/Intro/.local/android-sdk`), not needed inside `android/`.
- **Verified tasks:** `:desktop:compileKotlin` (57s cold) and `:android:assembleDebug` (8m cold) → BUILD SUCCESSFUL
- **Gradle version:** 8.10 (via wrapper, downloads on first run)
- **Kotlin:** 2.0.21 · **Compose:** 1.7.1 · **SQLDelight:** 2.0.2 · **Media3:** 1.4.1 · **Skiko:** 0.8.21

### Plan-vs-reality corrections applied (critical for future work)
| Original plan | Actual (correct) |
|---------------|------------------|
| SQLDelight pkg `com.auralis.db` | `com.auralis.database` |
| Media3 `1.5.0` | `1.4.1` |
| Desktop main `com.auralis.MainKt` | `com.auralis.desktop.MainKt` |
| Android pkg `com.auralis` | `com.auralis.android` (app), `com.auralis.shared` (shared) |
| SQLDelight `insert` returns `Long` | Returns `Unit` (use `executeAsOne` + returning clause for ID) |
| iOS targets | Not configured (Android + Desktop JVM only) |

### Build fixes committed
- `settings.gradle.kts`: added missing `repositories { }` wrapper inside `pluginManagement`
- `shared/build.gradle.kts`: added `compose.materialIconsExtended` dependency
- Repository `insertPlaylist`/`insertTrack`: return type `Long` → `Unit`; `position` Int → Long
- `auralis/local.properties`: added `sdk.dir=/home/Intro/.local/android-sdk`
- `gradle.properties`: added `org.gradle.java.home=/usr/lib/jvm/java-17-openjdk`
- `android/build.gradle.kts`: added Material Components dependency for XML theme
- `desktop/build.gradle.kts`: bumped Skiko to `0.8.21` + explicit `skiko-awt-runtime-linux-x64`
- `android/src/main/res/`: added mipmap icons (ic_launcher), `values/styles.xml` (Theme.Auralis), `values/colors.xml`

---

## 4. File Map

```
auralis/
├── local.properties                  # sdk.dir for Android SDK
├── build.gradle.kts              # Root: plugin versions (apply false)
├── settings.gradle.kts           # FIXED: repo blocks + module includes
├── gradle.properties
├── gradlew / gradle/wrapper/     # Gradle 8.10
│
├── shared/
│   ├── build.gradle.kts          # KMP module (FIXED: + materialIconsExtended)
│   └── src/commonMain/
│       ├── sqldelight/com/auralis/database/
│       │   ├── Track.sq          # + 3 distinct index labels (plan had dup)
│       │   ├── Playlist.sq
│       │   └── PlaylistTrack.sq
│       └── kotlin/com/auralis/
│           ├── model/            # AudioFormat, Track, Playlist
│           ├── repository/       # TrackRepository, PlaylistRepository (FIXED)
│           ├── service/          # PlayerService, PlayerState, DownloaderService, DownloadResult
│           └── ui/
│               ├── components/   # BottomPlayer, TrackItem
│               └── screens/      # MainScreen, LibraryScreen, DownloadScreen,
│                               #   PlaylistsScreen (stub), SettingsScreen (stub)
│
├── android/
│   ├── build.gradle.kts          # + Material Components dep
│   ├── proguard-rules.pro
│   └── src/main/
│       ├── AndroidManifest.xml
│       ├── res/
│       │   ├── mipmap-hdpi/ic_launcher.xml
│       │   ├── mipmap-mdpi/ic_launcher.xml
│       │   ├── mipmap-xhdpi/ic_launcher.xml
│       │   ├── mipmap-xxhdpi/ic_launcher.xml
│       │   ├── mipmap-xxxhdpi/ic_launcher.xml
│       │   └── values/{styles,colors}.xml
│       └── kotlin/com/auralis/android/
│           ├── MainActivity.kt
│           └── service/AndroidPlayerService.kt   # ExoPlayer
│
└── desktop/
    ├── build.gradle.kts          # mainClass = com.auralis.desktop.MainKt
    └── src/main/kotlin/com/auralis/desktop/
        ├── Main.kt
        └── service/DesktopPlayerService.kt       # javax.sound.sampled
```

**Generated code (gitignored, do not edit):**
`shared/build/generated/sqldelight/` — AuralisDatabase, Track, Playlist, PlaylistTrack, *Queries classes

---

## 5. Architecture Snapshot

```
UI (composables)  →  [GAP: no ViewModel/DI]  →  Services (PlayerService, DownloaderService)
                                                        ↓
                                              Repository (Track, Playlist)
                                                        ↓
                                              SQLDelight (AuralisDatabase) [GAP: no driver factory]
                                                        ↓
                                              Platform drivers (Android/JDBC) [unwired]
```

**Filled:** UI composables, service interfaces, repositories, schema, platform player impls.
**Gaps:** The connective tissue (DI/VM, DB driver factory, downloader impl) is the next milestone.

---

## 6. Context Documents (for resuming)

| Document | Path | Purpose |
|----------|------|---------|
| **This file** | `docs/compose/PROJECT-STATUS.md` | Current state + what's left |
| Android build setup | `docs/compose/ANDROID_BUILD_SETUP.md` | SDK install + Gradle config for Android target |
| Design spec | `docs/compose/specs/2026-06-29-auralis-design.md` | S1-S11 requirements |
| Implementation plan | `docs/compose/plans/2026-06-29-auralis-implementation.md` | T1-T13 original plan (has corrections noted above) |
| Session checkpoint | `~/.local/share/mimocode/memory/sessions/ses_0ec9c7affffeT3MMsu6E19nh3/checkpoint.md` | Session memory |
| Project memory | `~/.local/share/mimocode/memory/projects/global/MEMORY.md` | Durable Auralis rules + decisions |
| Compose prefs | `~/.local/share/mimocode/memory/projects/global/compose-preferences.md` | Execution style: subagent |

## 6a. Uncommitted Changes (must review before commit)

| File | Change |
|------|--------|
| `auralis/local.properties` | Added `sdk.dir=/home/Intro/.local/android-sdk` |
| `auralis/gradle.properties` | Added `org.gradle.java.home=/usr/lib/jvm/java-17-openjdk` |
| `auralis/android/build.gradle.kts` | Added Material Components dep |
| `auralis/desktop/build.gradle.kts` | Bumped Skiko to `0.8.21` + explicit `skiko-awt-runtime-linux-x64` |
| `auralis/android/src/main/res/` | Added mipmap icons, `values/styles.xml`, `values/colors.xml` (new) |
| `docs/compose/PROJECT-STATUS.md` | This file, updated |
| `docs/compose/ANDROID_BUILD_SETUP.md` | New: Android SDK install instructions |

## 8. Git History

```
e177c60 fix: settings.gradle.kts repository blocks, material-icons-extended dep, repository type mismatches
758b73f feat: implement Tasks 2-12 — database schema, models, repositories, services, UI screens, entry points
```

**Uncommitted work in progress:**
- Android SDK install at `~/.local/android-sdk`
- `auralis/local.properties` and `gradle.properties` Java home fixes
- Android resources (icons, styles, colors)
- Skiko bump + explicit linux-x64 runtime dep
- `docs/compose/ANDROID_BUILD_SETUP.md` (new)

To verify the build still works:
```bash
cd /home/Intro/spectre-enviroment/aureal/auralis
JAVA_HOME=/usr/lib/jvm/java-17-openjdk ./gradlew :desktop:compileKotlin
JAVA_HOME=/usr/lib/jvm/java-17-openjdk ./gradlew :android:assembleDebug
```

For desktop run (requires working Skiko native lib):
```bash
XDG_SESSION_TYPE=wayland GDK_BACKEND=wayland JAVA_HOME=/usr/lib/jvm/java-17-openjdk ./gradlew :desktop:run
```
