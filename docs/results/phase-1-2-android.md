# Phase 1.2 results: Android app (text, links, clipboard)

Done 2026-10-14 by the Android coding agent. The code is in `android/` (nothing is committed). How to build, test and drive it: [android/README.md](../../android/README.md). Milestone 1.1 for reference: [phase-1-1-android.md](phase-1-1-android.md). Screenshots from the emulator runs: [phase-1-2-android-screens/](phase-1-2-android-screens/).

Summary: the six fixes the 1.1 review required are done, and the phone now receives `clipboard` and `text`, sends `text` (share sheet, "Send to PC", clipboard tile, notification button) and waits for the ack. `gradlew.bat lintDebug testDebugUnitTest assembleDebug assembleRelease` passes: **180 tests, 0 failed, 1 skipped** (the opt-in fake PC), lint **"No issues found"**, release APK **3,486,675 bytes (3.33 MiB)**. Smoke-tested on `kin_api37` and `kin_api29` against the in-JVM fake PC on loopback only. **Not run:** the real desktop app (the desktop agent's 1.2 was still being written), a real phone, mDNS.

## Fixes the 1.1 review required

| # | Fix | Where | Test |
|---|---|---|---|
| 1 | **A pairing link from outside the app never pairs by itself.** Every `ACTION_VIEW` link opens a confirmation screen first: the PC name from the link, the short ID computed from `id`, and "This will replace <current PC>" if another PC is paired ("This PC is already paired" for the same one). Pairing starts only after Confirm. | rule: `pairing/PairingIntake.kt:46` (`decide`); state and `intake`/`confirm`/`cancel`: `pairing/PairingController.kt:44`, `:67`; entry: `MainActivity.kt:88` (`handleLink`); screen: `ui/ConfirmPairingScreen.kt`, shown by `ui/KinRoot.kt` | `PairingIntakeTest` (11), emulator API 37 and 29 (screenshots 01, 03, 10) |
| 1a | **In-app scan and paste skip the screen** (`PairingSource.IN_APP`). Reason: the user tapped Scan or Pair in Kin a moment ago, and those controls exist only on the first-run screen (no PC paired), so they can never replace a PC. This matches the new protocol.md item 31, which only mentions the scanner; **paste is my reading** (see ambiguities). | `pairing/PairingIntake.kt:55`, `MainActivity.kt` (`onPair`, `scan`) | `whatTheUserScannedOrPastedInTheAppStartsRightAway` |
| 1b | **Debug-only auto-confirm.** `adb shell "am start -a android.intent.action.VIEW -d '<link>' --ez app.kinrelay.android.AUTO_CONFIRM true"`. It lives in `src/debug/.../DebugHooks.kt`; `src/release/.../DebugHooks.kt` has the same object that always answers `false`. **Verified on the release APK:** the merged manifest has no `Debug*` component (aapt2 `dump xmltree`), and `classes.dex` holds none of `AUTO_CONFIRM`, `DebugHooks`, `DebugCommandReceiver`, `DebugClipboardActivity`, `SendClipboardAlias`, `SEND_TEXT` (all present in the debug APK). | `app/src/debug/java/app/kinrelay/android/DebugHooks.kt`, `app/src/release/java/app/kinrelay/android/DebugHooks.kt` | `PairingIntakeTest.theDebugAutoConfirm...`, the apk check above |
| 2 | **A temporary Keystore error no longer wipes the identity or the pairing.** `IdentityStore` replaces an identity only for: files incomplete, key file corrupt or in an unknown format (`CorruptKeyFileException`), `KeyMissingException` (the alias is gone: decrypt no longer creates a new key, `KeystoreKeyWrapper.kt:34`), `AEADBadTagException`, `KeyPermanentlyInvalidatedException`, `UnrecoverableKeyException` (also when wrapped in another exception), the stored key or certificate unparseable, or the certificate not belonging to the key. **Anything else** (`KeyStoreException`, `ProviderException`, `InvalidKeyException`, `IOException`, ...) throws `IdentityUnavailableException` and **touches no file**; `AppGraph` retries with growing delays (0.5 s ... 60 s), shows the problem on Home/Onboarding and in the debug state dump, and deletes **no** paired PC. Paired PCs are deleted only when the identity was really replaced. | `identity/IdentityStore.kt:35` (`IdentityFailure.classify` `:44`), `:92` (`loadOrCreate`), `:103` (`load`), `:132` (key/certificate match); `AppGraph.kt:96`, `:114` | `IdentityStoreTest` (17) |
| 2a | **Safe write order.** Key first, then certificate, each as write temp file, `fsync`, atomic rename. A crash in between leaves a key with no certificate, which the next load treats as incomplete (it was never used) and replaces. On load the certificate must belong to the key: a signature made with the private key must verify with the certificate's public key (the SPKI check); a mismatch is corrupt. | `identity/IdentityStore.kt:144` (order), `:166` (`writeAtomically`), `:153` (`keyMatchesCertificate`) | `theKeyIsWrittenBeforeTheCertificate`, `aCrashBetweenTheTwoWritesIsRecoveredOnTheNextRun`, `aCertificateThatDoesNotBelongToTheKeyIsTreatedAsCorrupt` |
| 3 | **Pairing errors show on Home when already paired** ("Pairing didn't work" card with the message and Dismiss; a running pairing shows its progress line). | `ui/HomeScreen.kt:58`, `ui/KinRoot.kt` | emulator: failed re-pair with a paired PC (screenshot 04) |
| 4 | **`text` handled fully** (see 1.2 below). The cut-off in `MessageVectorsTest` is gone: all 8 in-scope sequences, including the 1.1 ones with `text`, run to the end. | `protocol/MessageReceiver.kt:35`; `MessageVectorsTest.kt` | 35 valid + 123 invalid cases + 8 sequences |
| 5a | **Two Wi-Fi networks:** `WifiTracker` keeps every Wi-Fi network that is up; losing the newest moves the loop to the next newest, and only losing the last one shows "Not on Wi-Fi". | `link/WifiTracker.kt`, `link/LinkRuntime.kt:46`, `:60`, `:69` | `WifiTrackerTest` (9). Not run with two real networks (the emulator has one) |
| 5b | **No `runBlocking` in `onDestroy`.** The "stopped" marker is written on the app's scope, on a single-thread dispatcher shared with the "started" marker, so a stop can never overtake the next start. | `link/LinkService.kt:82`, `:48`; `AppGraph.kt` (`serviceBookkeeping`) | emulator (service stop/start). If the process dies in the few ms after `onDestroy`, the next start counts a kill that wasn't one |
| 5c | **A repeated `unpair` is acked again and closes the connection with 1000**, as the PC does. The spec says an `unpair` "acks, removes the sender, closes with 1000" and that a repeat gets the same response; it doesn't say whether the repeat closes (**no vector**). | `protocol/MessageReceiver.kt:146`, `:153` | `MessageVectorsTest` (own case) |
| 5d | **Unpair confirmation dialog** ("Unpair from <PC>?" / "Remove <PC>?"). | `ui/HomeScreen.kt:92` | emulator (screenshot 09) |
| 5e | **Fixture duplicate removed.** The three `android-generated-cert.*` files in `app/src/test/resources/fixtures/` are deleted. `CertificateBuilderTest` reads `protocol/fixtures/android-client.{der,json}` (the `protocol/` folder is already a test resource directory). The exporter now writes to `app/build/fixture-export/` (copy by hand). | `CertificateBuilderTest.kt`, `README.md` | `committedFixtureIsAValidProfileCertificateWithItsExpectedId` |
| 6 | **Docs and the permission state.** `ACCESS_LOCAL_NETWORK` is risk **R12** in `docs/risks.md`, and "denied / revoked later / older Android" are in the Phase 1 test plan (`docs/phases/phase-1-mvp.md`, "Permission cases"). In the app: requested when it opens and before a pairing (as in 1.1), a new `LinkState.PermissionNeeded`: no connection is attempted while the permission is missing, and Home, the first-run screen and the service notification say **"Nearby devices permission needed"** with an "Allow in settings" button. It is re-checked when the app opens (and the loop retried), and also while connection attempts fail, so a revoke while running shows up. | `link/LinkRuntime.kt:120`, `:151`; `ui/HomeScreen.kt` (`LocalNetworkCard`); `MainActivity.kt` | emulator API 37: `pm revoke` gives the state (screenshot 08), `pm grant` and opening the app reconnects. Not unit-tested (Android classes) |

**One more bug found and fixed on the way:** pairing a *different* PC while the service was running kept the connection loop of the old PC (its ID is pinned in the loop's TLS setup), so the new PC never connected until the service restarted. Now `LinkController.devicePaired()` rebuilds the loop (`link/LinkRuntime.kt:101`, `pairing/PairingController.kt:108`). It showed up on the API 29 emulator, which still had the integration run's PC ("DESK-PC") paired, and it was reachable before 1.2 too (not by any UI flow in 1.1, now by a confirmed link).

## The 1.2 features

| Feature | How it works | Where |
|---|---|---|
| **Receive `clipboard`** | Acked, then the clipboard is set silently from the service (main thread). Not stored, not shown, no notification. The clip is marked sensitive on Android 13+ (no preview in the system's confirmation). The log has `len=` and the first 8 hex digits of the SHA-256, never the text. | `protocol/MessageReceiver.kt:141`, `link/IncomingHandler.kt`, `clipboard/ClipboardPort.kt` |
| **Receive `text`** | Acked, copied to the clipboard, notification "Text from <PC>" with a preview (the lock screen shows only the title). With `isUrl` **and** the text really being one `http`/`https` URL: an **Open** action. The phone checks again and never trusts the flag. De-dup memory only (history is 1.4). | `link/IncomingHandler.kt`, `link/Notifications.kt:77`, `protocol/UrlRules.kt` |
| **Ordered delivery** | `DeliveryQueue`: two messages in a row can't swap places (a slow first one holds the second back). | `link/DeliveryQueue.kt`, `link/LinkLoop.kt` |
| **Send `text`** | `ShareActivity` (`ACTION_SEND`, `text/plain`) and `ProcessTextActivity` ("Send to PC") have no UI: they start the service while on screen, give the text to the sender on the app's scope, and finish. `isUrl` is set per the spec (one whole http(s) URL, trimmed, no whitespace). Over 1 MiB (whole JSON frame): "too long for now", logged, nothing sent (the `.txt` file is 1.3). | `share/*.kt`, `link/OutgoingSender.kt`, `protocol/Messages.kt` (`Frames.text`) |
| **Send the clipboard** | `ClipboardSendActivity` (transparent; reads once it has window focus, with a 1.5 s fallback), started by `SendClipboardTile` (`PendingIntent` overload of `startActivityAndCollapse` on Android 14+, the `Intent` one below; `unlockAndRun` when locked) and by a **"Send clipboard"** action on the service notification. | `clipboard/*.kt`, `link/Notifications.kt:55` |
| **Loop prevention** | The SHA-256 of whatever the PC just put on the clipboard (`clipboard` or the copy that comes with `text`) is kept for 60 s. A "send clipboard" of the same text in that time is skipped ("The PC already has this text."). Share and "Send to PC" are explicit and never skipped. | `clipboard/ClipboardGuard.kt` |
| **Sender reliability** | Waits up to 4 s for a connection (asks the loop to try now, starts the service), sends, waits **5 s for the ack**. No connection: a **"PC not reachable"** notification with the reason ("This phone isn't on Wi-Fi.", "Nearby devices permission needed...", ...). No ack: "No reply from <PC>" and the connection is treated as dead (as for any unacked message). `error` from the PC: "<PC> refused it". Nothing is queued or retried (1.4). Success: a toast "Sent to <PC>". | `link/OutgoingSender.kt`, `link/LinkController.kt:35`, `link/LinkSession.kt:132` |
| **Arrival time** | Tap to ack (from the activity's creation, or the debug command) in ms, saved and shown on the Debug tab and in `STATE` (`lastArrivalMs`) and in the log. Measured on the emulator: **6 to 15 ms** for `SEND_TEXT`, share and "Send to PC"; **39 to 179 ms** through the clipboard activity (it includes the wait for window focus). Loopback through the emulator's NAT, so only the app's own part is meaningful. | `data/KinSettings.kt`, `ui/DebugScreen.kt` |

## Hooks and adb commands

Full list with every command: [android/README.md](../../android/README.md) ("Automation hooks"). New in 1.2, all **debug builds only** (they are in `src/debug`, not in the release APK; protected by the `DUMP` permission, which only the adb shell holds):

```
# pair from a link without the confirmation screen (the integration agent: add this extra to the VIEW command in tools/e2e-1-1.ps1:121)
adb shell "am start -a android.intent.action.VIEW -d '<link>' --ez app.kinrelay.android.AUTO_CONFIRM true"
# send text (the same sender as the share sheet); result line: CMD SEND_TEXT result=Delivered(arrivalMillis=8)
adb shell am broadcast -a app.kinrelay.android.debug.SEND_TEXT -n app.kinrelay.android/.debug.DebugCommandReceiver --es text hello
# trigger "send clipboard" (the real path: a focused activity reads the clipboard)
adb shell "am broadcast -a app.kinrelay.android.debug.SET_CLIPBOARD -n app.kinrelay.android/.debug.DebugCommandReceiver --es text 'from the phone'"
adb shell am start -n app.kinrelay.android/.debug.SendClipboardAlias
# read the phone's real clipboard; logcat only: CMD CLIPBOARD len=21 sha=e5809efc text="..."
adb shell am start -n app.kinrelay.android/.debug.DebugClipboardActivity
adb logcat -d -s KinRelay | findstr "CMD CLIPBOARD"
# state, with the arrival time and the loop guard
adb shell am broadcast -a app.kinrelay.android.debug.STATE -n app.kinrelay.android/.debug.DebugCommandReceiver
```

The shell can't start the real `ClipboardSendActivity` (not exported: "Permission Denial ... not exported", seen on the emulator), hence the debug-only `SendClipboardAlias`. **Heads-up for the integration agent:** `tools/e2e-1-1.ps1` opens the pairing link without the extra, so on this build it stops at the confirmation screen. Add `--ez app.kinrelay.android.AUTO_CONFIRM true` to that command (I did not touch `tools/`).

## Tests

`gradlew.bat lintDebug testDebugUnitTest assembleDebug assembleRelease`: BUILD SUCCESSFUL, **180 tests, 0 failed, 1 skipped**. Lint: **No issues found**.

| Test class | Tests | New in 1.2 |
|---|---|---|
| `MessageVectorsTest` | 13 | now runs the `text-*`, `clipboard-*`, `duplicate-keys-*` cases and 3 more sequences: **35 valid, 123 invalid** cases, **8 sequences** (was 20 / 106 / 5). `to: pc` cases use a receiver that refuses `clipboard` (wrong direction). Only `text-frame-invalid-utf8` is skipped (PC side only) |
| `TextFlowTest` | 25 | **new**, real TLS 1.3 + WebSocket against the fake PC: PC sends `clipboard` (clipboard set, acked, sensitive, order of 20), PC sends `text` (notification, clipboard, ack, Open only for http/https, `javascript:`/`intent:`/`file:`/`ftp:`/`kinrelay:` never, repeated id acked twice and shown once, invalid text gets an `error` and the connection stays open, a failing clipboard write still notifies and acks); phone sends `text` (ack and arrival time recorded, `isUrl` cases, lone surrogate replaced, exactly 1 MiB sends and 1 MiB + 1 byte and the multibyte case are "too long", no ack: `NoReply` after 300 ms and after the **default 5 s** (measured 4.9 to 7 s window), `error`: `Refused`, no connection: "PC not reachable" and nothing sent, dead session, blank text); loop guard (echo skipped, only on the clipboard paths, forgotten after 60 s, different text sent) |
| `IdentityStoreTest` | 17 (was 3) | classification, every lost-key kind replaces, every temporary error leaves the files byte for byte and throws, crash between the two writes, write order, mismatched certificate, first-run encryption failure writes nothing |
| `PairingIntakeTest` | 11 | **new**: external link always asks, name and short ID, "replace", same PC again, https form, in-app starts, debug flag, bad links rejected |
| `UrlRulesTest` | 4 | **new**: 15 URLs that are, 31 that aren't, schemes that may never be opened |
| `ClipboardGuardTest` | 8 | **new**: echo, window, newest only, clock moved back, hash (SHA-256 of the wire text), fingerprint never holds the text |
| `WifiTrackerTest` | 9 | **new** |
| `LinkLoopTest` | 14 (was 13) | order of delivery with a slow first message |
| Others unchanged | `LinkSessionTest` 18, `PairingClientTest` 9, `TlsVectorsTest` 6, `StrictJsonTest` 12, `CertificateBuilderTest` 8, `DeviceIdVectorsTest` 7, `PairingUrlVectorsTest` 3, `DedupMemoryTest` 6, `LogTest` 5, `PreviousRunTest` 4, `FakePcSession` 1 skipped | |

`bun protocol/check-vectors.ts` was not run (I didn't change `protocol/`).

## APK size

| Build | Size |
|---|---|
| Debug | 37,655,936 bytes |
| Release, R8, unsigned | **3,486,675 bytes (3.33 MiB)**, +70,252 bytes over 1.1 (3,416,423). Target about 10 MB |

## Emulator results

Both runs: debug APK, fake PC on `127.0.0.1:47831` (reached as `10.0.2.2`), nothing else on the network, emulator and fake PC stopped afterwards. The adb server dropped a few times during the runs ("daemon not running", "device offline": another adb on this PC takes port 5037 back); I repeated those steps.

| Check | API 37 (`kin_api37`) | API 29 (`kin_api29`) |
|---|---|---|
| Deep link opens the **confirmation screen**, nothing is contacted (the fake PC saw 0 `POST /v1/pair`) | yes (01, 03) | yes (10); it also showed "This will replace DESK-PC" (a PC left paired by the integration run) |
| Cancel contacts nothing; Confirm pairs and connects | yes | yes (after the re-pair fix above) |
| Failed pairing while paired: card on Home, PC kept, Dismiss | yes (04) | not repeated |
| PC `clipboard` sets the real clipboard, acked (read with `DebugClipboardActivity`) | yes, `len=21` including an emoji and `é` | yes |
| PC `text`: notification, "Open" only on the https link, none on `javascript:alert(1)` | yes (`dumpsys notification`) | yes (titles and texts) |
| Share sheet lists Kin (system chooser) and sends | yes (05); `text sent (SHARE) ... 10 ms` | yes via `-n ShareActivity` |
| `ACTION_PROCESS_TEXT` | yes, 8 ms | yes, 12 ms |
| Notification action "Send clipboard" | yes (06): `tap to ack 179 ms` | same activity as the alias below |
| Quick settings tile renders, click sends | yes (07, `add-tile`, small tile): `138 ms` | yes (11, label "Send clipboard", subtitle "Fake PC"), 45 ms (the `Intent` overload) |
| `SendClipboardAlias` (debug) | not needed (shell can't start the real activity: confirmed) | yes, 39 ms |
| Loop guard: the PC's clipboard text is not sent back | yes: `send skipped (CLIPBOARD)` | not repeated |
| `noack`: "No reply from Fake PC" after 5000 ms | yes | not repeated |
| Wi-Fi off, then a send: "PC not reachable. ... This phone isn't on Wi-Fi." notification | yes | not repeated |
| `pm revoke ACCESS_LOCAL_NETWORK`: "Nearby devices permission needed" on Home and in the service notification, no connection attempt; `pm grant` and opening the app reconnects | yes (08) | n/a (no such permission) |
| Unpair asks first | yes (09), Cancel keeps the PC | not repeated |

**Not tested:** the real desktop app, a real phone, mDNS, the camera scanner, OEM battery managers, two real Wi-Fi networks, a send from a state where the service was killed (the activities start it while on screen; I only saw it running), a lock-screen tile click (`unlockAndRun`), notification action on API 29, Android 14/15/16 (only 29 and 37 exist here).

## Spec ambiguities and decisions

1. **Paste skips the confirmation.** protocol.md item 31 says a code read by the app's own scanner needs no extra step and a deep link needs confirmation. It doesn't mention the paste field. I treat it like the scanner (the user typed or pasted it in Kin on the first-run screen, which exists only with no PC paired). A reviewer who wants paste confirmed too only has to change one argument in `MainActivity.kt`.
2. **Unknown name:** the confirmation shows "Unknown PC" when the link has no `n` (as item 31 says).
3. **Repeated `unpair`** closes with 1000 (see 5c). Spec silent, no vector. Unreachable in practice: after an `unpair` the sender is removed from `devices`, so the repeat can only come on the same connection.
4. **A `text` frame's size** is measured on the serialized frame (as the spec says), so the longest sendable text is 1,048,576 minus 83 bytes (the other fields and a 36-character id), 1,048,493 bytes, as the vector says (checked to the byte in `TextFlowTest`).
5. **Blank text** (only whitespace) is not sent ("Nothing to send."). The spec allows any non-empty text; this is a UX choice for share/clipboard.
6. **The loop guard window (60 s)** is mine; the spec says only "a clipboard set by the other device isn't sent straight back". With manual sending there is no loop, so the guard is a safety net that can only ever suppress an identical, recent text.
7. **"Tap" for the arrival time** is the creation of the activity (or the debug command), not the real touch event: the tile and notification `PendingIntent`s are built ahead of time and can't carry a timestamp. It adds a few ms of activity start and, for the clipboard, the wait for window focus.
8. **`clipboard` marked sensitive** (Android 13+): not in the spec; it only hides the preview in the system's confirmation. `text` is not marked, because the user asked for it and the notification shows it anyway.
9. **No ack after 5 s means "may or may not have arrived".** The spec says to treat the connection as dead and, with a queue, to re-send with the same id. Without the queue (1.4) the user is told and nothing is re-sent, because a retry from the user would be a new message with a new id and could show twice.
10. **`isUrl` for the sender** uses my own small rule (scheme http/https case-insensitive, no whitespace or control characters inside, a non-empty host, optional port). It is stricter than "parses as a URL" in places (a host with an underscore is fine; `http://` alone is not). Both sides must agree on the same cases; the PC agent's rule should be compared against `UrlRulesTest`.
11. **For the desktop/protocol owner:** `protocol/README.md` ("Android fixture") still says `fixtures/android-client.der` was "copied from `android/app/src/test/resources/fixtures/android-generated-cert.der`", and the `description` in `protocol/fixtures/android-client.json` says the same. That path no longer exists. I didn't edit `protocol/`.
12. **`MessageReceiver` has a role switch** (`acceptsClipboard`) so the shared vectors, which address both roles, can run through the one receiver class. In the app the receiver is always the phone.

## Known gaps

- Text over 1 MiB is refused, not sent as a `.txt` file (1.3). No history, no persistent queue (1.4): a send with no connection is lost, and the user is told.
- Share targets are text only; a shared file says "Kin can only send text for now."
- The permission card's button opens the app's settings page. After a first denial Android stops showing the prompt, so the app asks again only through the settings.
- After a stop, the service's "running" marker is written a few ms later on another thread. A process killed in that window counts a kill that wasn't one.
- `LinkRuntime`, the activities, the tile and the notifications are covered by the emulator runs only, not by JVM tests.
