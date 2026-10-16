# Milestone 1.5, Android: the open-source scanner and the signed release APK

Date: 2026-10-15. Built by the Android agent (not yet reviewed or committed).

What changed: (1) the Google ML Kit code scanner is gone, replaced by ZXing on CameraX (D16); (2) the release APK is signed with a key kept outside the repo and minified with R8; (3) the signed APK was smoke-tested on the Android 10 and Android 17 emulators against the real Windows app; (4) install instructions for the friends test.

## 1. The scanner: choice and size

| Option | Verdict |
|---|---|
| Google ML Kit code scanner (was in 1.1 to 1.4) | **Removed.** It pulls in `datatransport`, `firebase` components, `play-services-mlkit` and `GoogleApiActivity`. Conflicts with D10 and does not work without Play services |
| `zxing-android-embedded` 4.3.0 | Rejected: last release October 2021 (the repo's last push was in 2024), built on the old `android.hardware.Camera` API |
| **ZXing core 3.5.4 + CameraX 1.6.2** | **Chosen.** ZXing core: Apache-2.0, pure Java, no network code, released 2025-11-11 (the official `zxing/zxing` releases and Maven Central). CameraX 1.6.2 is the latest stable (1.7.0 is still alpha). Both from official repositories (Maven Central, Google Maven) |

Dependencies added: `com.google.zxing:core`, `androidx.camera:camera-core`, `camera-camera2`, `camera-lifecycle`, `camera-view` (all 1.6.2). Removed: `play-services-code-scanner` and the explicit `androidx.fragment` pin (it only existed because the scanner pulled in `fragment 1.0.0`).

How it works: `MainActivity.scan()` asks for `CAMERA` **only when the user taps "Scan code"**, then starts `ScanActivity` (CameraX preview + `ImageAnalysis`, about 720p, latest frame only, back camera). Each frame's Y plane goes to `QrDecoder` (ZXing `QRCodeReader`, QR only, `TRY_HARDER`, a second try with inverted luminance for light-on-dark codes). The first code comes back as text and goes into the **in-app** pairing flow exactly like a pasted link (no confirmation screen: the tap on Scan is the confirmation). Nothing is stored and no frame leaves the class.

Denial: the pairing screen shows a red message and the paste field and Pair button keep working. First denial: "Kin needs the camera only to read the code. Tap Scan code again to allow it, or paste the pairing link instead." After "don't ask again": "Camera access is off for Kin. Turn it on in the phone's settings, or paste the pairing link instead." The camera and autofocus are `uses-feature required="false"` (checked with `aapt2 dump badging`: `uses-feature-not-required`).

**Size impact** (release, R8, same signing state aside):

| | Bytes | MiB |
|---|---|---|
| Release APK before (ML Kit via Play services, unsigned) | 3,782,631 | 3.61 |
| Release APK now (ZXing + CameraX, signed) | 4,662,856 | 4.45 |
| Difference | +880,225 | **+0.84** |
| Debug APK now | 46,512,483 | 44.4 (not shipped) |

(The ML Kit build was small only because the scanner itself lives in Play services on the phone.) The release APK is far below the ~10 MB target. Its native code is four small libraries (CameraX image processing, the graphics path, the DataStore counter) for four ABIs.

## 2. Telemetry scan: before and after

Tool: `android\tools\check-release.ps1` (aapt2 `dump xmltree` of the manifest inside the APK, a byte scan of every `classes*.dex`, a scan of the APK's file names, and `apksigner`). The dependency tree was also checked with `gradlew :app:dependencies --configuration releaseRuntimeClasspath`.

**Before** (release APK of 1.4, ML Kit scanner):

- Manifest: `GmsBarcodeScanningDelegateActivity`, `MlKitComponentDiscoveryService`, `MlKitInitProvider`, `VisionCommonRegistrar`, `CommonComponentRegistrar` (both `com.google.firebase.components:` entries), `com.google.android.gms.common.api.GoogleApiActivity`, `com.google.android.gms.version`, and **`com.google.android.datatransport.runtime.backends.TransportBackendDiscovery`, `CctBackendFactory`, `JobInfoSchedulerService`, `AlarmManagerSchedulerBroadcastReceiver`**. 11 forbidden names in total.
- Dex: `datatransport` x6, `firebase` x6, `com/google/android/gms` x16, `com.google.android.gms` x50, `mlkit` x17, `com/google/mlkit` x5, `vision/codescanner` x1.
- Files in the APK: `firebase-annotations.properties`, `firebase-components.properties`, `firebase-encoders-json.properties`, `firebase-encoders.properties`.

**After** (this release APK):

- Manifest (57 names): **none** of `gms`, `firebase`, `datatransport`, `mlkit`. The names are Kin's own, `androidx.*` initialisers, `okhttp3` and Room.
- Dex: none of `datatransport`, `firebase`, `play-services-mlkit`, `com/google/android/gms`, `mlkit`, `com/google/mlkit`, `vision/codescanner`, `googleapis.com`, `app-measurement`, `crashlytics`. **One allowed string:** `com.google.android.gms.org.conscrypt`, which OkHttp compares with a security provider's name to detect Conscrypt. It is a name in a comparison, not a Google component; the script lists it as "allowed".
- Files: none matching `gms`, `firebase`, `datatransport`, `mlkit`, `google`, except `META-INF/com.google.dagger_dagger.version`, a 5-byte version marker of Dagger (a code-generation library that CameraX uses; no network, no telemetry; allowed in the script).
- Dependency tree: the only `com.google.*` libraries are `zxing:core`, `guava`/`failureaccess`/`listenablefuture` (collections and futures), `dagger`, `auto-value-annotations`, `error_prone_annotations`, `j2objc-annotations`, `jsr305`/findbugs (annotations). No `com.google.android.gms`, `firebase`, `mlkit` or `datatransport` artifact.
- Permissions in the release manifest: `INTERNET`, `ACCESS_NETWORK_STATE`, `CHANGE_NETWORK_STATE`, `FOREGROUND_SERVICE`, `FOREGROUND_SERVICE_CONNECTED_DEVICE`, `ACCESS_LOCAL_NETWORK`, `POST_NOTIFICATIONS`, **`CAMERA`**, `RECEIVE_BOOT_COMPLETED`, `WAKE_LOCK`.

Result: **clean**. The scan is repeatable: `android\tools\check-release.ps1`.

## 3. Signing

- **Where:** `%USERPROFILE%\.kinrelay\release.jks` (a PKCS12 keystore) and `%USERPROFILE%\.kinrelay\signing.properties` next to it. Both are **outside the repo** (`*.jks` and `*.keystore` are also in `.gitignore`). The folder is limited to the current Windows user (`icacls`, no admin needed). Made by `android\tools\make-keystore.ps1`, which refuses to touch an existing key.
- **Key:** RSA 4096, `SHA256withRSA`, subject `CN=Kin Relay, O=Kin Relay`, valid 10,000 days (until **2054-03-02**), alias `kinrelay`. The password is 32 random characters from a cryptographic generator, written only to `signing.properties` (never printed, never in a log; keytool got it through an environment variable, not the command line).
- **How Gradle finds it:** `signing.properties` (`storeFile`, `storePassword`, `keyAlias`, `keyPassword`; a relative `storeFile` is relative to `.kinrelay`), or the environment (`KIN_KEYSTORE_FILE`, `KIN_KEYSTORE_PASSWORD`, `KIN_KEY_ALIAS`, `KIN_KEY_PASSWORD`). Missing: the release build still runs, prints a clear `NOTE: no release signing setup found ... UNSIGNED` message, and writes `app-release-unsigned.apk`. Nothing secret is in the repo.
- **Version:** `versionCode 1`, `versionName 0.1.0`. The app reports `BuildConfig.VERSION_NAME` as `appVersion` in `hello` (and in the pairing request), so the PC shows 0.1.0 (`LinkSessionTest` and `PairingClientTest` assert it).

> **WARNING: back up the `.kinrelay` folder now** (both files), on something private (an encrypted drive, or a password manager's file storage). **If this key is lost, no future update can install over an existing install**: Android refuses an APK with another signature. The only way out is to uninstall Kin first, which deletes the pairing, the queue and the history. **If someone else gets the key**, they can publish an update that installs over Kin. Never put it in the repo, a chat or an email. (R16)

**Certificate SHA-256** (`apksigner verify --print-certs`):

```
83fd45c8402978c061c784d1f5dd5f7f1cc91c6165faa89a767c5bc19ec7d419
```

SHA-1 `652fb7c8b9a1232303414e98a94a153be2bba0fd`. Signed with scheme v2 (enough for minSdk 29). `apksigner verify` says "Verifies", one signer. Friends can check the same value with `apksigner verify --print-certs Kin-0.1.0.apk`.

## 4. The release APK

- File: `android\app\build\outputs\apk\release\app-release.apk` (ignored by git; `*.apk` is in `.gitignore`). Copy it as `Kin-0.1.0.apk` when handing it out.
- Size: **4,662,856 bytes (4.45 MiB)**. Target was about 10 MB.
- minSdk 29, targetSdk 37, `versionName 0.1.0`, not debuggable, no debug receiver (`src/debug` is not in release), no `AUTO_CONFIRM`.

## 5. Release smoke test (real `Kin.exe` in dev mode, emulators)

Script: `android\tools\release-smoke.ps1 [-Avd kin_api29]`. It installs the **signed release APK** after uninstalling the debug build (the signatures differ), starts a copy of `Kin.exe` with `--dev-bind 127.0.0.1 --dev-qr-address 10.0.2.2` and a throwaway data directory, and uses only what a user or `adb` can do: `am start -d '<url>'`, a tap on Confirm found with uiautomator, the share intent, notifications. The PC clipboard is saved and put back; the emulator is stopped at the end.

| Check (34 per run) | Android 17 (kin_api37) | Android 10 (kin_api29) |
|---|---|---|
| Install the release APK; not debuggable; versionName 0.1.0, versionCode 1 | pass | pass |
| `CAMERA` not granted and not asked at start-up | pass | pass |
| Tap "Scan code": the system asks for the camera | pass | pass |
| Deny: the message appears, the paste field and Pair button stay | pass | pass |
| Tap "Scan code" again: asked again; Allow: the scan screen opens, the preview runs 4 s, the app stays alive | pass | pass |
| Cancel closes the scan screen | pass | pass |
| The link opens the confirmation screen; Confirm pairs; the PC lists the phone | pass | pass |
| Ping PC to phone | 4.6 ms | 2.2 ms |
| Text phone to PC (share intent), and it is on the PC clipboard | pass | pass |
| Text PC to phone (acked) and the phone's notification | pass | pass |
| File PC to phone (700,000 bytes): in `Download/Kin`, SHA-256 equal | pass | pass |
| File phone to PC (500,000 bytes, through the share intent): PC finished, SHA-256 equal | pass | pass |
| No crash, `ClassNotFoundException`, `NoSuchMethodError` etc. in logcat; process still alive | pass | pass |

Both ran with the final APK (the one whose SHA-256 is above): **34 of 34 on kin_api37 and 34 of 34 on kin_api29.** Screenshots of the runs (pairing screen, camera prompt, denial message, scan screen, confirmation) are in the temp folder of the run, not committed.

What the smoke did **not** do: scan a real QR code with a camera. The emulators' virtual camera shows a scene with no QR code. The decode logic is covered by unit tests (section 7). A real camera is the first thing to check on a real phone (R17).

Notes from building the script, for whoever extends it:
- `am start --grant-read-uri-permission ... --eu EXTRA_STREAM <uri>` is not enough on its own: `am` does not copy `EXTRA_STREAM` into the clip data, so the share activity had no grant ("start with files refused: SecurityException"). Adding the same URI as the intent data (`-d <uri>`) fixes it.
- `adb shell content query --where "..."` loses its quotes when the command goes through PowerShell arguments: pass it through stdin.
- uiautomator output is not UTF-8 safe through `adb shell cat` in PowerShell 5.1 ("Don’t allow" arrives mangled), so buttons are matched with a regular expression.
- A second emulator started right after another stopped can report "device offline" for a moment: the script now waits until the device answers.

## 6. R8 sanity

The release build is minified and resource-shrunk (`isMinifyEnabled`, `isShrinkResources`) with the default `proguard-android-optimize.txt` and the app's own `proguard-rules.pro`, which is still empty. **No new keep rule was needed.**

| Library | Result in the signed release |
|---|---|
| kotlinx.serialization | The app only uses `Json` trees (`JsonObject`, `JsonPrimitive`); there is no `@Serializable` class, so there is nothing for R8 to break. Pairing, hello, text and file messages worked |
| Room 3 | Database opened, devices, texts and transfers written and read (pairing, text, files); the generated `_Impl` classes survive through Room's consumer rules |
| OkHttp, TLS pinning, WebSocket | Pairing request, the TLS 1.3 connection, ping |
| DataStore, Keystore identity | Identity created, settings stored |
| CameraX | The preview ran for 4 s on both emulators and bound the analysis use case with no error |
| ZXing | No reflection in the library. **Not exercised by a real code in the release build** (see above); the unit tests exercise the same source. R8 renames and keeps the classes it uses |

No `R8` warnings about missing classes in the build output, and logcat of both runs has no `FATAL EXCEPTION` or `ClassNotFoundException`.

## 7. Tests and checks

| Check | Result |
|---|---|
| `gradlew.bat lintDebug testDebugUnitTest assembleDebug assembleRelease` | **passes**; lint: "No issues found" |
| Unit tests | **457 run, 0 failures, 0 errors, 1 skipped** (42 suites). The skipped one is `FakePcSession.serve`, a test helper that JUnit picks up and skips by an assumption, as in 1.4. 13 new tests, in `QrDecoderTest` |
| `bun protocol/check-vectors.ts` | OK: all 6387 checks passed |
| `tools/e2e-1-4.ps1` (debug build, kin_api37) | **OK: all 179 checks passed.** (A first attempt failed at "desktop started" because another agent had its own `Kin.exe` and emulator running at the same time; run again when they were gone, it passed) |

`QrDecoderTest` (13): encodes real `kinrelay://pair?...` links with the ZXing writer and decodes them from camera-style frames: a Desk PC link with two addresses and a name (and that the text then parses as a valid pairing link), **every canonical pairing URL of the shared vectors**, the `https://.../pair#...` form, rows wider than the frame (`rowStride` 704 for width 640), a frame rotated 90 degrees, a frame upside down, a light code on a dark background, a low-contrast code, a small code (3 pixels per module in 1280x720), ARGB bitmap pixels, an empty frame and random noise (no code), a truncated buffer (no crash), a Code 128 bar code (ignored), and 20 empty frames followed by the code (no state left over).

## 8. Neutral names

`grep -ril "<PC name>|<user name>" android/` (sources, tests, resources, scripts): no hit in tracked files. The only hit is `android/local.properties` (the SDK path with the Windows user name), which is gitignored and never committed. Test sources already use neutral names (`Desk PC`, `Anna Berg`).

## 9. Install instructions (for `docs/TESTING.md`)

Copy this section as it is.

### Get the APK

The file is `android\app\build\outputs\apk\release\app-release.apk` on the build PC. Rename a copy to `Kin-0.1.0.apk` and give that to your friends (a messaging app, a cloud drive, a USB cable). It is 4.5 MB. Its certificate fingerprint (SHA-256) is `83fd45c8402978c061c784d1f5dd5f7f1cc91c6165faa89a767c5bc19ec7d419`; anyone can compare it with `apksigner verify --print-certs Kin-0.1.0.apk` (Android SDK build-tools).

### Install it on the phone (sideload)

Needs Android 10 or newer.

1. Copy `Kin-0.1.0.apk` to the phone (or download it there).
2. Open it from the Files app (or "Downloads"). Android asks you to allow installs from that app: tap **Settings**, turn on **Allow from this source** for that file manager (or the browser), go back, and tap **Install**. (On Samsung and some other phones the switch is under Settings, Apps, Special access, **Install unknown apps**.)
3. Or over USB: turn on Developer options and USB debugging on the phone, plug it in and run `adb install Kin-0.1.0.apk` (use `adb install -r` to update).

### The Play Protect warning

Google Play Protect may say "**Blocked by Play Protect**" or "**App not scanned**" / "unknown developer", because Kin is not from the Play Store and comes from a developer Google does not know. This is expected for a test build. Tap **More details**, then **Install anyway**. If the dialog only offers "OK", open Play Store, then your profile picture, **Play Protect**, the gear icon, and turn off "Scan apps with Play Protect" while you install (turn it back on afterwards), or install with `adb install`. Kin collects and sends nothing to Google or anyone else.

### First run: what Kin asks for

| Asked | When | Why | If you say no |
|---|---|---|---|
| **Notifications** (Android 13+) | At the first start | Texts and files from the PC, and the "Connected to ..." notification | You will not see arrivals; Settings, Apps, Kin, Notifications turns it on |
| **Nearby devices** (Android 17 only) | At the first start and before pairing | Android 17 blocks every connection to a PC on your Wi-Fi without it | Kin cannot reach the PC and says so on Home; Settings, Apps, Kin, Permissions, Nearby devices |
| **Camera** | **Only when you tap "Scan code"** | To read the QR code the PC shows. The picture is not saved or sent | Tap "Scan code" again to allow it. **Pasting a pairing link is the other way** (the PC window had only the QR code when this was written; since v0.1.1 it has a "Copy pairing link" button, fixed) |

Pair: on the PC open Kin, choose "Pair new phone", then on the phone tap "Scan code" and point the camera at the code.

### Battery: let Kin keep running

Kin keeps a small connection to the PC. Phone makers stop background apps to save battery; on some brands that stops PC to phone sends. Kin's setup screen (Settings, Setup) opens the right screen for your phone; the steps are:

| Brand | What to do (the same steps as Kin's setup screen) |
|---|---|
| **Samsung** | Settings, Battery, Background usage limits. Open "Never sleeping apps" and add Kin. Turn off "Put unused apps to sleep", or make sure Kin isn't in the sleeping apps list |
| **Xiaomi, Redmi, POCO (MIUI, HyperOS)** | Open Autostart and turn Kin on. Open Kin's battery saver and choose "No restrictions". In the recent apps list, press and hold Kin's card and tap the lock, so it isn't cleared |
| **Oppo, Realme, OnePlus (ColorOS, OxygenOS)** | Open the startup manager and allow Kin to start automatically. Open Kin's battery settings and allow background activity (turn off "Optimise battery use"). In the recent apps list, pull Kin's card down (or tap the lock) |
| **Vivo, iQOO (Funtouch OS, OriginOS)** | Open the autostart list (i Manager, App manager) and turn Kin on. Open the battery settings, then "Background power consumption", and allow Kin |
| **Huawei, Honor (EMUI, MagicOS)** | Open App launch, find Kin and switch it to "Manage manually". Turn on Auto-launch, Secondary launch and Run in background for Kin |
| **Google Pixel, Motorola, Nokia, other plain Android** | Open the battery optimisation list and set Kin to "Not optimised" (Unrestricted): Settings, Apps, Kin, App battery usage |
Kin's setup screen has a button for each of these and opens the closest settings page the phone has. (No real OEM phone has been tested yet: the screens are collected from the field guides, and checking them on the friends' phones is part of the test.) Also: do not swipe Kin away from Recents on aggressive brands, and keep Wi-Fi on.

### Update later

Install the new `Kin-<version>.apk` over the old one, the same way (no uninstall). It keeps the pairing, the queue and the history **only if it is signed with the same key**: the build PC's `%USERPROFILE%\.kinrelay\release.jks`. A build signed with another key will not install over it; Android says "App not installed", and the only way is to uninstall first (which loses the pairing). **Back up `%USERPROFILE%\.kinrelay` (both files).** Each build that anyone installs needs a higher `versionCode` than the one before (set in `android/app/build.gradle.kts`).

### Uninstall

Settings, Apps, Kin, Uninstall. This removes the pairing on the phone; on the PC, open Kin's pairing window and use "Remove this device".

## 10. Files changed (Android side of 1.5)

- `android/gradle/libs.versions.toml`, `android/app/build.gradle.kts` (scanner dependencies, signing, the unsigned-build note)
- `android/app/src/main/AndroidManifest.xml` (`CAMERA`, optional camera features, `ScanActivity`)
- `android/app/src/main/java/app/kinrelay/android/scan/` (`QrDecoder`, `ScanActivity`, `ScanContract`), `MainActivity.kt` (the camera flow)
- `android/app/src/test/java/app/kinrelay/android/scan/QrDecoderTest.kt`
- `android/tools/make-keystore.ps1`, `check-release.ps1`, `release-smoke.ps1`
- `android/README.md`, `docs/android.md` (permissions table and libraries), `docs/decisions.md` (D16), `docs/risks.md` (R16, R17), this file

## 11. Open points

- **(Fixed in v0.1.1: the PC window now has a "Copy pairing link" button, so the pasted-link path has something to paste. What follows is how it was when this was written.) The PC window shows only the QR code, no pairing link.** If a friend's camera does not work (or they deny it, or the phone has none), the pasted-link path the app offers has nothing to paste: the PC would need a "Copy pairing link" button next to the code (a desktop change, not done here). Until then the only fallback is opening the link with `adb`. Worth adding before the friends test.
- **A real camera has not scanned a real code yet** (R17). First check on a real phone; fixes if it is poor: a bigger code on the PC, tap to focus, or a "scan from a picture" button.
- The release APK carries libraries for four ABIs. At 4.45 MiB there is no need to split; if size ever matters, `ndk.abiFilters` to `arm64-v8a` and `armeabi-v7a` drops the two x86 sets.
- The friends need the key to be the same for the whole test: if the key is replaced after the first install, every friend must uninstall.
