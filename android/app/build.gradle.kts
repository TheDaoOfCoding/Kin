import java.util.Properties

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.compose)
    alias(libs.plugins.ksp)
}

// Release signing. The keystore and its passwords are never in the repo: they come from the environment
// (KIN_KEYSTORE_FILE, KIN_KEYSTORE_PASSWORD, KIN_KEY_ALIAS, KIN_KEY_PASSWORD) or from
// %USERPROFILE%\.kinrelay\signing.properties (storeFile, storePassword, keyAlias, keyPassword; a relative
// storeFile is relative to that folder). With neither, the release APK is built unsigned.
val kinHome = File(System.getProperty("user.home"), ".kinrelay")
val signingFile = File(kinHome, "signing.properties")
val signingFromFile = Properties().apply { if (signingFile.isFile) signingFile.inputStream().use { load(it) } }
fun signingValue(env: String, key: String): String? =
    System.getenv(env)?.takeIf { it.isNotBlank() } ?: signingFromFile.getProperty(key)?.takeIf { it.isNotBlank() }
val signingStore = signingValue("KIN_KEYSTORE_FILE", "storeFile")?.let { path ->
    File(path).let { if (it.isAbsolute) it else File(kinHome, path) }
}
val signingStorePassword = signingValue("KIN_KEYSTORE_PASSWORD", "storePassword")
val signingAlias = signingValue("KIN_KEY_ALIAS", "keyAlias")
val signingKeyPassword = signingValue("KIN_KEY_PASSWORD", "keyPassword") ?: signingStorePassword
// Opt-in ABIs for the smoke build type (see buildTypes): -PkinSmokeAbis=x86_64 (comma separated).
val kinSmokeAbis = (findProperty("kinSmokeAbis") as String?)?.split(',')?.map { it.trim() }?.filter { it.isNotEmpty() } ?: emptyList()
// Auto-send screenshots (2B.4, risk R18): on by default. If Google Play refuses the photo permission, a Play build is made with `-PkinAutoSendShots=false`: the switch and the watcher are gone
// (BuildConfig.AUTO_SEND_SHOTS) and the manifest overlay `src/noshots` removes READ_MEDIA_IMAGES and READ_EXTERNAL_STORAGE. The sideload/GitHub build keeps the feature. One codebase.
val kinAutoSendShots = (findProperty("kinAutoSendShots") as String?)?.toBooleanStrictOrNull() ?: true
val releaseSigningReady = signingStore != null && signingStore.isFile && signingStorePassword != null && signingAlias != null && signingKeyPassword != null

android {
    namespace = "app.kinrelay.android"
    compileSdk {
        version = release(37)
    }

    defaultConfig {
        applicationId = "app.kinrelay.android"
        minSdk = 29
        targetSdk = 37
        versionCode = 2
        versionName = "0.1.1"
        buildConfigField("boolean", "AUTO_SEND_SHOTS", kinAutoSendShots.toString())

        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
    }

    signingConfigs {
        if (releaseSigningReady) {
            create("release") {
                storeFile = signingStore
                storePassword = signingStorePassword
                keyAlias = signingAlias
                keyPassword = signingKeyPassword
            }
        }
    }

    buildTypes {
        release {
            // Phones are ARM. The release APK carries no x86 or x86_64 copy of the few native libraries that CameraX and DataStore bring
            // (the emulator runs the debug build, which keeps all of them).
            ndk { abiFilters += listOf("arm64-v8a", "armeabi-v7a") }
            signingConfig = if (releaseSigningReady) signingConfigs.getByName("release") else null
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro"
            )
        }
        // Smoke-test only, opt-in: `gradlew assembleSmoke -PkinSmokeAbis=x86_64` builds the release variant (same R8, shrink, proguard and signing)
        // plus the given ABIs, so the minified code can run on an x86_64 emulator. Without the property this build type does not exist, and
        // `release` stays ARM-only. Output: app/build/outputs/apk/smoke/app-smoke.apk (never app-release.apk).
        if (kinSmokeAbis.isNotEmpty()) {
            create("smoke") {
                initWith(getByName("release"))
                matchingFallbacks += listOf("release")
                // Its own application ID, so a smoke APK can never install over (or be mistaken for) the real app: app.kinrelay.android.smoke.
                // The classes keep the namespace app.kinrelay.android (`am start -n app.kinrelay.android.smoke/app.kinrelay.android.MainActivity`).
                applicationIdSuffix = ".smoke"
                ndk { abiFilters.clear(); abiFilters += listOf("arm64-v8a", "armeabi-v7a") + kinSmokeAbis }
            }
        }
    }
    if (kinSmokeAbis.isNotEmpty()) {
        // The release-only sources (the empty DebugHooks) are part of the smoke build, as they are of the release.
        sourceSets.getByName("smoke").kotlin.directories.add("src/release/java")
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_11
        targetCompatibility = JavaVersion.VERSION_11
    }
    buildFeatures {
        compose = true
        buildConfig = true
    }
    sourceSets {
        if (!kinAutoSendShots) {
            getByName("debug").manifest.srcFile("src/noshots/AndroidManifest.xml")
            getByName("release").manifest.srcFile("src/noshots/AndroidManifest.xml")
        }
        // The shared protocol vectors are loaded by the unit tests as classpath resources.
        getByName("test").resources.directories.add("../../protocol")
        // The instrumented link-display test runs the same vectors on android.icu (a device or an emulator), as assets of the test APK.
        getByName("androidTest").assets.directories.add("../../protocol")
    }
}

