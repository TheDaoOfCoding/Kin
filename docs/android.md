# Android app

## Basics

- Kotlin, Jetpack Compose (kept basic), coroutines and Flow
- minSdk 29 (Android 10). targetSdk is the latest stable at the start of Phase 1
- **Libraries:**

  | Job | Library |
  |---|---|
  | HTTP, WebSocket, TLS | OkHttp |
  | JSON | kotlinx.serialization |
  | Database (devices, transfers, queue) | Room |
  | Settings | DataStore |
  | QR scanning | Google code scanner (ML Kit, through Play services). It doesn't need the camera permission. A scanner for phones without Google Play comes in Phase 2 |

- No dependency injection framework. The app is small enough to wire up by hand.

## Pieces

| Piece | Android type | Job |
|---|---|---|
| `LinkService` | Foreground service, type `connectedDevice` | Holds the connection, reconnects, sends the queue, receives |
| `ShareActivity` | Activity with no UI, for `ACTION_SEND` / `ACTION_SEND_MULTIPLE`, any MIME type | Sends text, or hands files to the transfer engine, then closes |
| `TransferEngine` | Plain class on the app's scope | Creates, runs, cancels, retries and recovers file transfers (uploads and downloads), one at a time per direction |
| `TransferActionReceiver` | `BroadcastReceiver`, not exported | The Cancel and Retry buttons of the transfer notifications |
| `ProcessTextActivity` | Activity with no UI, for `ACTION_PROCESS_TEXT` | "Send to PC" in the text selection menu |
| `ClipboardSendActivity` | Transparent activity | Reads the clipboard once it has focus, sends it, closes. Used by the tile and the notification button |
| `SendClipboardTile` | `TileService` | Quick settings tile |
| `BootReceiver` | `BroadcastReceiver` for `BOOT_COMPLETED` | Starts `LinkService` after a reboot |
| `MainActivity` | Compose | Pairing, status, history, settings, debug |
| Direct share shortcut | Sharing shortcuts API (`ShareShortcuts`, `res/xml/shortcuts.xml`) | Shows the PC in the share sheet's top row |

## Why sending the clipboard is manual

Since Android 10, only the focused app or the active keyboard can read the clipboard. So we read it only right after the user taps something: the tile, the notification button, or the text selection menu. `ClipboardSendActivity` takes focus for a moment so it's allowed to read, then closes. On Android 14+, the tile has to start it through the `PendingIntent` version of `startActivityAndCollapse`.

Writing to the clipboard from the background is allowed, so PC → phone is automatic. Android 13+ shows a small system confirmation when that happens. That's expected.

## Text and clipboard (built in 1.2)

