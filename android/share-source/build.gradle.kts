plugins {
    id("com.android.application") version "9.4.1"
}

android {
    namespace = "app.kinrelay.sharesource"
    compileSdk {
        version = release(37)
    }
    defaultConfig {
        applicationId = "app.kinrelay.sharesource"
        minSdk = 29
        targetSdk = 37
        versionCode = 1
        versionName = "1.0"
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_11
        targetCompatibility = JavaVersion.VERSION_11
    }
}
