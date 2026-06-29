# Add project specific ProGuard rules here.
# Keep SQLDelight generated code
-keep class com.auralis.database.** { *; }

# Keep model classes (used via reflection in serialization)
-keep class com.auralis.model.** { *; }
