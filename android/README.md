# Kin Android app

Milestone 1.1 (connection spike): identity, pairing, a foreground service that holds one WebSocket to the PC, ping, local log. Milestone 1.2: text, links and the clipboard in both directions. Milestone 1.3: files in both directions (share sheet and direct-share shortcut, upload with progress, receive into MediaStore, resume, cancel). Milestone 1.4: the persistent queue, history, notifications rework, setup (onboarding), settings and debug screens. Milestone 1.5: the signed release APK, and the QR scanner without Google services (ZXing on CameraX, D16). v0.1.1: up to 4 files of a transfer on the wire at once, in both directions ([v0-1-1-android.md](../docs/results/v0-1-1-android.md)). The contract is [docs/protocol.md](../docs/protocol.md); the design is [docs/android.md](../docs/android.md). Details of what was built and measured: [phase-1-1-android.md](../docs/results/phase-1-1-android.md), [phase-1-2-android.md](../docs/results/phase-1-2-android.md), [phase-1-3-android.md](../docs/results/phase-1-3-android.md).

## Build and test

Needs JDK 21 and the Android SDK (`JAVA_HOME`, `ANDROID_HOME`; see [phase-0-android.md](../docs/results/phase-0-android.md)). From this folder:

```
gradlew.bat lintDebug testDebugUnitTest assembleDebug assembleRelease
gradlew.bat --stop
```

- Debug APK: `app/build/outputs/apk/debug/app-debug.apk`. Release (R8, signed): `app/build/outputs/apk/release/app-release.apk` (`app-release-unsigned.apk` when no signing is set up, see "Release build").
- The unit tests load the shared vectors from `../protocol/` as test resources (`app/build.gradle.kts`).

## Release build

The release APK is minified (R8) and signed with a key that is **not in the repo**. Gradle reads it from `%USERPROFILE%\.kinrelay\signing.properties` (`storeFile`, `storePassword`, `keyAlias`, `keyPassword`; a relative `storeFile` is relative to that folder), or from the environment: `KIN_KEYSTORE_FILE`, `KIN_KEYSTORE_PASSWORD`, `KIN_KEY_ALIAS`, `KIN_KEY_PASSWORD`. With neither, `assembleRelease` still works but says so and builds `app-release-unsigned.apk`, which can't be installed.

- **Make the key once:** `android\tools\make-keystore.ps1` (RSA 4096, valid 27 years, a random password written only to `signing.properties`; it never overwrites an existing key).
- **Back up the whole `%USERPROFILE%\.kinrelay` folder now.** **If this key is lost, no update can ever install over an existing install** (Android refuses an APK with another signature; the only way out is to uninstall, which deletes the pairing and the queue). Keep the backup private: whoever has the key can publish updates that install over Kin.
- **Check a build:** `android\tools\check-release.ps1` (signature and certificate SHA-256, size, and a scan of the manifest and dex for Google telemetry parts: `gms`, `firebase`, `datatransport`, `mlkit`). `apksigner verify --print-certs app-release.apk` gives the certificate.
- **Smoke test the signed APK** on an emulator against the real Windows app: `android\tools\release-smoke.ps1 [-Avd kin_api29]` (camera permission flow of "Scan code", pairing, ping, text, a file and a 3-file batch both ways, no crash in logcat; it builds the smoke APK first, `-NoBuild` skips that).
- **Links (2B.1) on the emulator:** `android	oolserify-2b1.ps1 [-Avd kin_api29 -Only Api29]` runs the link checks once against a test Kin (`openLinksAutomatically` off in its throwaway settings, so no browser opens on the PC) and `link-target/`, a throwaway app that owns `https://kin-link-test.example/...` (build: `gradlew.bat -p link-target assembleDebug`). Results: [phase-2b-1-android.md](../docs/results/phase-2b-1-android.md).
- **Notification mirroring (2B.2) on the emulator:** `android\tools\verify-2b2.ps1 [-Avd kin_api29 -Only Api29] [-ScreenshotDir ...]` runs the phone side once against a test PC stub (`FakePcSession`, a unit-test class run on demand: it lists `notif`, keeps every frame the phone sends and sends `notif.dismiss` / `notif.action` on command; see its header). The desktop round trip is the integration agent's `tools\e2e-2b-2.ps1`. Results: [phase-2b-2-android.md](../docs/results/phase-2b-2-android.md).
- **Media controls (2B.3) on the emulator:** `android\tools\verify-2b3.ps1 [-Avd kin_api29 -Only Api29] [-ScreenshotDir ...]` runs the phone side once against the same test PC stub (it now also lists `media` and has the `m*` commands: `mstatus`, `mstate`, `mclear`, `mvolume`, `mcmd`, `mcmdsid`, `mraw`; see its header). **Silent:** the phone's own media is `DebugMediaSession` (a `MediaSession` of Kin's with no `MediaPlayer`, no `AudioTrack` and no audio focus), the PC is a stub. Results: [phase-2b-3-android.md](../docs/results/phase-2b-3-android.md).
- **ABIs:** the release APK has the ARM libraries only (`arm64-v8a`, `armeabi-v7a`): the debug build keeps x86_64 for the emulator, so **the release APK does not install on an x86 emulator**. For smoke tests there is an opt-in **`gradlew assembleSmoke -PkinSmokeAbis=x86_64`**: the release build type (same R8, shrinking, proguard rules and signing) plus the given ABIs, written to `app/build/outputs/apk/smoke/app-smoke.apk`, never to `app-release.apk`; without the property the `smoke` build type doesn't exist and `assembleRelease` is unchanged. The smoke APK has its own application ID, **`app.kinrelay.android.smoke`** (`applicationIdSuffix`), so it can never install over the real app. `release-smoke.ps1` builds and installs that APK itself (also: a 3-file batch both ways; it needs `share-source`, built on demand).
- `versionCode` / `versionName` are in `app/build.gradle.kts` (2 / 0.1.1). The app reports `BuildConfig.VERSION_NAME` as `appVersion` in `hello` and in the pairing request. Raise `versionCode` for every build that anybody installs, or an update can't replace the older one.
- **QR scanner (D16):** `scan/ScanActivity` (CameraX preview and frames) and `scan/QrDecoder` (ZXing core, plain Kotlin, unit-tested on generated codes). The camera permission is asked for only when the user taps "Scan code".
## Emulator tests with a real desktop next to the user's Kin (v0.1.1)

