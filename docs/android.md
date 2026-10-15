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
  | QR scanning | ZXing core (Apache-2.0) decoding CameraX frames on the phone. No Google Play services and no network (D16). Needs the `CAMERA` permission, asked for when the user taps "Scan code" |

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
| `MainActivity` | Compose | Pairing, Home, History, Settings, Debug, the setup (onboarding) |
| `TextOutbox`, `OutboxFlusher` | Plain classes on the app scope | The queue (1.4): texts saved first, flushed in creation order after the `hello`, with uploads |
| `HistoryRepository` | Plain class | The history list from the database (1.4) |
| Direct share shortcut | Sharing shortcuts API (`ShareShortcuts`, `res/xml/shortcuts.xml`) | Shows the PC in the share sheet's top row |

## Why sending the clipboard is manual

Since Android 10, only the focused app or the active keyboard can read the clipboard. So we read it only right after the user taps something: the tile, the notification button, or the text selection menu. `ClipboardSendActivity` takes focus for a moment so it's allowed to read, then closes. On Android 14+, the tile has to start it through the `PendingIntent` version of `startActivityAndCollapse`.

Writing to the clipboard from the background is allowed, so PC → phone is automatic. Android 13+ shows a small system confirmation when that happens. That's expected.

## Text and clipboard (built in 1.2)

- **Receiving `clipboard`:** set silently from the service, never stored, never shown. The clip is marked sensitive on Android 13+, so the system's confirmation doesn't show a preview of it. The log holds only the length and a short keyed hash.
- **Receiving `text`:** copied to the clipboard and shown as a notification "Text from <PC>" with a preview (the lock screen shows only the title). A text with `isUrl` gets an **Open** action only if the text really is one `http` or `https` URL (the phone checks again and never trusts the flag). An explicit text (a hotkey send) is recorded in the history (1.4); a sensitive text and the automatic `clipboard` never are. Messages are delivered in the order they arrived.
- **Sending (queue-first since 1.4):** `ShareActivity`, `ProcessTextActivity` and `ClipboardSendActivity` start `LinkService` while they are on screen (a visible activity may), hand the text to a sender that runs on the app's scope, and finish at once. The sender **saves the text as `queued` first** (the `texts` table), waits up to 4 s for a connection, and flushes the queue: one `text` at a time, each waiting 5 s for its ack. With no connection the text stays queued ("Saved. Kin sends it when <PC> is back"); with no ack it is **re-queued, not failed** (the message may or may not have arrived; it goes out again with the same ID, which the PC recognises). Only an `error` from the PC fails it. See "Queue, history, notifications (built in 1.4)". Text over 128 KiB (UTF-8, wire form) is sent as a `.txt` file through the ordinary upload (1.3), which is queued like any upload; a sensitive text over 128 KiB is not sent at all (a file has no `sensitive` mark).
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
- **Queued while offline:** the transfer stays `queued` with a "PC not reachable" notification (Retry, Discard), and files under 50 MB are copied into `cacheDir/queue/<local ID>/`. When the link is up the queue flush starts it (1.4). A larger file that lost access fails with "Share it again". The cache folder is named after the transfer's stable **local ID** (`transfers.localId`), not its transfer ID, so it survives a new ID (a changed source starts over under a new one). Cached copies go when the transfer ends, and after 7 days (the queue's expiry).
- **A `404` to `POST` (1.4 ruling):** the PC ended that transfer for good, or the ID belongs to another device, or it was used and is over. An **automatic** resume (a reconnect, the queue) marks the transfer failed ("the PC no longer has it") and **never sends again**. Only the user's **Retry** may: it first `POST`s the old ID (a transfer the PC finished within 24 h is recognised and just finishes), and on `404` sends **only the files that never got a `200`** under a new ID, as a new transfer; the old transfer stays failed and points at it (`retriedAs`), and the cache is shared through the local ID.
- **Free space during a download (1.4):** a new `offer` is checked against what the unfinished downloads *still have to write*, counted from the bytes really written (the in-memory `LiveProgress`), because the phone's free space already shrank by them. The stored `bytesDone` is only saved when a download stops.
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
| `CAMERA` | The QR scanner (1.5, D16). **A runtime permission, asked for only when the user taps "Scan code"**, never at start-up. The camera and autofocus are declared `required="false"`, so a phone without a camera can still install Kin and paste the pairing link. If the user says no, the pairing screen says so and the pasted link still works. After "don't ask again" the message points to the app's settings |
| `RECEIVE_BOOT_COMPLETED` | Restart after a reboot. Check that a `connectedDevice` service can still start from boot on Android 15+ |
| `WAKE_LOCK` | Only while file bytes move (added in 1.3) |
| `CHANGE_WIFI_MULTICAST_STATE` | Only if mDNS lookups need it on the test phones |
| `ACCESS_LOCAL_NETWORK` | **Runtime permission on Android 17 (API 37), the "Nearby devices" group.** An app that targets API 37 needs it for any connection to a local network address (TCP, UDP, multicast, `NsdManager`), which is every connection to the PC. Without it a connect just times out, with no error that says why. Requested with `POST_NOTIFICATIONS` when the app opens and before a pairing starts. The user can revoke it later in settings, so Home shows an "Allow in settings" card while it is missing. It doesn't exist before Android 17. Found in the 1.1 emulator runs |

