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
| `ShareActivity` | Activity with no UI, for `ACTION_SEND` / `ACTION_SEND_MULTIPLE` | Queues files or text for the PC, then closes |
| `ProcessTextActivity` | Activity with no UI, for `ACTION_PROCESS_TEXT` | "Send to PC" in the text selection menu |
| `ClipboardSendActivity` | Transparent activity | Reads the clipboard once it has focus, sends it, closes. Used by the tile and the notification button |
| `SendClipboardTile` | `TileService` | Quick settings tile |
| `BootReceiver` | `BroadcastReceiver` for `BOOT_COMPLETED` | Starts `LinkService` after a reboot |
| `MainActivity` | Compose | Pairing, status, history, settings, debug |
| Direct share shortcut | Sharing shortcuts API | Shows the PC in the share sheet's top row |

## Why sending the clipboard is manual

Since Android 10, only the focused app or the active keyboard can read the clipboard. So we read it only right after the user taps something: the tile, the notification button, or the text selection menu. `ClipboardSendActivity` takes focus for a moment so it's allowed to read, then closes. On Android 14+, the tile has to start it through the `PendingIntent` version of `startActivityAndCollapse`.

Writing to the clipboard from the background is allowed, so PC → phone is automatic. Android 13+ shows a small system confirmation when that happens. That's expected.

## LinkService

- **Started when:** the app opens, after pairing, after a reboot, or when the user shares something. Starting from a visible activity is always allowed.
- **Notification:** low importance and silent. It reads "Connected to Desk PC", "Looking for Desk PC" or "Not on Wi-Fi", with a "Send clipboard" action.
- **Network:** a network callback watches for Wi-Fi. When Wi-Fi becomes available, the service connects. When it's lost, the service closes cleanly and stops trying.
- Returns `START_STICKY`, so Android may restart it if it's killed. OEM battery managers often prevent that.
- **Wake locks:** none while idle. During a transfer, a partial wake lock is held only while bytes are moving.

## Receiving files

- Photos go to `Pictures/Kin` and videos to `Movies/Kin` through MediaStore, so they appear in the gallery. Everything else goes to `Download/Kin`.
- Files are written with `IS_PENDING=1`, so half-written files don't show up anywhere. The flag is cleared when the file is complete.
- **Resume:** keep the pending entry and append to it. Check in 1.3 that append mode works on every test phone. If it doesn't, write into the app's own folder and move the file when it's complete.
- On arrival, show a notification such as "Photo from Desk PC" with Open and Share actions.

## Sending files

- Share intents only grant temporary access to the shared files. `ShareActivity` closes immediately, so it must pass the URIs on to `LinkService` with `FLAG_GRANT_READ_URI_PERMISSION` to keep access. Verify this in 1.3.
- **Queued while offline:** access can be lost if the app process dies or the phone reboots. Copy files under 50 MB into the app's cache when they're queued. If a larger file loses access, mark it failed with "Share it again".
- Show a progress notification with Cancel.

## Permissions

This is the expected list. Confirm it on Android 14 and 15 during Phase 1.

| Permission | Why |
|---|---|
| `INTERNET`, `ACCESS_NETWORK_STATE` | Networking |
| `CHANGE_NETWORK_STATE` | One of the permissions Android 14+ requires for a `connectedDevice` foreground service |
| `FOREGROUND_SERVICE`, `FOREGROUND_SERVICE_CONNECTED_DEVICE` | LinkService |
| `POST_NOTIFICATIONS` | Notifications. Requested at runtime on Android 13+ |
| `RECEIVE_BOOT_COMPLETED` | Restart after a reboot. Check that a `connectedDevice` service can still start from boot on Android 15+ |
| `WAKE_LOCK` | Only during transfers |
| `CHANGE_WIFI_MULTICAST_STATE` | Only if mDNS lookups need it on the test phones |

No location, no camera (the code scanner doesn't need it), and no storage permission (on Android 10+, MediaStore lets an app write its own files without one).

## Battery and OEM killers

- **Onboarding:** opens the app's battery settings page and shows OEM-specific steps, such as Xiaomi's autostart toggle or Samsung's "Never sleeping apps" list.
- **Detecting kills:** if the service died without a clean stop, count it on its next start. Kills show on the debug screen and in the soak logs.

## UI (basic)

Screens: Onboarding and pairing (scan), Home (PC status and recent history), History, Settings, Debug.

## Testing

- Unit tests for protocol parsing, using the test vectors in `protocol/`.
- Manual testing follows the matrix in [targets.md](targets.md).
