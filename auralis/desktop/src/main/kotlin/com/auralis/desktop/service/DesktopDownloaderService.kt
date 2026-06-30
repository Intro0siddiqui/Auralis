package com.auralis.desktop.service

import com.auralis.model.AudioFormat
import com.auralis.service.DownloadResult
import com.auralis.service.DownloaderService
import com.auralis.service.MediaMetadata
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.channelFlow
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.*
import java.io.File
import java.util.concurrent.TimeUnit

class DesktopDownloaderService : DownloaderService {

    private val binaryManager = BundledBinaryManager()

    private val musicDir: File by lazy {
        val home = System.getProperty("user.home")
        File(home, "Music/Auralis").apply { mkdirs() }
    }

    override suspend fun isSupportedUrl(url: String): Boolean {
        val lower = url.lowercase()
        return lower.contains("youtube.com") ||
                lower.contains("youtu.be") ||
                lower.contains("instagram.com")
    }

    override suspend fun getMetadata(url: String): MediaMetadata? = withContext(Dispatchers.IO) {
        val ytdlpPath = binaryManager.getBinaryPath("yt-dlp") ?: return@withContext null

        try {
            val process = ProcessBuilder(
                ytdlpPath, "--dump-json", "--no-download", url
            ).redirectErrorStream(true).start()

            val output = process.inputStream.bufferedReader().readText()
            process.waitFor(30, TimeUnit.SECONDS)

            if (process.exitValue() != 0) return@withContext null

            val json = Json.parseToJsonElement(output).jsonObject

            val title = json["title"]?.jsonPrimitive?.content ?: "Unknown"
            val uploader = json["uploader"]?.jsonPrimitive?.content
            val thumbnail = json["thumbnail"]?.jsonPrimitive?.content
            val duration = json["duration"]?.jsonPrimitive?.double?.toLong()
            val webpageUrl = json["webpage_url"]?.jsonPrimitive?.content ?: url
            val extractor = json["extractor"]?.jsonPrimitive?.content ?: "unknown"

            MediaMetadata(
                title = title,
                artist = uploader,
                thumbnailUrl = thumbnail,
                duration = duration,
                source = extractor,
                sourceUrl = webpageUrl
            )
        } catch (e: Exception) {
            null
        }
    }

    override suspend fun downloadAudio(
        url: String,
        format: AudioFormat,
        quality: Int
    ): Flow<DownloadResult> = channelFlow {
        val metadata = try {
            getMetadata(url)
        } catch (e: Exception) {
            null
        }

        val ytdlpPath = binaryManager.getBinaryPath("yt-dlp")
        if (ytdlpPath == null) {
            send(DownloadResult.Error("yt-dlp binary not found in app resources"))
            return@channelFlow
        }

        val ffmpegPath = findFfmpeg()
        if (ffmpegPath == null) {
            send(DownloadResult.Error("ffmpeg not found. Please install ffmpeg."))
            return@channelFlow
        }

        send(DownloadResult.Progress(0f, "Starting download..."))

        try {
            withContext(Dispatchers.IO) {
                val outputTemplate = musicDir.absolutePath + "/%(title)s.%(ext)s"

                val args = mutableListOf(
                    ytdlpPath,
                    "-x",
                    "--audio-format", format.extension,
                    "--audio-quality", quality.toString(),
                    "--newline",
                    "--progress",
                    "--ffmpeg-location", File(ffmpegPath).parent,
                    "-o", outputTemplate,
                    url
                )

                send(DownloadResult.Progress(0.05f, "Extracting audio..."))

                val process = ProcessBuilder(args)
                    .redirectErrorStream(true)
                    .start()

                val reader = process.inputStream.bufferedReader()
                var lastProgress = 0f

                while (true) {
                    val line = reader.readLine() ?: break

                    val progressMatch = Regex("""\[download\]\s+([\d.]+)%""").find(line)
                    if (progressMatch != null) {
                        val percent = progressMatch.groupValues[1].toFloat() / 100f
                        if (percent - lastProgress >= 0.01f) {
                            lastProgress = percent
                            send(DownloadResult.Progress(percent * 0.9f, "Downloading..."))
                        }
                    }

                    if (line.contains("[ExtractAudio]")) {
                        send(DownloadResult.Progress(0.92f, "Converting audio..."))
                    }

                    if (line.contains("[Merger]") || line.contains("[FixupM3u8]")) {
                        send(DownloadResult.Progress(0.95f, "Merging/fixing..."))
                    }
                }

                process.waitFor(600, TimeUnit.SECONDS)

                if (process.exitValue() != 0) {
                    send(DownloadResult.Error("yt-dlp exited with code ${process.exitValue()}"))
                    return@withContext
                }

                val downloadedFile = musicDir.listFiles()
                    ?.filter { it.isFile && it.extension == format.extension }
                    ?.maxByOrNull { it.lastModified() }

                if (downloadedFile == null) {
                    send(DownloadResult.Error("Downloaded file not found"))
                    return@withContext
                }

                send(DownloadResult.Progress(1.0f, "Complete"))

                val finalMetadata = metadata ?: MediaMetadata(
                    title = downloadedFile.nameWithoutExtension,
                    source = "unknown",
                    sourceUrl = url
                )

                send(DownloadResult.Success(downloadedFile.absolutePath, finalMetadata))
            }
        } catch (e: Exception) {
            send(DownloadResult.Error(e.message ?: "Unknown error", e))
        }
    }

    private fun findFfmpeg(): String? {
        val paths = listOf(
            "/usr/bin/ffmpeg",
            "/usr/local/bin/ffmpeg",
            "/opt/homebrew/bin/ffmpeg",
            System.getProperty("user.home") + "/.local/bin/ffmpeg"
        )

        for (path in paths) {
            if (File(path).exists()) return path
        }

        return try {
            val process = ProcessBuilder("which", "ffmpeg")
                .redirectErrorStream(true)
                .start()
            process.waitFor(5, TimeUnit.SECONDS)
            if (process.exitValue() == 0) {
                process.inputStream.bufferedReader().readText().trim()
            } else null
        } catch (e: Exception) {
            null
        }
    }
}
