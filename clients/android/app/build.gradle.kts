plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "io.palm.client"
    compileSdk = 35

    defaultConfig {
        applicationId = "io.palm.client"
        minSdk = 26
        targetSdk = 35
        versionCode = 1
        versionName = "0.1.0"
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions { jvmTarget = "17" }
}

dependencies {
    implementation("androidx.core:core-ktx:1.15.0")
    implementation("androidx.appcompat:appcompat:1.7.0")
    // WebSocket（Android 无 JDK java.net.http，用 OkHttp）
    implementation("com.squareup.okhttp3:okhttp:4.12.0")
    // 应用内扫码（自带相机扫码 Activity + zxing 解码，无需 Play 服务）
    implementation("com.journeyapps:zxing-android-embedded:4.3.0")
}
