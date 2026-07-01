import org.jetbrains.compose.desktop.application.dsl.TargetFormat

plugins {
    kotlin("jvm")
    id("org.jetbrains.compose")
    id("org.jetbrains.kotlin.plugin.compose")
}

dependencies {
    implementation(project(":shared"))
    implementation(compose.material3)
    implementation(compose.components.resources)
    implementation(compose.desktop.common)
    implementation(compose.desktop.currentOs)
    implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.7.3")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-swing:1.9.0")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-jdk8:1.9.0")
}

compose.desktop {
    application {
        mainClass = "com.auralis.desktop.MainKt"

        nativeDistributions {
            targetFormats(TargetFormat.Dmg, TargetFormat.Msi, TargetFormat.Deb, TargetFormat.Rpm)
            packageName = "Auralis"
            packageVersion = "1.0.0"
            description = "Offline Music Player"
            vendor = "Auralis"

            linux {
                iconFile.set(project.file("icon.png"))
            }

            macOS {
                bundleID = "com.auralis.desktop"
                dmgPackageVersion = "1.0.0"
                dmgPackageBuildVersion = "1"
            }

            windows {
                iconFile.set(project.file("icon.ico"))
                menuGroup = "Auralis"
                upgradeUuid = "a1b2c3d4-e5f6-7890-abcd-ef1234567890"
            }
        }
    }
}

tasks.register("packageAppImage") {
    description = "Creates a standalone app-image with bundled JRE"
    group = "distribution"

    dependsOn("jar")

    doLast {
        val jarFile = tasks.jar.get().archiveFile.get().asFile
        val outputDir = layout.buildDirectory.dir("app-image").get().asFile
        val os = System.getProperty("os.name").lowercase()
        val arch = System.getProperty("os.arch").lowercase()
        val platform = when {
            os.contains("mac") || os.contains("darwin") -> "mac-${if (arch.contains("aarch64")) "arm64" else "x64"}"
            os.contains("linux") -> "linux-x86_64"
            else -> "unknown"
        }
        val tarName = "Auralis-${platform}.tar.gz"

        outputDir.deleteRecursively()
        outputDir.mkdirs()

        val jpackageArgs = mutableListOf(
            "jpackage",
            "--type", "app-image",
            "--dest", outputDir.absolutePath,
            "--name", "Auralis",
            "--input", jarFile.parentFile.absolutePath,
            "--main-jar", jarFile.name,
            "--main-class", "com.auralis.desktop.MainKt",
            "--icon", file("icon.png").absolutePath,
            "--java-options", "-Xmx512m",
            "--java-options", "-Dfile.encoding=UTF-8"
        )

        println("Running jpackage...")
        println(jpackageArgs.joinToString(" "))

        val process = ProcessBuilder(jpackageArgs)
            .directory(projectDir)
            .redirectErrorStream(true)
            .start()

        val output = process.inputStream.bufferedReader().readText()
        val exitCode = process.waitFor()

        if (exitCode != 0) {
            println(output)
            throw GradleException("jpackage failed with exit code $exitCode")
        }

        println("App image created successfully")

        val appImageDir = outputDir.listFiles()?.firstOrNull()
        if (appImageDir != null) {
            val tarFile = outputDir.resolve(tarName)
            val tarProcess = ProcessBuilder(
                "tar", "czf", tarFile.absolutePath,
                "-C", outputDir.absolutePath,
                appImageDir.name
            ).start()
            val tarExit = tarProcess.waitFor()
            if (tarExit == 0) {
                println("Created: ${tarFile.absolutePath}")
                println("Size: ${tarFile.length() / 1024 / 1024}MB")
            }
        }
    }
}

kotlin {
    jvmToolchain(17)
}

tasks.jar {
    manifest {
        attributes("Main-Class" to "com.auralis.desktop.MainKt")
    }
    from(configurations.runtimeClasspath.get().map { if (it.isDirectory) it else zipTree(it) }) {
        exclude("org/sqlite/native/Android/**")
        exclude("org/sqlite/native/Windows/**")
        exclude("org/sqlite/native/Mac/**")
        exclude("org/sqlite/native/FreeBSD/**")
        exclude("org/sqlite/native/Linux-Android/**")
        exclude("org/sqlite/native/Linux-Musl/**")
        exclude("org/sqlite/native/Linux/arm/**")
        exclude("org/sqlite/native/Linux/aarch64/**")
        exclude("org/sqlite/native/Linux/ppc64/**")
        exclude("org/sqlite/native/Linux/armv6/**")
    }
    duplicatesStrategy = DuplicatesStrategy.EXCLUDE
}
