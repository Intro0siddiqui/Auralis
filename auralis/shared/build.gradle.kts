plugins {
    kotlin("multiplatform")
    kotlin("plugin.serialization")
    id("org.jetbrains.compose")
    id("org.jetbrains.kotlin.plugin.compose")
    id("app.cash.sqldelight")
    id("com.android.library")
}

kotlin {
    androidTarget {
        compilations.all {
            kotlinOptions {
                jvmTarget = "17"
            }
        }
    }

    jvm("desktop")

    sourceSets {
        val commonMain by getting {
            dependencies {
                implementation(compose.runtime)
                implementation(compose.foundation)
                implementation(compose.material3)
                implementation(compose.components.resources)
                implementation("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.9.0")
                implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.7.3")
                implementation("org.jetbrains.kotlinx:kotlinx-datetime:0.6.1")
                implementation("app.cash.sqldelight:coroutines-extensions:2.0.2")
            }
        }

        val commonTest by getting {
            dependencies {
                implementation(kotlin("test"))
            }
        }

        val androidMain by getting {
            dependencies {
                implementation("androidx.core:core-ktx:1.13.1")
                implementation("androidx.media3:media3-exoplayer:1.4.1")
                implementation("androidx.media3:media3-session:1.4.1")
                implementation("app.cash.sqldelight:android-driver:2.0.2")
                implementation("com.google.zxing:core:3.5.3")
                implementation("androidx.camera:camera-core:1.3.4")
                implementation("androidx.camera:camera-camera2:1.3.4")
                implementation("androidx.camera:camera-lifecycle:1.3.4")
                implementation("androidx.camera:camera-view:1.3.4")
                implementation("com.google.mlkit:barcode-scanning:17.2.0")
                implementation("androidx.lifecycle:lifecycle-runtime-ktx:2.8.4")
                implementation("androidx.activity:activity-compose:1.9.1")
            }
        }

        val desktopMain by getting {
            dependencies {
                implementation("org.jetbrains.skiko:skiko:0.8.21")
                implementation("org.jetbrains.kotlinx:kotlinx-coroutines-swing:1.9.0")
                implementation("app.cash.sqldelight:sqlite-driver:2.0.2")
                implementation("com.google.zxing:core:3.5.3")
                implementation("com.github.sarxos:webcam-capture:0.3.12")
            }
        }
    }
}

sqldelight {
    databases {
        create("AuralisDatabase") {
            packageName.set("com.auralis.database")
        }
    }
}

android {
    namespace = "com.auralis.shared"
    compileSdk = 34
    defaultConfig {
        minSdk = 26
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}