dependencies {
    implementation(libs.androidx.core.ktx)
    implementation(libs.androidx.lifecycle.runtime.ktx)
    implementation(libs.androidx.activity.compose)
    implementation(platform(libs.androidx.compose.bom))
    implementation(libs.androidx.compose.ui)
    implementation(libs.androidx.compose.ui.graphics)
    implementation(libs.androidx.compose.ui.tooling.preview)
    implementation(libs.androidx.compose.material3)
    implementation(libs.okhttp)
    implementation(libs.kotlinx.serialization.json)
    implementation(libs.kotlinx.coroutines.android)
    implementation(libs.room.runtime)
    ksp(libs.room.compiler)
    implementation(libs.androidx.datastore.preferences)
    // QR scanning without Google services: CameraX for the preview and the frames, ZXing (Apache-2.0, local only) to decode.
    implementation(libs.zxing.core)
    implementation(libs.androidx.camera.core)
    implementation(libs.androidx.camera.camera2)
    implementation(libs.androidx.camera.lifecycle)
    implementation(libs.androidx.camera.view)

    testImplementation(libs.junit)
    testImplementation(libs.kotlinx.coroutines.test)
    testImplementation(libs.okhttp.mockwebserver)
    testImplementation(libs.okhttp.tls)
    // A real SQLite on the JVM, for the migration test (the unit tests have no Android SQLite).
    testImplementation(libs.sqlite.bundled.jvm)
    // `android.icu` is only a stub on the JVM: the link-display unit tests run the same UTS 46 algorithm on ICU4J (the instrumented test uses the real android.icu).
    testImplementation(libs.icu4j)
    androidTestImplementation(libs.androidx.junit)
    androidTestImplementation(libs.androidx.espresso.core)
    androidTestImplementation(platform(libs.androidx.compose.bom))
    androidTestImplementation(libs.androidx.compose.ui.test.junit4)
    debugImplementation(libs.androidx.compose.ui.tooling)
    debugImplementation(libs.androidx.compose.ui.test.manifest)
}

ksp {
    // Every database version is exported and checked in: the migration tests read the old schemas from here.
    arg("room.schemaLocation", "$projectDir/schemas")
}

// Said once when a release is built without a signing setup (so nobody ships an unsigned APK by accident).
if (!releaseSigningReady && gradle.startParameter.taskNames.any { it.contains("release", ignoreCase = true) || it.contains("smoke", ignoreCase = true) }) {
    logger.lifecycle(
        "NOTE: no release signing setup found, so the release APK is built UNSIGNED and cannot be installed.\n" +
            "      Create $signingFile (storeFile, storePassword, keyAlias, keyPassword) or set KIN_KEYSTORE_FILE,\n" +
            "      KIN_KEYSTORE_PASSWORD, KIN_KEY_ALIAS and KIN_KEY_PASSWORD. See android/README.md, \"Release build\"."
    )
}
