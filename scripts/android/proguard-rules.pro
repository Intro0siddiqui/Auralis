# ==============================================================================
# Auralis Android ProGuard / R8 Rules
# ==============================================================================
# Preserves classes, methods, and companion objects invoked dynamically from
# Rust via JNI (libauralis_lib.so). Without these rules, R8 treats reflection/JNI
# targets as dead code and removes or obfuscates them in release builds.
# ==============================================================================

# Keep any class or member annotated with @Keep
-keep @androidx.annotation.Keep class * { *; }
-keepclassmembers class * {
    @androidx.annotation.Keep *;
}

# MediaPlaybackService (Foreground Service, Notification & MediaSession)
-keep class com.auralis.v2.MediaPlaybackService { *; }
-keep class com.auralis.v2.MediaPlaybackService$* { *; }
-keepclassmembers class com.auralis.v2.MediaPlaybackService {
    public static ** *(...);
    public ** *(...);
    <fields>;
}
-keepclassmembers class com.auralis.v2.MediaPlaybackService$* {
    public static ** *(...);
    public ** *(...);
    <fields>;
}

# MainActivity (Permission dispatch & Activity lifecycle)
-keep class com.auralis.v2.MainActivity { *; }
-keep class com.auralis.v2.MainActivity$* { *; }
-keepclassmembers class com.auralis.v2.MainActivity {
    public static ** *(...);
    public ** *(...);
    <fields>;
}
-keepclassmembers class com.auralis.v2.MainActivity$* {
    public static ** *(...);
    public ** *(...);
    <fields>;
}

# NativeBridge (Rust -> Kotlin -> Rust command dispatcher)
-keep class com.auralis.v2.NativeBridge { *; }
-keepclassmembers class com.auralis.v2.NativeBridge {
    public static ** *(...);
    public ** *(...);
    <fields>;
}

# MediaStoreScanner (Android SAF / MediaStore query engine)
-keep class com.auralis.v2.MediaStoreScanner { *; }
-keepclassmembers class com.auralis.v2.MediaStoreScanner {
    public static ** *(...);
    public ** *(...);
    <fields>;
}

# Preserve all native method declarations across all classes
-keepclasseswithmembernames class * {
    native <methods>;
}
