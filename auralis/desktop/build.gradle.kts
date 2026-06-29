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
