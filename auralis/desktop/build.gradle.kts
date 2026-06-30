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
    implementation("org.jetbrains.skiko:skiko-awt-runtime-linux-x64:0.8.21")
    implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.7.3")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-swing:1.9.0")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-jdk8:1.9.0")
}

compose.desktop {
    application {
        mainClass = "com.auralis.desktop.MainKt"

        nativeDistributions {
            targetFormats(TargetFormat.Dmg, TargetFormat.Msi, TargetFormat.Deb)
            packageName = "Auralis"
            packageVersion = "1.0.0"
            description = "Offline Music Player"
            vendor = "Auralis"

            linux {
                iconFile.set(project.file("icon.png"))
            }

            macOS {
                iconFile.set(project.file("icon.icns"))
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
