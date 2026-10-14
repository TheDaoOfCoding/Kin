# Phase 0 results: Android toolchain and hello world

Done 2026-10-14. No phone was connected and the emulator was not started, so success here means "it builds". Nothing has run on a device yet.

Everything was installed for the current user only. No admin rights, no installers, no machine-wide changes.

## Tools installed

| Tool | Version | Location | Source (SHA-256 checked) |
|---|---|---|---|
| JDK (Eclipse Temurin) | 21.0.12.1+1 | `%LOCALAPPDATA%\Programs\Java\jdk-21` | Adoptium API, zip |
| Android command-line tools | build 15859902 | `%LOCALAPPDATA%\Android\Sdk\cmdline-tools\latest` | dl.google.com, zip |
| Platform-Tools (`adb`) | 37.0.1 | `%LOCALAPPDATA%\Android\Sdk\platform-tools` | `sdkmanager` |
| Android SDK Platform | 37.0 (`platforms;android-37.0`) | `%LOCALAPPDATA%\Android\Sdk\platforms\android-37.0` | `sdkmanager` |
| Build-Tools | 37.0.0 | `%LOCALAPPDATA%\Android\Sdk\build-tools\37.0.0` | `sdkmanager` |
| Gradle (via wrapper) | 9.8.1 | `~\.gradle\wrapper\dists` | services.gradle.org |

- Both zips and the Gradle distribution matched the published SHA-256. The generated `gradle-wrapper.jar` matches Gradle's published `gradle-9.8.1-wrapper.jar.sha256`.
- JDK 21 was picked over 17. AGP 9.4 requires 17 or newer, and 21 is the LTS that Gradle 9.8 and Kotlin 2.4 both support.
- SDK licences were accepted, as authorised. The files are in `%LOCALAPPDATA%\Android\Sdk\licenses`.
- Disk use: JDK about 330 MB, SDK about 580 MB, `~\.gradle` about 1.3 GB after the first build.
- Not installed: Android Studio, emulator, system images.

## Environment variables (user scope)

| Variable | Value |
|---|---|
| `JAVA_HOME` | `%LOCALAPPDATA%\Programs\Java\jdk-21` (expanded path) |
| `ANDROID_HOME` | `%LOCALAPPDATA%\Android\Sdk` |
| `Path` (user), appended | `...\Programs\Java\jdk-21\bin`, `...\Android\Sdk\platform-tools`, `...\Android\Sdk\cmdline-tools\latest\bin` |

- The PATH entry for Java is the literal path, not `%JAVA_HOME%\bin`. A literal path works no matter how the PATH value is typed or expanded. (The user PATH here is `REG_EXPAND_SZ`, so `%JAVA_HOME%` would also expand, but that would break if the value were ever rewritten as plain `REG_SZ`.)
- The machine PATH was not touched. Existing user PATH entries were kept.
- Open a new terminal to pick these up. Verified from a fresh environment built from the registry values: `java -version` (Temurin 21.0.12.1), `adb --version` (1.0.41, 37.0.1) and `sdkmanager --list_installed` (the three SDK packages above) all work.
- `sdkmanager` prints a deprecation notice that points to a newer "Android CLI" (`android sdk`). It still works. Worth a look later.

## Versions chosen

All were the latest stable at the time, taken from the official Maven metadata and docs, not from memory. Pre-releases (AGP 9.5 alphas, Kotlin 2.5.0-Beta1, API 37.2 betas) were skipped.