- **Receiving `clipboard`:** set silently from the service, never stored, never shown. The clip is marked sensitive on Android 13+, so the system's confirmation doesn't show a preview of it. The log holds only the length and a short hash.
- **Receiving `text`:** copied to the clipboard and shown as a notification "Text from <PC>" with a preview (the lock screen shows only the title). A text with `isUrl` gets an **Open** action only if the text really is one `http` or `https` URL (the phone checks again and never trusts the flag). History comes in 1.4; until then the de-duplication memory is all that remembers a text. Messages are delivered in the order they arrived.
- **Sending:** `ShareActivity`, `ProcessTextActivity` and `ClipboardSendActivity` start `LinkService` while they are on screen (a visible activity may), hand the text to a sender that runs on the app's scope, and finish at once. The sender waits up to 4 s for a connection, sends one `text` and waits 5 s for the ack. With no connection it shows a "PC not reachable" notification and sends nothing; with no ack it shows "No reply from <PC>" (the message may or may not have arrived). Nothing is queued until 1.4. Text over 128 KiB (UTF-8, wire form) is sent as a `.txt` file through the ordinary upload (1.3); a sensitive text over 128 KiB is not sent at all (a file has no `sensitive` mark).
- **Line ends (1.3):** the wire uses LF only. The sender turns CRLF into LF in one pass and leaves a lone CR; the receiver keeps the text as it came. The loop-guard hash is taken on the wire form: a received text is hashed as received, a clipboard text is converted first.
- **Sensitive text (1.3):** the phone sets `sensitive` on a `text` when the clip it sends carries `ClipDescription.EXTRA_IS_SENSITIVE` (API 33+; the clipboard send path, and a share whose clip data carries it). A received `sensitive` text is put on the clipboard marked sensitive, and its notification says "Text from <PC>" with no preview and no Open action. Nothing about it is logged, not even its length or hash.
- **Openable links (1.3):** the `isUrl` rule and the Open action use the shared rule of protocol.md ("Links: `isUrl` and openable URLs"), in `UrlRules`, with its own whitespace list, port and 8 KiB checks.
- **Loop prevention:** the hash of the last text the PC put on the clipboard is remembered for 60 s. A "send clipboard" of the same text in that time is skipped. Share and "Send to PC" are explicit and never skipped.
- **Arrival time:** the time from the tap (the activity's creation) to the PC's ack is kept for the Debug screen and the debug state dump.
- **Permission state:** while `ACCESS_LOCAL_NETWORK` is missing (API 37), `LinkService` makes no connection attempt and its state, the Home screen and the notification say "Nearby devices permission needed".

## LinkService

- **Started when:** the app opens, after pairing, after a reboot, or when the user shares something. Starting from a visible activity is always allowed.
- **Notification:** low importance and silent. It reads "Connected to Desk PC", "Looking for Desk PC", "Not on Wi-Fi" or "Nearby devices permission needed", with a "Send clipboard" action.
- **Network:** a network callback watches for Wi-Fi and keeps track of every Wi-Fi network that is up. When Wi-Fi becomes available, the service connects over the newest one. When that one is lost and another Wi-Fi network is still up, it moves to that one; only when the last one is gone does it close cleanly, say "Not on Wi-Fi" and stop trying.
- Returns `START_STICKY`, so Android may restart it if it's killed. OEM battery managers often prevent that.
- **Wake locks:** none while idle. During a transfer, a partial wake lock is held only while bytes are moving.

## Files (built in 1.3)

### Receiving files

- On an `offer` the phone checks free space *before* the ack. If the files don't fit it answers with an `error` (`disk_full`) instead of the ack. Otherwise it acks, stores the transfer in Room (`transfers`, `transfer_files`) and downloads on its own coroutine: the WebSocket is never blocked by file I/O.
- Photos go to `Pictures/Kin` and videos to `Movies/Kin` through MediaStore, so they appear in the gallery. Everything else goes to `Download/Kin`. The folder follows the file's extension when that says what it is, and the sender's MIME type only when the extension says nothing (a `.exe` that claims `image/jpeg` is a download).
- Files are written with `IS_PENDING=1`, so half-written files show up nowhere. The flag is cleared when that file is complete, so photos appear one by one.
- **Names:** cleaned per protocol.md ("File names", `FileNames`), then created new, never overwriting. Scoped storage shows the app only its own files, so `existing` is what the app listed plus what unfinished transfers claimed. MediaStore may still rename on insert, and **the name MediaStore chose is the one stored and shown**.
- **Resume (R7):** the pending entry keeps its bytes, and the next `GET` asks for the rest with `Range: bytes=N-` (never for a 0-byte file). N is the entry's real length, not a counter. A `200` to a range request restarts the file at byte 0, a `416` starts it over once. Appending works through `openOutputStream(uri, "wa")` and through a read-write descriptor positioned at the end, on Android 10 and on Android 17, and the app uses the second (it can also cut off a longer entry). The fallback "write in the app's folder and move it" is **not needed**. A brand new pending entry has no file yet: opening it for reading fails with `ENOENT` until it was opened for writing once, so `MediaStoreSink.create` opens it for writing at once. (The emulator found this: without it the app took the entry for deleted and made new ones in a loop. It now also gives up after two re-creations.)
- **A file that fails for good** (`404`, `409`) fails the whole offer: partial files are deleted, a `cancel` with reason `error` goes to the PC, files already saved stay.
- **A dropped connection** is not final. The transfer waits (no wake lock, no polling) for the PC to come back, up to 15 minutes, then goes back to `queued` and resumes when the link is up again.
- When done: `complete` (re-sent with the same ID if not acked) and a notification "Photo from <PC>", "Video from <PC>", "File from <PC>" or "<n> files from <PC>" with Open (the first file) and Share (all of them).

### Sending files

- Share intents only grant temporary access to the shared files, and `ShareActivity` closes at once. It therefore passes the URIs to `LinkService` in the start intent with `FLAG_GRANT_READ_URI_PERMISSION`, and finishes only after the transfer was created. **Verified in 1.3 (R5):** the service's grant outlives the activity and the sending app; without it the files are unreadable within seconds (control run). Only `content:` URIs are accepted from an intent.
- **Reading a shared file uses an *unstable* provider client** (`acquireUnstableContentProviderClient`), not `ContentResolver.openFileDescriptor`: that one keeps a stable reference to the provider for as long as the descriptor is open, and Android then kills Kin when the owning app (a gallery) dies mid-upload. Seen on the emulator.
- **Upload:** `POST /v1/transfers`, then one `PUT` per file from its offset. `409 offset_mismatch` continues from the offset the PC returns; `507`, `413`, `500` fail the transfer with no retry; a dropped connection waits and resumes (`POST` again); a source that changed size or modified time means `DELETE` and a new transfer ID. A cancel sends `DELETE` (a `404` on it counts as done).
- **Queued while offline:** the transfer stays `queued` with a "PC not reachable" notification (Retry, Discard), and files under 50 MB are copied into `cacheDir/queue/<transfer>/`. The persistent queue is 1.4: it only has to call `TransferEngine.retryQueuedUploads()` when the PC comes back. A larger file that lost access fails with "Share it again". Cached copies go when the transfer ends, and after 24 hours.
- **Progress** is a notification with Cancel, updated at most every 400 ms.
- **Wake lock:** a partial wake lock is held only around a `PUT` or a `GET` that moves bytes, and released at once (reference counted, with a 30-minute safety timeout).

### Which foreground service type carries a transfer

Transfers run inside the existing `LinkService`, which is a `connectedDevice` foreground service. Reasons: (1) it is already running and keeps the process in the foreground state for the whole transfer, with no extra start; (2) `connectedDevice` has no time limit, while a `dataSync` service gets about 6 hours per 24 on Android 15 and can't be started from a boot receiver; (3) `dataSync` needs one more permission (`FOREGROUND_SERVICE_DATA_SYNC`), and this document's list says no more; (4) a user-initiated data transfer job (Android 14+) would add a second mechanism that runs on a job constraint (network type) instead of "the PC is on this Wi-Fi", and only for API 34 and later, while minSdk is 29. A 2 GB video over Wi-Fi takes minutes, far from any limit. If the service is killed, the transfer rows are `active`: the next `LinkService` start resumes them when the link is up (uploads need the source access to have survived: otherwise "Share it again").

## Permissions

This is the expected list. Confirm it on Android 14 and 15 during Phase 1.

| Permission | Why |
|---|---|
| `INTERNET`, `ACCESS_NETWORK_STATE` | Networking |
| `CHANGE_NETWORK_STATE` | One of the permissions Android 14+ requires for a `connectedDevice` foreground service |
| `FOREGROUND_SERVICE`, `FOREGROUND_SERVICE_CONNECTED_DEVICE` | LinkService |
| `POST_NOTIFICATIONS` | Notifications. Requested at runtime on Android 13+ |
| `RECEIVE_BOOT_COMPLETED` | Restart after a reboot. Check that a `connectedDevice` service can still start from boot on Android 15+ |
| `WAKE_LOCK` | Only while file bytes move (added in 1.3) |
| `CHANGE_WIFI_MULTICAST_STATE` | Only if mDNS lookups need it on the test phones |
| `ACCESS_LOCAL_NETWORK` | **Runtime permission on Android 17 (API 37), the "Nearby devices" group.** An app that targets API 37 needs it for any connection to a local network address (TCP, UDP, multicast, `NsdManager`), which is every connection to the PC. Without it a connect just times out, with no error that says why. Requested with `POST_NOTIFICATIONS` when the app opens and before a pairing starts. The user can revoke it later in settings, so Home shows an "Allow in settings" card while it is missing. It doesn't exist before Android 17. Found in the 1.1 emulator runs |

No location, no camera (the code scanner doesn't need it), and no storage permission (on Android 10+, MediaStore lets an app write its own files without one). Checked in 1.3 on Android 10 and 17: files are saved and shown in the gallery collection with no storage prompt.

## Battery and OEM killers

- **Onboarding:** opens the app's battery settings page and shows OEM-specific steps, such as Xiaomi's autostart toggle or Samsung's "Never sleeping apps" list.
- **Detecting kills:** if the service died without a clean stop, count it on its next start. Kills show on the debug screen and in the soak logs.

## UI (basic)

Screens: Onboarding and pairing (scan), Home (PC status and recent history), History, Settings, Debug.

## Testing

- Unit tests for protocol parsing, using the test vectors in `protocol/`.
- Manual testing follows the matrix in [targets.md](targets.md).
