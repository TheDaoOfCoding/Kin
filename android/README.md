# Kin Android app

Milestone 1.1 (connection spike): identity, pairing, a foreground service that holds one WebSocket to the PC, ping, local log. Milestone 1.2: text, links and the clipboard in both directions. The contract is [docs/protocol.md](../docs/protocol.md); the design is [docs/android.md](../docs/android.md). Details of what was built and measured: [phase-1-1-android.md](../docs/results/phase-1-1-android.md), [phase-1-2-android.md](../docs/results/phase-1-2-android.md).

## Build and test

Needs JDK 21 and the Android SDK (`JAVA_HOME`, `ANDROID_HOME`; see [phase-0-android.md](../docs/results/phase-0-android.md)). From this folder:

```
gradlew.bat lintDebug testDebugUnitTest assembleDebug assembleRelease
gradlew.bat --stop
```

- Debug APK: `app/build/outputs/apk/debug/app-debug.apk`. Release (R8): `app/build/outputs/apk/release/app-release-unsigned.apk`.
- The unit tests load the shared vectors from `../protocol/` as test resources (`app/build.gradle.kts`).

## Code layout (`app/src/main/java/app/kinrelay/android/`)

| Package | What |
|---|---|
| `identity` | P-256 key, hand-rolled DER certificate (`CertificateBuilder`), device and short IDs, key storage (AES-GCM under an Android Keystore key). `IdentityStore` replaces an identity only when it is clearly lost; a temporary Keystore error changes nothing |
| `tls` | Pinning trust manager, client key manager, the TLS 1.3 OkHttp client, the address race |
| `protocol` | Strict JSON, message receiver (validation order, replies, de-dup), close-code rules, `UrlRules` (`isUrl`, which links may be opened) |
| `pairing` | Pairing URL parser, `POST /v1/pair`, the pairing flow, `PairingIntake` (a link from outside the app must be confirmed) |
| `link` | `LinkService`, `LinkRuntime` (network callback, `WifiTracker`), `LinkLoop` (connection order, backoff), `LinkSession` (one WebSocket), mDNS, notifications, `BootReceiver`. 1.2: `IncomingHandler` (receive `clipboard` and `text`), `OutgoingSender` (send `text`, wait for the ack), `DeliveryQueue` (deliver in arrival order) |
| `clipboard` | `ClipboardSendActivity`, `SendClipboardTile`, `ClipboardGuard` (loop prevention), `ClipboardPort` |
| `share` | `ShareActivity` (share sheet), `ProcessTextActivity` (text menu, "Send to PC") |
| `data` | Room (`devices`, `seen_messages`) and DataStore settings and counters |
| `log` | Rotating file log with secret redaction. Text is never logged, only its length and a short hash |
| `ui` | Compose screens: confirm pairing, onboarding, home, debug |

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
| `STATE` | Dumps the state | `CMD STATE link=Connected(...) paired=<name> pcId=<id> lastAddresses=... lastPing=... lastArrivalMs=<tap to ack of the last send, ms> clipboardGuard=<sha, age> identityProblem=<null or text> pairing=<state> connects=N disconnects=N kills=N phoneId=<id>` |
| `CONNECT` | A user action: starts the service, resets the backoff, reconnects now | `CMD CONNECT done`, then the usual `link:` lines |
| `UNPAIR` | The Unpair button (without the confirmation dialog) | `CMD UNPAIR done` |
| `PAIR --es url "<link>"` | Pairs from a link as if it had been pasted in the app (no confirmation screen). Prefer the deep link with the auto-confirm extra: starting the service from a background broadcast may be refused on Android 12+ | the `pairing:` lines, `CMD PAIR PairNow` |
| `SEND_TEXT --es text "<text>"` | Sends the text to the PC with the same sender as the share sheet, and waits for the ack | `CMD SEND_TEXT result=Delivered(arrivalMillis=8)` / `NotReachable(reason=...)` / `NoReply` / `Refused(code=...)` |
| `SET_CLIPBOARD --es text "<text>"` | Puts text on the phone's clipboard (to prepare a "send clipboard" test) | `CMD SET_CLIPBOARD len=N sha=xxxxxxxx` |
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
- `DebugClipboardActivity` prints to logcat only, never to the log file, because the log file holds no text, only lengths and short hashes. `STATE` has `clipboardGuard=sha=… ageMs=…`: the hash of the text the PC last set.
- A plain `am start -a android.intent.action.SEND -t text/plain ...` (without `-n`) opens the system chooser, because other apps take text too. Tap Kin and "Just once", or name the component as above.
- The arrival time (targets.md priority 2) is **tap to ack**: from the moment the send activity was created (or the debug command ran) to the PC's `ack`. It is in `CMD SEND_TEXT ... arrivalMillis`, in `STATE` (`lastArrivalMs`), in the log (`text sent (...) ...; tap to ack N ms`) and on the Debug tab.
- Loop prevention: text the PC just put on the clipboard (`clipboard`, or the copy that comes with `text`) is remembered by its SHA-256 for 60 s. A "send clipboard" of the same text in that time is not sent ("The PC already has this text."; the log says `send skipped (CLIPBOARD)`). Share and "Send to PC" are explicit and never skipped.

### Reading the state

- **Logcat:** `adb logcat -d -s KinRelay`. Every line has the tag `KinRelay` and looks like `<ISO time> <I|W|E> <component>: <message>`. Components: `app`, `identity`, `pairing`, `tls`, `link`, `service`, `boot`, `mdns`, `clipboard`, `share`, `debug`. The lines that matter:
  - `link: connected to '<PC>' via <host:port>`: hello accepted
  - `link: connection ended: ... (code N, upgrade N, hello accepted for N ms) -> <ACTION>`
  - `link: disconnected: ...`
  - `link: ping received from '<PC>'` (and the "Ping from <PC>" notification)
  - `link: clipboard from the PC len=N sha=...`, `link: text from '<PC>' len=N sha=...`, `link: text sent (SHARE|PROCESS_TEXT|CLIPBOARD|DEBUG) len=N sha=...; tap to ack N ms`, `link: send failed (...)`, `link: send skipped (CLIPBOARD)`. Text is never logged, only its length and the first 8 hex digits of its SHA-256
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

It serves for `KIN_FAKE_PC_SECONDS` (default 300). Drive it by writing a word into `app/build/fake-pc/command.txt`: `ping`, `close4001`, `close4003`, `unpair`, `hello2` (a future-protocol hello) or `stop`. Milestone 1.2 adds `clipboard <text>`, `text <text>`, `url <link>` (a `text` with `isUrl`), `noack` / `ack` (stop / resume acking the phone's `text`) and `errtext` (answer it with an `error`). It logs what it saw to `events.txt`. While it runs it holds `app/build` open, so stop it (`stop`) before the next Gradle build.

## The certificate fixture

The certificate produced by the app's own `CertificateBuilder` lives in **one place**: `protocol/fixtures/android-client.{der,json}` (the desktop verifier tests use it too). The Android tests read it from there (`../protocol/` is a test resource directory), so there is no second copy. To regenerate it (not needed normally): `set KIN_EXPORT_FIXTURE=1` and run `gradlew.bat testDebugUnitTest --tests "*CertificateBuilderTest*"`. The new files are written to `app/build/fixture-export/` and have to be copied over `protocol/fixtures/` by hand (that folder belongs to the protocol owner).
