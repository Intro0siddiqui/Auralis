# Auralis — Android Build Setup

> Documenting the exact steps to build the Android target on a fresh machine.
> Last updated: 2026-06-30

## Prerequisites

- JDK 17 (Java 26 will be rejected by Gradle 8.10)
- Linux x86_64

## Steps Already Completed on This Machine

1. Download Android command-line tools to `~/.local/android-sdk/cmdline-tools/latest/`
2. Accept licenses: `JAVA_HOME=/usr/lib/jvm/java-17-openjdk ~/.local/android-sdk/cmdline-tools/latest/bin/sdkmanager --licenses --sdk_root=/home/Intro/.local/android-sdk`
3. Install packages:
   - `platform-tools`
   - `platforms;android-34`
   - `build-tools;34.0.0`
4. Created app resources under `auralis/android/src/main/res/`:
   - `mipmap-{hdpi,mdpi,xhdpi,xxhdpi,xxxhdpi}/ic_launcher.xml`
   - `values/colors.xml`
   - `values/styles.xml`
5. Configured Gradle for Java 17 (`org.gradle.java.home` in `gradle.properties`)
6. Added Material Components dependency to `android/build.gradle.kts`

## Required Config Files

### `auralis/local.properties`
```
sdk.dir=/home/Intro/.local/android-sdk
```
This must exist at the project root (`auralis/`), not inside the `android/` subfolder.

### `auralis/gradle.properties`
Ensure this line is present:
```
org.gradle.java.home=/usr/lib/jvm/java-17-openjdk
```

## Build Command

```bash
cd /home/Intro/spectre-enviroment/aureal/auralis
JAVA_HOME=/usr/lib/jvm/java-17-openjdk ./gradlew :android:assembleDebug
```

Output APK:
```
auralis/android/build/outputs/apk/debug/android-debug.apk
```

## Environment Variables for Desktop (Sway/Wayland)

When running desktop target on Linux:
```bash
XDG_SESSION_TYPE=wayland GDK_BACKEND=wayland JAVA_HOME=/usr/lib/jvm/java-17-openjdk ./gradlew :desktop:run
```

## Known Issues

- **Skiko Linux native library:** `:desktop:run` fails with `LibraryLoadException: Cannot find libskiko-linux-x64.so` even with `skiko-awt-runtime-linux-x64:0.8.21` on the classpath. The dependency resolves, but the native `.so` is not loaded at runtime. Needs investigation or a working Skiko version for JVM Linux x86_64 with Compose 1.7.1.
- **App icons:** Placeholder vector icons are used. Real icons should replace the XML vectors in `mipmap-*/ic_launcher.xml`.