The desktop's `Kin.exe` runs as a test Kin **beside** the user's installed Kin when it has a `--data-dir` of its own (own single-instance lock, port, toast identity; no mDNS; [desktop/README.md](../desktop/README.md#a-test-kin-beside-the-installed-one), D17), so these scripts need no private build. Build the desktop once with `desktop\build.ps1` (the scripts refuse a `Kin.exe` that is not from `tauri build`), then:

- `tools\e2e-v011.ps1 [-Avd kin_api29] [-Only S1,S2]` (it runs `android\tools\e2e-v011.ps1`): the v0.1.1 scenarios on the emulator against that desktop: 50 files each way, same names, cancel with several files in flight, cancel then Retry, the queue with a batch, Wi-Fi off in the middle. `-Only S8` is the stall hunt (Wi-Fi cuts with the phone and PC logs kept), `-Only S9` the 50 x 3.4 MB timings.
- `android\tools\bench-batch.ps1 -BeforeApk <0.1.0 debug APK>`: the same batches timed for the committed (before) and the new (after) debug APK. The emulator's own network path caps at about 2 to 9 MB/s, so only many-small-file batches tell the builds apart.


## Code layout (`app/src/main/java/app/kinrelay/android/`)

| Package | What |
|---|---|
| `identity` | P-256 key, hand-rolled DER certificate (`CertificateBuilder`), device and short IDs, key storage (AES-GCM under an Android Keystore key). `IdentityStore` replaces an identity only when it is clearly lost; a temporary Keystore error changes nothing |
| `tls` | Pinning trust manager, client key manager, the TLS 1.3 OkHttp client, the address race |
| `protocol` | Strict JSON, message receiver (validation order, replies, de-dup, and since 1.3 `offer`, `complete`, `cancel`), close-code rules, `UrlRules` (`isUrl`, which links may be opened), `FileNames` (cleaning and create-new), `TextRules` (128 KiB threshold) |
| `pairing` | Pairing URL parser, `POST /v1/pair`, the pairing flow, `PairingIntake` (a link from outside the app must be confirmed) |
| `link` | 1.4: `TextOutbox` (the text queue), `OutboxFlusher` (sends texts and uploads in creation order after the hello). `LinkService`, `LinkRuntime` (network callback, `WifiTracker`), `LinkLoop` (connection order, backoff), `LinkSession` (one WebSocket), mDNS, notifications, `BootReceiver`. 1.2: `IncomingHandler` (receive `clipboard` and `text`), `OutgoingSender` (send `text`, wait for the ack), `DeliveryQueue` (deliver in arrival order) |
| `clipboard` | `ClipboardSendActivity`, `SendClipboardTile`, `ClipboardGuard` (loop prevention), `ClipboardPort` |
| `share` | `ShareActivity` (share sheet: text and files), `ProcessTextActivity` (text menu, "Send to PC"), `ShareIntents` (reads the URIs), `ShareShortcuts` (the direct-share shortcut) |
| `transfer` | 1.3: `TransferEngine` (jobs, cancel, retry, recovery, the `offer` gate, unpair clean-up, the start-up sweep, crash clean-up), `ProgressThrottle` (one progress update per 500 ms), `UploadRunner`, `DownloadRunner`, `TransferHttp` (the four file endpoints), `MediaStoreSink`, `AndroidSourceFiles`, `TransferNotifications`, `AndroidWakeLocks`. The runners only know the ports in `TransferPorts.kt`, so the unit tests run them with fakes |
| `data` | Room (`devices`, `seen_messages`, 1.3 `transfers` and `transfer_files`, 1.4 `texts`): **database version 4**, `exportSchema` on. The schema of every version is in `app/schemas/` (checked in, not ignored), and `MigrationTest` runs the 2 → 3 and 3 → 4 migrations on a real SQLite; `RetentionSqlTest` runs the storage queries (`QuerySql`) on one too. DataStore holds the settings and counters (device name, latency history, setup done). `QueuePolicy`: 7-day expiry, 500 history items 
| `log` | Rotating file log with secret redaction. Text is never logged, only its length and a short keyed hash (HMAC-SHA256, per-install key wrapped by the Keystore: `LogFingerprint`) |
| `scan` | 1.5: `ScanActivity` (the camera screen), `ScanContract`, `QrDecoder` (ZXing) |
| `notifmirror` | 2B.2: `MirrorListenerService` (the listener), `MirrorController` (the hooks of the link and the service), `MirrorEngine` (skip rules, encoding, remove rules), `MirrorFilter`, `NotifEncoder` (limits, MessagingStyle, `pickActions`), `OtpDetector` + `OtpKeywords`, `RedactionMatcher` + `RedactedStrings`, `MirrorSender` (the only writer of `notif.*`), `ActionHandler` (`notif.action`, `notif.dismiss`), `AppCatalog`, `RestrictedSettings`, `MirrorPolicyStore` (DataStore), `AndroidIcons`, `SbnMapper`. `protocol/NotifInbound.kt` stands for the PC in the shared vectors |
| `media` | 2B.3: `MediaHub` (the hooks of the link, the user's choices, the PC's messages), `MediaSource` (reads the phone's sessions through the notification access: `getActiveSessions` and callbacks, no polling), `MediaSender` (the only writer of `media.*`: peer-status gate, 250 ms throttle, image before state, 1.5 s clear debounce, command limits), `MediaPick`, `MediaPosition`, `MediaMapping`, `MediaVolumeMath`, `MediaGate`, `MediaLimits`, `MediaInbound` (validation of `media.*`), `MediaCommandRunner`, `ArtworkEncoder` (`ArtLadder`), `AndroidVolume`, `PcMediaModel` + `PcMediaSession` + `PcMediaNotification` + `PcMediaReceiver` (the PC's media on the phone: one `MediaSession` and its `MediaStyle` notification), `MediaPolicyStore` (four booleans) |
| `presence`, `pause`, `ring`, `shots`, `features` | 2B.4: `PresenceWire` (`presence`, `ring.status`), `PresenceSender` (the only writer of `presence`: snapshots, 2-point / 30 s / 1 s rules, no periodic timer), `PresenceHub` (battery receiver only while a session is ready and sharing is on, the PC's pause and laptop battery), `LowBatteryNudge`, `BatteryMap`. `Pause` (`PauseGate`: the `Pause.kt` rules and the 08:00 / 8-day maths, `PauseController`, `PauseStore`, `PauseTile`, `PauseSettingsActivity`, `PauseReceiver`). `RingMachine` (pure state machine), `RingController` (the real player, alarm volume, vibration, wake lock), `RingNotification`, `RingActivity`. `ShotPicker` (the rules), `ShotWatcherCore` (baseline, 5 s hold, Cancel), `ShotController` (the `ContentObserver`, MediaStore ports), `ShotsStore`. `Features` (the per-feature resolver) |
| `ui` | Compose screens: confirm pairing, pairing, Home, History, Settings, the setup (onboarding), Debug |
| `history`, `setup`, `notify`, `debuginfo` | 1.4: `HistoryBuilder`, `SuccessRate`; `OemTable` (the one table of OEM settings screens) and `SetupActions`; `NotificationLedger`, `TextArrivals`, `TrailingThrottle`, channels; `ConnectionHistory`, `StateDump`, `LogExport` |

## Automation hooks (for the emulator and the integration agent)

All of these assume the emulator is running (`tools\emulator\start.ps1`) and the debug build is installed (`tools\emulator\install.ps1`). Use `adb -s emulator-5554`.

### Permissions (Android 17 / API 37)

Apps targeting API 37 need the runtime permission `ACCESS_LOCAL_NETWORK` ("Nearby devices") to reach any address on the local network, **including `10.0.2.2`**. Without it a connection just times out. The app asks for it before pairing, and shows "Nearby devices permission needed" on Home and in the service notification while it is missing or revoked. For scripts, grant it up front:

```
adb shell pm grant app.kinrelay.android android.permission.ACCESS_LOCAL_NETWORK
adb shell pm grant app.kinrelay.android android.permission.POST_NOTIFICATIONS
```

(On API 29 neither permission exists; `pm grant` of them is not needed.)

### Pairing: the deep link

A pairing link that arrives by intent from outside the app (a browser, a message, `adb`) **never pairs by itself**. It opens a confirmation screen with the PC name, the PC's short ID worked out from the link, and "This will replace <current PC>" if another PC is paired. Pairing starts only after Confirm. Text the user scanned or pasted inside the app skips the screen, because that tap is the confirmation (the scan and paste fields exist only on the first-run screen, so they can never replace a PC).

For scripts, the **debug build only** has an auto-confirm extra. The release build has no such code (`classes.dex` holds no `AUTO_CONFIRM`):

```
adb shell "am start -a android.intent.action.VIEW -d 'kinrelay://pair?v=1&a=10.0.2.2:47821&id=<pc-device-id>&tk=<token>&n=Desk%20PC' --ez app.kinrelay.android.AUTO_CONFIRM true"
```

Quote the URL (`&` is special in the device shell). Without the extra, the same command shows the confirmation screen (tap Confirm, found with `uiautomator dump`, or use the `PAIR` broadcast below). The `https://<any host>/pair#...` form works the same way. With the extra, pairing and starting the service happen without further taps (once the permissions above are granted).

### Debug-only commands (a broadcast receiver)

`app/src/debug/` holds a receiver that exists **only in debug builds** (it is in `src/debug`, so release builds don't contain it). It requires the `DUMP` permission, which the adb shell has and no normal app has.

```
adb shell am broadcast -a app.kinrelay.android.debug.<COMMAND> -n app.kinrelay.android/.debug.DebugCommandReceiver [extras]
```

| Command | What it does | Result line (logcat tag `KinRelay`, also in the log file) |
|---|---|---|
| `PING` | Starts the service if needed and sends a `ping` to the PC | `CMD PING result=Rtt(millis=6.6)` / `NoReply` / `NoConnection` / `Failed(...)` |
| `STATE` | Dumps the state | `CMD STATE link=Connected(...) paired=<name> pcId=<id> lastAddresses=... lastPing=... lastArrivalMs=<tap to ack of the last send, ms> clipboardGuard=<set + age, or none> identityProblem=<null or text> pairing=<state> connects=N disconnects=N kills=N phoneId=<id>` |
| `CONNECT` | A user action: starts the service, resets the backoff, reconnects now | `CMD CONNECT done`, then the usual `link:` lines |
| `UNPAIR` | The Unpair button (without the confirmation dialog) | `CMD UNPAIR done` |
| `PAIR --es url "<link>"` | Pairs from a link as if it had been pasted in the app (no confirmation screen). Prefer the deep link with the auto-confirm extra: starting the service from a background broadcast may be refused on Android 12+ | the `pairing:` lines, `CMD PAIR PairNow` |
| `SEND_TEXT --es text "<text>"` | Sends the text to the PC with the same sender as the share sheet: **saved in the queue first**, then flushed, and waits for the ack | `CMD SEND_TEXT result=Delivered(arrivalMillis=8)` / `Queued(reason=..., afterNoReply=false)` (no connection, or no ack: it goes out by itself later) / `Refused(code=...)` / `NotPaired` / `NotSent(reason=...)` (a `--ez sensitive true` text with no open connection or no ack: **never queued, never stored**, a "Not sent: <PC> offline" notification) |
| `LINK_FROM_PC --es url <url> [--es title <t>] [--es id <id>]` | A `link` as if the PC had sent it (2B.1): the real `IncomingHandler` and notifications, so the title, the real address, the tap, Copy link, the group and the lock-screen rule are the real ones | `CMD LINK_FROM_PC shown=true` (`false`: not an http or https address, nothing shown) |
| `TEXT_FROM_PC --es text <text> [--ez sensitive true]` | A `text` as if the PC had sent it (the notification with its Copy button; the clipboard follows the setting) | `CMD TEXT_FROM_PC done` |
| `SEND_LINK --es url <url> [--es title <t>]` | The share path of a web link: queued first as a `link`, goes out as a `link` or, to a PC without the capability, as a `text` | `CMD SEND_LINK result=Delivered(...)` / `Queued(...)` / `NotAWebLink` |
| `SET_LINK_PREFS [--ez lockscreen true\|false] [--ez copytext true\|false]` | "Show link previews on the lock screen" and "Copy received text automatically" | `CMD SET_LINK_PREFS lockscreen=... copytext=...` |
| `MIRROR_ALLOW --es pkg <p>[,<p>] [--ez on false]` | 2B.2: allow (or, with `on false`, stop allowing) apps in the notification mirror. `com.android.shell` is the package of `adb shell cmd notification post`. Grant access first: `adb shell cmd notification allow_listener app.kinrelay.android/app.kinrelay.android.notifmirror.MirrorListenerService` | `CMD MIRROR_ALLOW allowed=[...]` |
| `MIRROR_HIDDEN --es pkg <p>[,<p>] [--ez on false]` | The apps that may show notifications hidden on the lock screen | `CMD MIRROR_HIDDEN hiddenAllowed=[...]` |
| `MIRROR_ENABLE [--ez on false]` | The master switch | `CMD MIRROR_ENABLE enabled=true` |
| `MIRROR_DISMISS_PREF [--ez on false]` | "Dismiss on the phone too" | `CMD MIRROR_DISMISS_PREF dismissOnPhone=...` |
| `MIRROR_POST --es kind a..g [--es tag <t>]` | The debug test notifications from Kin itself on the channel `kin_debug_mirror` (a: conversation + reply + Mark as read + Open, b: code, c: ongoing, d: group + summary, e: hidden on the lock screen, f: low importance, g: redacted look-alike). Tag `t`, ids 1 to 9 | `CMD MIRROR_POST kind=a posted=true` |
| `MIRROR_CANCEL --es tag <t> --ei id <n>` | Cancels one of those, as an app removing its notification | `CMD MIRROR_CANCEL done` |
| `MIRROR_CLEAR_ALL` | Clears the shade through the listener (`adb shell cmd notification post` is limited to 50 notifications per package, and the shell has no "cancel all") | `CMD MIRROR_CLEAR_ALL done` |
| `MIRROR_STATE` | Counts and settings, **no content**: `enabled apps hiddenAllowed dismissOnPhone access listener pcAcceptsNotif online sent removed identical actions dismissed dropped={...} announced lastAction replies lastReplySha marksRead sdk` | `CMD MIRROR_STATE ...` |
| `MEDIA_SET [--ez share b] [--ez show b] [--ez lockscreen b] [--ez volkeys b]` | 2B.3: the four media switches (all off by default): share this phone's media, show the PC's, PC media on the lock screen, the phone's volume keys for the PC | `CMD MEDIA_SET share=.. show=.. lockscreen=.. volkeys=..` |
| `MEDIA_TEST_START [--es title t] [--es artist a] [--ei durationMs n] [--ez art false]` | A **silent** test session of Kin's own (`DebugMediaSession`: no audio at all, art is a generated 640 x 640 gradient). A debug build reads only this session, unless `MEDIA_READ_ALL` | `CMD MEDIA_TEST_START running=true` |
| `MEDIA_TEST_STATE --es state playing\|paused\|buffering\|stopped [--ei positionMs n] [--ei rateMilli n] [--es actions play,pause,toggle,next,prev,seek]` | Sets its `PlaybackState` | `CMD MEDIA_TEST_STATE done` |
| `MEDIA_TEST_TRACK [--es title t] [--es artist a] [--ei durationMs n]` / `MEDIA_TEST_STOP` / `MEDIA_TEST_BURST --ei count 200` | A track change / ends the session / count state changes in about 3 s (the sender's rate rules) | `CMD MEDIA_TEST_...` |
| `MEDIA_STATE` | Counts and settings, **no content**: `share show access listener online open peerShow peerShare sentStates images volumes clears statuses cmdsSent shown dropped={...} cmdsIn={...} cmdErrors={...} pcSession pcState readAll testSession cmdsReceived={...} lastCmd musicVolume musicMuted sdk` | `CMD MEDIA_STATE ...` |
| `MEDIA_PC_CMD --es cmd play\|pause\|next\|prev\|seek\|volume [--ei positionMs n] [--ei level n] [--ei delta n]` | Calls the callback of Kin's own PC-media session through its own controller: exactly what a carousel button does (no input injection) | `CMD MEDIA_PC_CMD result=sent` |
| `MEDIA_PC_FAKE --es state s [--es title t] [--es artist a] [--ei durationMs n] [--ei positionMs n] [--es actions a,b] [--ez art false] [--ez volkeys true]` / `MEDIA_PC_FAKE_CLEAR` | Shows (or removes) a made-up PC session without the wire (spike S3 and screenshots) | `CMD MEDIA_PC_FAKE shown=true` |
| `MEDIA_VOLUME --ei level n` | The phone's `STREAM_MUSIC` (the emulator is not the user's device) | `CMD MEDIA_VOLUME result=ok index=a/b` |
| `MEDIA_READ_ALL [--ez on false]` | The reader takes every session, not only the debug one (emulator only) | `CMD MEDIA_READ_ALL on=true` |
| `MEDIA_PROBE` | Spike S3: what the platform lets Kin do (`getActiveSessions` with the listener component, `transportControls`, `STREAM_MUSIC` to 1 and 0, mute) | `CMD MEDIA_PROBE ...` |
| `PAUSE_SET --es mode off\|1h\|tomorrow\|untilresumed\|resume` or `--el untilMs N` | 2B.4: this phone's Pause. `untilMs` sets an exact end (epoch ms: a past one tests the expiry, one more than 8 days ahead tests the guard) | `CMD PAUSE_SET paused=.. untilResumed=.. untilMs=.. quiet=N peerPaused=..` |
| `PAUSE_STATE` | The same line, no change | `CMD PAUSE_STATE ...` |
| `PRESENCE_STATE` | Counts and the PC's laptop battery and pause (a debug build only: the `CMD` echo lines are dropped by the log export) | `CMD PRESENCE_STATE connected=.. listsPresence=.. presence: sent N, received N, dropped {..} pcBattery=.. peerPaused=.. share=.. show=.. receiver=..` |
| `PRESENCE_SET [--ez share b] [--ez show b]` | "Share this phone's battery" and "Show the PC's status" (both on by default) | `CMD PRESENCE_SET ...` |
| `RING_SET [--ez allow b] [--ez loud b] [--ez silent b]` | "Let my PC ring this phone" (on by default; a change goes to the PC as a new `hello`), "Ring at maximum volume", and **`silent`: the ring starts no `MediaPlayer` and no `Vibrator` and never touches the `STREAM_ALARM` volume** (the file `files/debug-ring-silent`; use it on any emulator that is not headless with `-no-audio`) | `CMD RING_SET state=.. audible=.. ring: started N, ... savedAlarmIndex=.. canFullScreen=.. silent=..` |
| `RING_STATE` / `RING_STOP_LOCAL` | The ring's state (idle or ringing, why it last stopped, the saved alarm index, whether a full-screen intent is allowed); the Stop button | `CMD RING_STATE ...` |
| `SHOTS_SET [--ez screenshots b] [--ez camera b]` | Auto-send (both off by default). Turning one **on** takes the baseline first and is refused (`refused=true`) when the permission isn't granted (use `adb shell pm grant app.kinrelay.android android.permission.READ_MEDIA_IMAGES`; `READ_EXTERNAL_STORAGE` on 10 to 12) or the PC's `hello` lacks `presence` | `CMD SHOTS_SET refused=.. screenshots=.. camera=.. permission=.. watching=.. baseline=.. holding=.. shots: seen N, sent N, ...` |
| `SHOTS_INSERT --es kind screenshot\|camera\|other\|own [--ei ageSec N]` | Inserts a **generated** PNG into the emulator's MediaStore (`Pictures/Screenshots/`, `DCIM/Camera/`, `Pictures/Other/`, `Pictures/Kin/`); `ageSec` moves the picker's clock forward for 3 s (MediaProvider sets `DATE_ADDED` itself) so the 15-second rule can be seen | `CMD SHOTS_INSERT kind=.. ageSec=.. id=..` |
| `SHOTS_STATE` / `SHOTS_CANCEL --el id N` | Baseline, counts and skip reasons (**no names or paths**); the Cancel button of the 5-second hold | `CMD SHOTS_STATE ...` |
| `DB_INFO` | Checkpoints the database and prints `PRAGMA journal_mode` and `PRAGMA synchronous` (design 5.7, R38) | `CMD DB_INFO journal=wal synchronous=N` |

The emulator's battery is set from the shell, not by Kin: `adb shell dumpsys battery set level 14`, `set ac 0` (or `set usb 0`), `unplug`, and `adb shell dumpsys battery reset` to end the test (it changes the **emulator's** battery only). `adb shell cmd notification set_dnd none` (total silence) / `set_dnd off` and `adb shell appops set app.kinrelay.android USE_FULL_SCREEN_INTENT allow|deny` are the emulator's own too.
| `SET_CLIPBOARD --es text "<text>"` | Puts text on the phone's clipboard (to prepare a "send clipboard" test) | `CMD SET_CLIPBOARD len=N sha=xxxxxxxx` |
| `EXPORT_LOGS` | The same zip as Settings → Debug → Export logs, without the share sheet (read it with `run-as ... cat cache/exports/<file>`) | `CMD EXPORT_LOGS uri=... file=kin-logs-<time>.zip bytes=N` |
| `SET_PING_INTERVAL --ei seconds N` | WebSocket ping interval, for the soak (default 240; applies at the next connection) | `CMD SET_PING_INTERVAL Ns ...` |

Wait for a result with logcat, for example:

```
adb logcat -c
adb shell am broadcast -a app.kinrelay.android.debug.PING -n app.kinrelay.android/.debug.DebugCommandReceiver
adb logcat -d -s KinRelay | findstr "CMD PING"
```

### Text and the clipboard (milestone 1.2)

Quote anything with spaces for the device shell (the whole command as one string, with single quotes inside).

```
# phone -> PC, text
adb shell am broadcast -a app.kinrelay.android.debug.SEND_TEXT -n app.kinrelay.android/.debug.DebugCommandReceiver --es text hello
adb shell "am start -n app.kinrelay.android/.share.ShareActivity -a android.intent.action.SEND -t text/plain --es android.intent.extra.TEXT 'shared text'"
adb shell "am start -n app.kinrelay.android/.share.ProcessTextActivity -a android.intent.action.PROCESS_TEXT -t text/plain --es android.intent.extra.PROCESS_TEXT 'selected words'"

# phone -> PC, the clipboard (the real path: a focused activity reads it)
adb shell "am broadcast -a app.kinrelay.android.debug.SET_CLIPBOARD -n app.kinrelay.android/.debug.DebugCommandReceiver --es text 'from the phone'"
adb shell am start -n app.kinrelay.android/.debug.SendClipboardAlias

# read the phone's real clipboard (what a PC -> phone `clipboard` message set)
adb shell am start -n app.kinrelay.android/.debug.DebugClipboardActivity
adb logcat -d -s KinRelay | findstr "CMD CLIPBOARD"      # CMD CLIPBOARD len=21 sha=e5809efc text="..."

# the quick settings tile
adb shell cmd statusbar add-tile app.kinrelay.android/app.kinrelay.android.clipboard.SendClipboardTile
adb shell cmd statusbar expand-settings
```

- `SendClipboardAlias` and `DebugClipboardActivity` are debug-only (`src/debug`) and protected by the `DUMP` permission. The adb shell can't start the real `ClipboardSendActivity` (it isn't exported), so the alias stands in for the tile and the notification button, which start that same activity.
- `DebugClipboardActivity` prints to logcat only, never to the log file, because the log file holds no text, only lengths and short hashes. `STATE` has `clipboardGuard=set ageMs=…` (no hash).
- A plain `am start -a android.intent.action.SEND -t text/plain ...` (without `-n`) opens the system chooser, because other apps take text too. Tap Kin and "Just once", or name the component as above.
- The arrival time (targets.md priority 2) is **tap to ack**: from the moment the send activity was created (or the debug command ran) to the PC's `ack`. It is in `CMD SEND_TEXT ... arrivalMillis`, in `STATE` (`lastArrivalMs`), in the log (`text sent (...) ...; tap to ack N ms`) and on the Debug tab.
- Loop prevention: text the PC just put on the clipboard (`clipboard`, or the copy that comes with `text`) is remembered by its SHA-256 for 60 s. A "send clipboard" of the same text in that time is not sent ("The PC already has this text."; the log says `send skipped (CLIPBOARD)`). Share and "Send to PC" are explicit and never skipped.

### Files (milestone 1.3)

All debug-only (`src/debug`), protected by the `DUMP` permission. Transfer IDs must be new each time: an `offer` whose ID was seen in the last 24 hours is acked and ignored (de-duplication), and since the 1.3 review an `offer` whose ID belongs to anything else the phone has (an upload, another PC's transfer, a finished one) is refused with `error bad_request`. A transfer ID is never reused.

```
# make files to share (in files/debug-share/): name:size = deterministic random bytes, name@WxH = a real JPEG/PNG
adb shell am broadcast -a app.kinrelay.android.debug.MAKE_FILES -n app.kinrelay.android/.debug.DebugCommandReceiver --es spec "photo.jpg@1200x800,big.bin:2000000,empty.txt:0"
#   result: CMD MAKE_FILES <name> <size> <first 16 hex of the SHA-256>

# share them like another app's Share would (starts the real ShareActivity, with FLAG_GRANT_READ_URI_PERMISSION and content:// URIs)
adb shell "am start -n app.kinrelay.android/.debug.DebugShareActivity --es files 'photo.jpg,big.bin,empty.txt'"
#   one file = ACTION_SEND, more = ACTION_SEND_MULTIPLE (--es action SEND|SEND_MULTIPLE to force one)

# or skip the activity and give the files to the transfer engine
adb shell am broadcast -a app.kinrelay.android.debug.SEND_FILES -n app.kinrelay.android/.debug.DebugCommandReceiver --es files "photo.jpg,big.bin"

# transfer state: CMD TRANSFER <id> <up|down> <queued|active|done|failed|canceled> <done>/<total> files=N names=... err=<code>
adb shell am broadcast -a app.kinrelay.android.debug.TRANSFERS -n app.kinrelay.android/.debug.DebugCommandReceiver
adb shell am broadcast -a app.kinrelay.android.debug.CANCEL_TRANSFER -n app.kinrelay.android/.debug.DebugCommandReceiver --es id <transfer id>
adb shell am broadcast -a app.kinrelay.android.debug.RETRY_TRANSFER -n app.kinrelay.android/.debug.DebugCommandReceiver --es id <transfer id>
adb shell am broadcast -a app.kinrelay.android.debug.RETRY_QUEUED -n app.kinrelay.android/.debug.DebugCommandReceiver

# what was received, with the path MediaStore reports and whether it is still pending
adb shell am broadcast -a app.kinrelay.android.debug.RECEIVED -n app.kinrelay.android/.debug.DebugCommandReceiver
#   CMD RECEIVED <images|video|downloads> id=.. name='..' size=.. mime=.. path=Pictures/Kin/ pending=0|1 uri=content://media/...
#   the files themselves: adb shell sha256sum /sdcard/Pictures/Kin/<name> (also /sdcard/Movies/Kin, /sdcard/Download/Kin)
adb shell am broadcast -a app.kinrelay.android.debug.DELETE_RECEIVED -n app.kinrelay.android/.debug.DebugCommandReceiver   # a clean slate

# R7: does appending to a pending MediaStore entry work here? (wa, w, and the way the app does it)
adb shell am broadcast -a app.kinrelay.android.debug.APPEND_CHECK -n app.kinrelay.android/.debug.DebugCommandReceiver
adb shell am broadcast -a app.kinrelay.android.debug.EMPTY_ENTRY_CHECK -n app.kinrelay.android/.debug.DebugCommandReceiver

# the 1.3 review fixes: leave what a crash would leave (a pending Download/Kin entry no transfer owns, an hour-old cache folder), then sweep
adb shell am broadcast -a app.kinrelay.android.debug.MAKE_ORPHANS -n app.kinrelay.android/.debug.DebugCommandReceiver
adb shell am broadcast -a app.kinrelay.android.debug.SWEEP -n app.kinrelay.android/.debug.DebugCommandReceiver   # what every app start does; "start-up sweep: deleted N ..." in the log
adb shell run-as app.kinrelay.android touch files/debug-refuse-names    # every new file name is "taken": a download fails after the 100 suffixes (internal, cancel reason error)
adb shell run-as app.kinrelay.android rm files/debug-refuse-names
```

**Added by the integration run (`tools/e2e-1-3.ps1` uses them).** In a file name for `MAKE_FILES`, `DebugShareActivity` and `SEND_FILES`, `%uXXXX;` stands for one code point (`rtl%u202E;gpj.exe`, `%u4E2D;`): adb can't carry bidi controls or CJK through a Windows console, and a trailing space or dot needs it too (`tail.txt.%u0020;`). `SEND_TEXT` and `SET_CLIPBOARD` take `--es textfile <name>` instead of `--es text`: the text is the UTF-8 content of `files/debug-share/<name>` (an argument can't hold 200 KiB or a bare CR). Big files are not made by `MAKE_FILES` (it keeps the whole file in memory): make them on the phone, `adb shell "run-as app.kinrelay.android sh -c 'head -c 314572800 /dev/urandom > files/debug-share/big.bin'"`, and hash them with `sha256sum` the same way. The `CMD CLIPBOARD` line now has `sensitive=<bool>` between the hash and the text.

`STATE` also shows `wakeLockHeld`. `SEND_TEXT` and `SET_CLIPBOARD` take `--ez sensitive true` (the text is sent flagged / the clip is marked sensitive). For the R5 control experiment, `adb shell run-as app.kinrelay.android touch files/debug-no-grant` makes a share skip the read grant to the service (debug build only).

Useful: `adb shell dumpsys shortcut` (the direct-share shortcut `pc`), `adb shell svc wifi disable` / `enable` (a transfer waits for the PC and resumes), `adb shell dumpsys notification --noredact`.

**A second app to test the share path for real:** `share-source/` is a tiny throwaway app (not part of Kin, not in the release) with a provider that is **not exported**, so Kin can read its files only through the grant on the share intent, exactly like a gallery. Build it with `cd share-source && ..\gradlew.bat assembleDebug` and install it; then:

```
adb shell am start -n app.kinrelay.sharesource/.ShareActivity --ei n 3 --ei size 3000000          # share 3 files straight to Kin
adb shell am start -n app.kinrelay.sharesource/.ShareActivity --ei n 2 --ei size 40000000 --ez kill true   # the source app dies 0.5 s after the share
adb shell am start -n app.kinrelay.sharesource/.ShareActivity --ei n 2 --ei size 5000 --ez jpeg true --ez chooser true   # open the system share sheet: the PC is in the top row
```

### Queue, history and settings (milestone 1.4)

All debug-only, protected by the `DUMP` permission. Nothing here prints a text: only IDs, states and counts.

```
adb shell am broadcast -a app.kinrelay.android.debug.TEXTS  -n app.kinrelay.android/.debug.DebugCommandReceiver
#   CMD TEXT <id> <up|down> <queued|active|done|failed|canceled> attempts=N src=<share|process_text|clipboard|debug|pc> sensitive=<bool> bytes=N err=<code> bodyKept=<bool>
adb shell am broadcast -a app.kinrelay.android.debug.CANCEL_TEXT -n app.kinrelay.android/.debug.DebugCommandReceiver --es id <message id>   # only a queued text is canceled
adb shell am broadcast -a app.kinrelay.android.debug.RETRY_TEXT  -n app.kinrelay.android/.debug.DebugCommandReceiver --es id <message id>   # a failed text goes out again as a new message
adb shell am broadcast -a app.kinrelay.android.debug.FLUSH       -n app.kinrelay.android/.debug.DebugCommandReceiver                         # flush the queue now (what the hello does)
adb shell am broadcast -a app.kinrelay.android.debug.EXPIRE      -n app.kinrelay.android/.debug.DebugCommandReceiver                         # fail what waited 7 days
#   --el ahead <ms>: judge the queue as if the clock were that far ahead (604800000 = a text made now is 7 days old; 603000000 expires nothing). CMD EXPIRE expired=N
```

- `STATE` now also has `successRate=<n% (x of y)>`.
- The setup screen picks its OEM guide from `Build.MANUFACTURER`. To see another maker's steps on the emulator: `adb shell "run-as app.kinrelay.android sh -c 'echo xiaomi > files/debug-oem'"` (debug build only; `rm` the file to undo).
- **Queue test on the emulator:** `adb shell svc wifi disable`, `SEND_TEXT` (result `Queued`), `am force-stop` the app to prove the queue survives a kill, `svc wifi enable`, open the app: the text arrives right after the hello. The fake PC's `holdtext <text>` is the PC's own queue: the text goes out right after the phone's next `hello`.
- **Notification cap:** `adb shell dumpsys notification --noredact` lists the notifications of the app; after 60 texts there is one "N texts from <PC>" (id 3), and after 60 offers at most 10 results (id 11) and one summary (id 21).
- **Export logs** (Debug screen): the zip is in `cache/exports/`; `adb shell "run-as app.kinrelay.android sh -c 'unzip -l cache/exports/*.zip; unzip -p cache/exports/*.zip state.txt'"`.

### Reading the state

- **Logcat:** `adb logcat -d -s KinRelay`. Every line has the tag `KinRelay` and looks like `<ISO time> <I|W|E> <component>: <message>`. Components: `app`, `identity`, `pairing`, `tls`, `link`, `service`, `boot`, `mdns`, `clipboard`, `share`, `debug`. The lines that matter:
  - `link: connected to '<PC>' via <host:port>`: hello accepted
  - `link: connection ended: ... (code N, upgrade N, hello accepted for N ms) -> <ACTION>`
  - `link: disconnected: ...`
  - `link: ping received from '<PC>'` (and the "Ping from <PC>" notification)
  - `link: clipboard from the PC len=N sha=...`, `link: text from '<PC>' len=N sha=...`, `link: text sent (SHARE|PROCESS_TEXT|CLIPBOARD|DEBUG) len=N sha=...; tap to ack N ms`, `link: send failed (...)`, `link: send skipped (CLIPBOARD)`. Text is never logged, only its length and 8 hex digits of a keyed hash (HMAC-SHA256 with a per-install key); a sensitive text is not logged at all, except that the echo skip (`send skipped (CLIPBOARD)`) is still logged, with no length and no hash
  - `service: service started` / `service stopped` / `kill detected: ...`
  - `boot: BOOT_COMPLETED received` / `service start requested`
  - `identity: ...`: `identity unavailable (attempt N)` means a temporary Keystore problem (nothing was deleted); `the stored identity was replaced: ...` means the old one was lost for good
- **Log file** (a rotating pair, 256 KiB each, secrets redacted): `adb shell run-as app.kinrelay.android cat files/logs/kin.log` (and `kin.log.1`). Works for debug builds.
- **Notifications:** `adb shell dumpsys notification --noredact | findstr "Connected Ping Text PC"`. A text from the PC shows as "Text from <PC>", with an "Open" action only for a real `http`/`https` link. A failed send shows "PC not reachable" or "No reply from <PC>".
- **Service:** `adb shell dumpsys activity services app.kinrelay.android` (look for `isForeground=true` and `types=0x00000010`).
- The Debug tab of the app shows the last log lines, the connect, disconnect and kill counts, the last ping and the last send time.

### Other useful adb commands

```
adb shell svc wifi disable        # "Not on Wi-Fi", clean close with 1000; a send then says "PC not reachable"
adb shell svc wifi enable         # reconnects
adb shell run-as app.kinrelay.android kill -9 <pid>    # a kill; the next service start logs "kill detected"
adb reboot                         # BootReceiver; look for the boot: lines
adb shell pm revoke app.kinrelay.android android.permission.ACCESS_LOCAL_NETWORK   # API 37: "Nearby devices permission needed"
```

## A fake PC for the emulator (test code, not part of the app)

`app/src/test/.../testing/FakePcSession.kt` runs an in-JVM stand-in for the Windows app (TLS 1.3 with a mandatory client certificate, `POST /v1/pair`, `/v1/ws`) on **loopback only** (`127.0.0.1:47831`, reached from the emulator as `10.0.2.2:47831`). It accepts any pairing and writes `app/build/fake-pc/pair-url.txt`. It is opt-in:

```
set KIN_FAKE_PC=1
gradlew.bat testDebugUnitTest --tests "*FakePcSession*" --rerun
```

It serves for `KIN_FAKE_PC_SECONDS` (default 300). Milestone 1.3 adds the file endpoints (`FakePcFiles`: `POST`, `PUT`, `DELETE`, `GET` with `Range`). Drive it by writing a word into `app/build/fake-pc/command.txt`: `ping`, `close4001`, `close4003`, `unpair`, `hello2` (a future-protocol hello) or `stop`. Milestone 1.2 adds `clipboard <text>`, `text <text>`, `url <link>` (a `text` with `isUrl`), `noack` / `ack` (stop / resume acking the phone's `text`) and `errtext` (answer it with an `error`). Milestone 1.3 adds `offer <id> <spec,...>` (`name:size[:mime]` or `name@WxH`), `cancel <id>`, `gone <id>`, `changed <id> <fileId>`, `postfail <status>`, `putfail <status>`, `dropput <bytes>`, `dropget <bytes>`, `throttle <bytes per 100 ms>`, `stale <offset>`, `used <transferId>` (that ID is used up: 404 to POST, PUT and DELETE), `holdtext <text>` (the PC keeps a text and sends it right after the phone's next hello), `ackmsg` / `noackmsg` (the list is in the header of `FakePcSession.kt`). It logs what it saw to `events.txt`, including what the PC stored (`stored <transfer> <file> ... sha=<hex>`) and the phone's `complete` / `cancel` messages. While it runs it holds `app/build` open, so stop it (`stop`) before the next Gradle build.

## The certificate fixture

The certificate produced by the app's own `CertificateBuilder` lives in **one place**: `protocol/fixtures/android-client.{der,json}` (the desktop verifier tests use it too). The Android tests read it from there (`../protocol/` is a test resource directory), so there is no second copy. To regenerate it (not needed normally): `set KIN_EXPORT_FIXTURE=1` and run `gradlew.bat testDebugUnitTest --tests "*CertificateBuilderTest*"`. The new files are written to `app/build/fixture-export/` and have to be copied over `protocol/fixtures/` by hand (that folder belongs to the protocol owner).
