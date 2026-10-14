# Kin Android app

Milestone 1.1 (connection spike): identity, pairing, a foreground service that holds one WebSocket to the PC, ping, local log. The contract is [docs/protocol.md](../docs/protocol.md); the design is [docs/android.md](../docs/android.md). Details of what was built and measured: [docs/results/phase-1-1-android.md](../docs/results/phase-1-1-android.md).

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
| `identity` | P-256 key, hand-rolled DER certificate (`CertificateBuilder`), device and short IDs, key storage (AES-GCM under an Android Keystore key) |
| `tls` | Pinning trust manager, client key manager, the TLS 1.3 OkHttp client, the address race |
| `protocol` | Strict JSON, message receiver (validation order, replies, de-dup), close-code rules |
| `pairing` | Pairing URL parser, `POST /v1/pair`, the pairing flow |
| `link` | `LinkService`, `LinkRuntime` (network callback), `LinkLoop` (connection order, backoff), `LinkSession` (one WebSocket), mDNS, notifications, `BootReceiver` |
| `data` | Room (`devices`, `seen_messages`) and DataStore settings and counters |
| `log` | Rotating file log with secret redaction |
| `ui` | Compose screens: onboarding, home, debug |

## Automation hooks (for the emulator and the integration agent)

All of these assume the emulator is running (`tools\emulator\start.ps1`) and the debug build is installed (`tools\emulator\install.ps1`). Use `adb -s emulator-5554`.

### Permissions (Android 17 / API 37)

Apps targeting API 37 need the runtime permission `ACCESS_LOCAL_NETWORK` ("Nearby devices") to reach any address on the local network, **including `10.0.2.2`**. Without it a connection just times out. The app asks for it before pairing. For scripts, grant it up front:

```
adb shell pm grant app.kinrelay.android android.permission.ACCESS_LOCAL_NETWORK
adb shell pm grant app.kinrelay.android android.permission.POST_NOTIFICATIONS
```

(On API 29 neither permission exists; `pm grant` of them is not needed.)

### Pairing: the deep link

```
adb shell am start -a android.intent.action.VIEW -d "kinrelay://pair?v=1&a=10.0.2.2:47821&id=<pc-device-id>&tk=<token>&n=Desk%20PC"
```

Quote the URL. The `https://<any host>/pair#...` form works the same way. Pairing and starting the service happen without further taps (once the permissions above are granted).

### Debug-only commands (a broadcast receiver)

`app/src/debug/` holds a receiver that exists **only in debug builds** (it is in `src/debug`, so release builds don't contain it). It requires the `DUMP` permission, which the adb shell has and no normal app has.

```
adb shell am broadcast -a app.kinrelay.android.debug.<COMMAND> -n app.kinrelay.android/.debug.DebugCommandReceiver [extras]
```

| Command | What it does | Result line (logcat tag `KinRelay`, also in the log file) |
|---|---|---|
| `PING` | Starts the service if needed and sends a `ping` to the PC | `CMD PING result=Rtt(millis=6.6)` / `NoReply` / `NoConnection` / `Failed(...)` |
| `STATE` | Dumps the state | `CMD STATE link=Connected(...) paired=<name> pcId=<id> lastAddresses=... lastPing=... connects=N disconnects=N kills=N phoneId=<id>` |
| `CONNECT` | A user action: starts the service, resets the backoff, reconnects now | `CMD CONNECT done`, then the usual `link:` lines |
| `UNPAIR` | The Unpair button | `CMD UNPAIR done` |
| `PAIR --es url "<link>"` | Pairs from a link (use the deep link instead when you can: starting the service from a background broadcast may be refused on Android 12+) | the `pairing:` lines |
| `SET_PING_INTERVAL --ei seconds N` | WebSocket ping interval, for the soak (default 240; applies at the next connection) | `CMD SET_PING_INTERVAL Ns ...` |

Wait for a result with logcat, for example:

```
adb logcat -c
adb shell am broadcast -a app.kinrelay.android.debug.PING -n app.kinrelay.android/.debug.DebugCommandReceiver
adb logcat -d -s KinRelay | findstr "CMD PING"
```

### Reading the state

- **Logcat:** `adb logcat -d -s KinRelay`. Every line has the tag `KinRelay` and looks like `<ISO time> <I|W|E> <component>: <message>`. Components: `app`, `identity`, `pairing`, `tls`, `link`, `service`, `boot`, `mdns`, `debug`. The lines that matter:
  - `link: connected to '<PC>' via <host:port>`: hello accepted
  - `link: connection ended: ... (code N, upgrade N, hello accepted for N ms) -> <ACTION>`
  - `link: disconnected: ...`
  - `link: ping received from '<PC>'` (and the "Ping from <PC>" notification)
  - `service: service started` / `service stopped` / `kill detected: ...`
  - `boot: BOOT_COMPLETED received` / `service start requested`
- **Log file** (a rotating pair, 256 KiB each, secrets redacted): `adb shell run-as app.kinrelay.android cat files/logs/kin.log` (and `kin.log.1`). Works for debug builds.
- **Notifications:** `adb shell dumpsys notification --noredact | findstr "Connected to Ping from"`.
- **Service:** `adb shell dumpsys activity services app.kinrelay.android` (look for `isForeground=true` and `types=0x00000010`).
- The Debug tab of the app shows the last log lines, the connect, disconnect and kill counts and the last ping.

### Other useful adb commands

```
adb shell svc wifi disable        # "Not on Wi-Fi", clean close with 1000
adb shell svc wifi enable         # reconnects
adb shell run-as app.kinrelay.android kill -9 <pid>    # a kill; the next service start logs "kill detected"
adb reboot                         # BootReceiver; look for the boot: lines
```

## A fake PC for the emulator (test code, not part of the app)

`app/src/test/.../testing/FakePcSession.kt` runs an in-JVM stand-in for the Windows app (TLS 1.3 with a mandatory client certificate, `POST /v1/pair`, `/v1/ws`) on **loopback only** (`127.0.0.1:47831`, reached from the emulator as `10.0.2.2:47831`). It accepts any pairing and writes `app/build/fake-pc/pair-url.txt`. It is opt-in:

```
set KIN_FAKE_PC=1
gradlew.bat testDebugUnitTest --tests "*FakePcSession*" --rerun
```

It serves for `KIN_FAKE_PC_SECONDS` (default 300). Drive it by writing a word into `app/build/fake-pc/command.txt`: `ping`, `close4001`, `close4003`, `unpair`, `hello2` (a future-protocol hello) or `stop`. It logs what it saw to `events.txt`. While it runs it holds `app/build` open, so stop it (`stop`) before the next Gradle build.

## The certificate fixture

`app/src/test/resources/fixtures/android-generated-cert.{der,pem,json}`: a certificate produced by the app's own `CertificateBuilder`, with its expected device ID. The desktop verifier tests use it to prove the PC accepts a real Android certificate. To regenerate (not needed normally): `set KIN_EXPORT_FIXTURE=1` and run `gradlew.bat testDebugUnitTest --tests "*CertificateBuilderTest*"`.