| Item | Version | Checked at |
|---|---|---|
| Android Gradle Plugin | 9.4.1 | `dl.google.com/dl/android/maven2/com/android/tools/build/gradle/maven-metadata.xml`. Requirements: [AGP release notes](https://developer.android.com/build/releases/gradle-plugin): Gradle 9.6.0+, JDK 17+, max API 37, Build-Tools 36.0.0+ |
| Gradle | 9.8.1 | `services.gradle.org/versions/current` |
| Kotlin and Compose compiler plugin | 2.4.21 | Maven Central (`kotlin-gradle-plugin`) and Gradle plugin portal (`org.jetbrains.kotlin.plugin.compose`). 2.5.0-Beta1 is newer but not stable |
| Compose BOM | 2026.09.00 (Material 3 1.4.0) | Google Maven (`androidx/compose/compose-bom`) |
| `androidx.core:core-ktx` | 1.19.1 | Google Maven |
| `androidx.lifecycle:lifecycle-runtime-ktx` | 2.11.0 | Google Maven |
| `androidx.activity:activity-compose` | 1.13.0 | Google Maven |
| JUnit | 4.13.2 | Maven Central |
| `androidx.test.ext:junit` | 1.3.0 | Google Maven |
| Espresso | 3.7.0 | Google Maven |
| compileSdk / targetSdk | 37 (Android 17) | `sdkmanager --list`: `platforms;android-37.0` is stable. AGP 9.4 supports up to API 37 |
| minSdk | 29 | decision D8 |

Notes on the build setup:

- AGP 9 has Kotlin built in, so there is no `org.jetbrains.kotlin.android` plugin. Only the Compose compiler plugin is applied.
- `compileSdk` uses the AGP 9 DSL: `compileSdk { version = release(37) }`.
- Versions are in `android/gradle/libs.versions.toml`. The app uses exactly the libraries the Android Studio "Empty Activity" template uses, and nothing else.
- `platforms;android-37.1` and `37.2` also exist. They were not used because AGP 9.4 documents API 37 as its maximum.

## Build results

Run from `android/`, with the JDK 21 environment above.

| Check | Result | Time |
|---|---|---|
| `gradlew.bat assembleDebug`, first build (downloads the Gradle distribution and all dependencies) | pass | 2 min 1 s |
| `gradlew.bat clean assembleDebug` (warm caches, warm daemon) | pass | 17 s |
| `gradlew.bat clean assembleDebug testDebugUnitTest` (warm caches, cold daemon) | pass | 23 s |
| `gradlew.bat testDebugUnitTest` | pass, 1 test, 0 failures | 5 s |
| Incremental `assembleDebug` after a one-line change to `MainActivity.kt` | pass | 2.7 s |
| `gradlew.bat assembleDebugAndroidTest` (compile only, nothing run) | pass | 12 s |

- **APK:** `android/app/build/outputs/apk/debug/app-debug.apk`, 29,818,790 bytes (about 28.4 MiB). It is a debug build and includes Compose tooling. Release size will differ.
- **Signing:** the default debug keystore at `~\.android\debug.keystore`. No keystores are in the repo.
- **Timings:** one run each, on this PC. Treat them as rough.

### Warnings

- `Unable to strip the following libraries, packaging them as they are: libandroidx.graphics.path.so`. This is normal when the NDK isn't installed. It only affects native debug symbols and is harmless.
- `Configuration.setVisible(boolean) method has been deprecated. This is scheduled to be removed in Gradle 11.` This comes from a plugin (AGP or the Kotlin plugin), not our scripts. Nothing to do until a newer plugin version fixes it.
- Gradle suggests enabling the configuration cache. Not enabled yet.
- No Kotlin or Android lint warnings, and none were suppressed.

## What you need to do at home

1. **Phone:** Settings → About phone → tap the build number 7 times. Then in Developer options, turn on **USB debugging**. On Android 11+ you can also turn on **Wireless debugging** and pair with `adb pair <ip>:<port>`.
2. **Connect it:** plug in by USB and accept the "Allow USB debugging?" prompt. Open a new terminal and run `adb devices`. The phone should be listed as `device`, not `unauthorized`.
3. **Install:** from `android/`, run `gradlew.bat installDebug`. Open the "Kin" app. You should see "Kin" and "Hello from Kin Relay".
4. **Record the phone** (model, Android version, OEM skin, router) in the reference devices table in `docs/targets.md`.
5. **Optional:** install Android Studio (latest stable). It will find the SDK at `%LOCALAPPDATA%\Android\Sdk`, which is already set as `ANDROID_HOME`. Choose JDK 21 when asked, or use its bundled one.

## Rebuild commands

```
cd C:\Dev\ecosystem-app\android
gradlew.bat assembleDebug
gradlew.bat testDebugUnitTest
gradlew.bat installDebug    (phone connected)
```

`android/local.properties` holds `sdk.dir` and is gitignored (see `android/.gitignore`). On another PC, run `sdkmanager "platform-tools" "platforms;android-37.0" "build-tools;37.0.0"` and set `ANDROID_HOME`. Gradle falls back to `ANDROID_HOME` when `local.properties` is missing.
