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