No location, no camera except for the scanner (above), and no storage permission (on Android 10+, MediaStore lets an app write its own files without one). Checked in 1.3 on Android 10 and 17: files are saved and shown in the gallery collection with no storage prompt.

## Battery and OEM killers

- **Setup (onboarding, built in 1.4):** one screen, shown once after the first pairing and reopened any time from Settings → "Fix connection reliability". Each step shows done or to do: notifications (Android 13+), Nearby devices (Android 17), the battery list, and the steps for the phone's maker. It reads the real state again when the user comes back from a settings screen.
- **Battery exemption: the compliant route (decision).** Google Play restricts the permission `REQUEST_IGNORE_BATTERY_OPTIMIZATIONS` (behind the one-tap "Let Kin always run in the background?" dialog) to apps whose core function is hurt without it. Companion and device-connection apps are among the listed use cases, but the permission has to be declared in the Play Console and the review decides (the wording of the policy was not available to the agent: it was checked through secondary sources, see the 1.4 report). The route that needs **no permission and no declaration** is the system's own list, `Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS`, where the user sets Kin to "Not optimised" with two taps. **Kin uses that and does not declare `REQUEST_IGNORE_BATTERY_OPTIMIZATIONS`.** Whether it is done is read with `PowerManager.isIgnoringBatteryOptimizations` (no permission needed). If Phase 2 wants the one-tap dialog, it is one permission, one intent and a Play declaration (`SetupActions`).
- **OEM steps:** `Build.MANUFACTURER` (and `Build.BRAND` for Redmi and POCO) picks a guide from **one table**, `setup/OemTable.kt`: Xiaomi, Redmi and POCO (MIUI, HyperOS: Autostart, battery saver "No restrictions", lock the card), Samsung ("Never sleeping apps"), Oppo, Realme and OnePlus (ColorOS: startup manager, battery), Vivo and iQOO (autostart, background power), Huawei and Honor (App launch). Each guide has short text steps and a list of settings activities; the first that exists on the phone is opened, and if none does Kin's own settings page opens, so a stale entry costs nothing. Update the table when a vendor moves a screen. Other makers get only the general battery step.
- **Detecting kills:** if the service died without a clean stop, count it on its next start. Kills show on the debug screen and in the soak logs.

## Queue, history, notifications (built in 1.4)

