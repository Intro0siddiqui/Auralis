package com.auralis.desktop.service

import java.io.File
import java.io.InputStream
import java.nio.file.Files
import java.nio.file.attribute.PosixFilePermissions

class BundledBinaryManager {

    private val tempDir: File by lazy {
        val dir = File(System.getProperty("java.io.tmpdir"), "auralis-binaries")
        dir.mkdirs()
        dir
    }

    fun getBinaryPath(name: String): String? {
        val binaryFile = extractBinary(name) ?: return null
        return binaryFile.absolutePath
    }

    private fun extractBinary(name: String): File? {
        val binaryFile = File(tempDir, name)

        if (binaryFile.exists() && binaryFile.canExecute()) {
            return binaryFile
        }

        val resourcePath = "/bin/$name"
        val inputStream: InputStream? = javaClass.getResourceAsStream(resourcePath)
            ?: return null

        inputStream.use { input ->
            Files.copy(input, binaryFile.toPath())
        }

        if (System.getProperty("os.name").lowercase().contains("linux") ||
            System.getProperty("os.name").lowercase().contains("mac")) {
            val perms = PosixFilePermissions.fromString("rwxr-xr-x")
            Files.setPosixFilePermissions(binaryFile.toPath(), perms)
        }

        return binaryFile
    }

    fun cleanup() {
        tempDir.listFiles()?.forEach { it.delete() }
        tempDir.delete()
    }
}