- **The queue.** Every explicit send is stored as `queued` before it is attempted: a text in `texts`, a file upload in `transfers`. After the PC's `hello` the `OutboxFlusher` sends the queue in **creation order**, texts and uploads together: a text is sent and waits for its ack (5 s); an upload is only *started* (its bytes move on their own coroutine), so a big file doesn't hold back the texts after it. An unacked text is re-queued and the flush stops (the connection is treated as dead); the next `hello` flushes again. A new send while connected flushes the whole queue (older items first). One flush at a time. The **automatic clipboard is never queued** (it is not an explicit send, and nothing is stored for it). The phone's clipboard sends (tile, notification button) are manual, so they are explicit sends and are queued.
- **Expiry and cancel.** A queued text or an upload **that never started** and has waited **7 days** from its creation (`QueuePolicy`, expired at exactly 7 days; a Retry does not reset the creation time) fails with code `expired`, shows as failed ("Expired") in the history, and never goes out. An upload the PC has answered (`started`) follows the PC's `.part` rule (24 hours without progress) instead: the phone learns it from a `404` when it resumes, and never expires it at 7 days. Retry is not offered for an expired upload, one whose shared file can't be read any more (`source_lost`), or one meant for a PC that is no longer paired (`peer_changed`): their cached copies are gone. A user cancel of a queued item means it never goes out (`claim` is `queued → active` for one caller, so a cancel that came first wins; an item that is being sent can't be canceled).
- **Texts and PCs.** A queued item is for one PC. Unpairing cancels what was queued for it; an item for a PC that is no longer paired fails with `peer_changed`; an item for the paired PC stays queued while only another PC's connection is up (the re-pairing window).
- **History.** The `HistoryScreen` lists both directions, newest first, the last 500 (`QueuePolicy.HISTORY_LIMIT`; see "Storage" below for what is kept for older ones): time, direction, status, size, error. A sensitive text and an automatic clipboard sync are **never shown** (a sensitive text keeps its body only while it waits; a received sensitive text isn't stored at all). Tap a finished received file to open it, a finished text to copy it; **Retry** on a failed outgoing item (a failed text goes out again as a **new row** with a new message ID, and the failed row stays and says "Sent again as a new message"; a failed upload follows the `404` rule above). A **Retry made while the PC is away** is stored on the transfer (`userRetry`) and stays the user's until its run has reached its `POST`, so the queue flush that starts it after the reconnect may still send the files under a new ID on a `404`, **Cancel** on a waiting one. A file the phone sent is on the PC, so there is nothing to open.
- **Success rate (targets.md #1).** The Debug screen counts the sends that ended: delivered on the first attempt over delivered or failed. Expired, unpaired and wrong-PC items aren't sends gone wrong; a retried item never counts as a first try (`attempts`).
- **Notifications.** Three channels: *Connection status* (low), *Arrivals* (default: text, files and pings from the PC) and *File transfers* (low: progress, and the results of what the phone sent). The channels of earlier builds are deleted. Android drops **new** notifications once an app has 50, so each group (sends and progress; arrivals) keeps at most 10 transfers on screen (`NotificationLedger`: a new one pushes out the oldest) with a summary notification ("N file transfers") when there are two or more. A result is a new notification under its own tag, never an update, so it is never rate-limited away. Texts from the PC are **one updating notification** ("60 texts from Desk PC", the last five previews, a sensitive one counts but shows no preview), updated at most twice a second with the last update always posted (`TrailingThrottle`). The two **summaries are the only notifications that are updated**, and Android sheds updates of an app that posts more than about five a second (the last one, in a burst), so each summary is posted at most once a second with the last request always posted (`TrailingThrottle`). The ledger is the record of what the app posted, with the time of each post: it forgets a slot only when its notification is missing from `activeNotifications` **and** was last posted more than 2 s ago, because that list lags a fresh `notify` (a 15-file storm now leaves 10 notifications and a summary that says 10). Send problems keep at most 5 notifications, on the **Arrivals** channel (default importance: a "Not sent" notice has to be seen).
- **Database.** Version 4 (`app/schemas/` has the exported schema of every version; the 2 → 3 and 3 → 4 migrations are tested on a real SQLite): `texts` and, on `transfers`, `localId`, `retriedAs`, `attempts` (v3) and `userRetry`, `started`, `owed`, `owedId` (v4), `retriedAs` on `texts`, and indexes for the queue and history queries. `rename`, `delete` and `supersede` are single transactions, and a transfer that is over is never rewritten by a late status change.
- **The PC in every request.** A connection knows the PC it is pinned to (`LinkConnection.peerDeviceId`); a transfer, an offer, a cancel and a queued text are only started, accepted, cancelled or recorded for that PC. Right after a re-pair the old PC's connection is briefly still the current one, and none of them may go to it.

- **`complete` and `cancel` are owed until acked (1.4 review, protocol item 52).** A download that ends writes the message it owes the PC (`complete`, or `cancel` with reason `user` or `error`) **in the same database write as its status** (`owed`, `owedId`). It is sent at once if there is a connection, and **again after every `hello`, before the queue flush** (`OwedMessages.resendAll`, called from `TransferEngine.onLinkReady`), under the same message ID, until the PC acks it (an `error` answer is final). It survives a dropped connection and a process restart. An `offer` whose ID the phone knows is decided by `TransferEngine.offerHistory`: the **same paired PC with the same file IDs and sizes** is a *repeat* (the first response, the ack, and nothing downloaded again; an unfinished download carries on, a done one sends its unacked `complete`, a canceled or failed one its unacked `cancel`); **anything else** (an upload of ours, another PC, other files, an ID a `text` used) is `bad_request` with `ref` = the offer ID. A `cancel` from the owning PC turns a transfer that failed with `not_found` into "Canceled on <PC>" (`peer_canceled`); it never changes `done`.
- **Storage.** A transfer ID is never used twice, so **every transfer row is kept for ever** and every text row too. Beyond the newest 500 the history shows, the file rows of finished transfers (names, URIs; kept while a message is still owed) and the bodies of finished texts are dropped (`QuerySql`, run on every connect, at start and once a day). The cached copies of an upload are swept at start unless the transfer is queued, active, or failed with its files and not retried (`liveLocalIds`); the 7-day age sweep that ignored this is gone.
- **Logs and the export.** A text is logged as its length and `sha=` + 4 bytes of **HMAC-SHA256 with a random per-install key** (`LogFingerprint`, `LogKeyStore`: the key is stored wrapped by the Android Keystore and never exported; with no key only the length is logged); a sensitive text is not fingerprinted at all, not even on the loop-guard path. File names are never logged. **Export logs** runs every file through `ExportRedactor` (the same rules as the desktop's `export.rs`): IPv4 addresses and ports become `<ip>`, the names of the PC (every PC the log names) `<pc>`, the phone's own name `<phone>`, a full device ID is cut to 8 characters, and the debug build's `debug: CMD` echo lines are dropped. A failed export (no space, no share target) shows a message instead of crashing.
- **Reconnect on a new network.** `LinkLoop.networkChanged()` resets the backoff and tries at once when the network came back or changed under a loop that is still connecting (a new loop for a new Wi-Fi network already started from the first step; `LinkRuntime` now also calls it when the addresses of the current network change or it becomes validated). Repeated failures on an unchanged network still climb 5 s, 30 s, 2 min, 10 min. A text whose send is cancelled while it waits for the ack goes back to `queued` instead of staying `active`.

## UI

Screens: Home (PC status, the three newest history items), History, Settings, Debug, the setup (onboarding), and the pairing screens (scan, confirm). Tabs: Kin, History, Settings; Debug and the setup open from Settings (and the setup after the first pairing).

- **Settings:** the device name sent in `hello` (a change applies at the next connection), "Fix connection reliability", the notification channels (links to the system's channel settings), a note on the clipboard (PC → phone is automatic, phone → PC is manual; there is no images-to-clipboard on the phone and no "open links automatically"), unpair with a confirmation, About.
- **Debug:** first-try success rate, recent errors with their codes, ping round trip and tap-to-ack (last and median of the last 20), the connect and disconnect history (read from the log), the service kill count, **Export logs** (a zip of the log files and a state dump of counts and codes, no secrets and no text, shared through the share sheet with a `FileProvider` grant), and the log tail.

## Testing

- Unit tests for protocol parsing, using the test vectors in `protocol/`.
- Manual testing follows the matrix in [targets.md](targets.md).
