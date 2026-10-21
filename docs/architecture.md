# Architecture

## Big picture

```
 Android phone                                    Windows PC (plugged in)
 +----------------------------+                   +----------------------------+
 | Share sheet, text menu,    |                   | Tray, hotkey, drop window, |
 | quick tile, notification   |                   | "Send to", toasts          |
 |             |              |                   |             |              |
 |   LinkService              |  WebSocket        |   HTTPS server (Rust)      |
 |   (foreground service) ----+------------------>|   mDNS advertiser          |
 |   OkHttp client -----------+------------------>|   Clipboard watcher        |
 |                            |  HTTPS PUT / GET  |                            |
 |   Room DB: devices,        |  (mutual TLS)     |   SQLite: devices,         |
 |   transfers, queue         |                   |   transfers, queue         |
 +----------------------------+                   +----------------------------+
```

## The core rule: the PC serves, the phone connects out

- The PC is plugged in, so it does the expensive work. It listens for connections and announces itself on the network with mDNS.
- The phone never listens and never announces itself. While both are on the same Wi-Fi, it keeps **one** idle, encrypted WebSocket open to the PC. An idle connection costs very little; push notifications work the same way. What drains battery is scanning, advertising and holding wake locks, and the phone does none of those while idle.
- When the PC sends something, it goes down that open WebSocket.
- Every HTTP request is started by the phone. File uploads are PUTs and file downloads are GETs. The PC only ever talks to the phone through the WebSocket.

What this means:

- **Phone → PC always works** when both are on the same network and the PC is on. Any user action starts the service if it isn't running.
- **PC → phone depends on the phone's service being alive.** If the OS kills it, sends wait in the queue until the phone reconnects. The Phase 2 wake-up push fixes this.
- **Phone to phone isn't possible in the MVP.** That's fine for now.

## Identity

- On first run, each install creates a P-256 key pair and a self-signed certificate. The certificate follows a fixed profile ([protocol.md](protocol.md#transport)): X.509 v3, ECDSA P-256 key, `ecdsa-with-SHA256`, a serial of at most 20 bytes, no critical extensions. The PC's TLS library rejects anything else, so a phone certificate outside the profile can't connect.
  - Android: a software key, stored encrypted with an AES-GCM key held in the Android Keystore. Keystore-resident private keys can't reliably be used for TLS client authentication through OkHttp/JSSE on every phone. A hardware-backed key can be an upgrade later.
  - Windows: stored in a file encrypted with DPAPI, so it's tied to the Windows user.
- **Device ID** = SHA-256 of the public key (SPKI). The short ID shown in the UI is the first 8 characters in base32.
- **Trust** is a list of pinned device IDs. There's no certificate authority and no account.

## Pairing (summary)

The full steps are in [protocol.md](protocol.md#pairing).

1. On the PC, the user clicks "Pair new phone". The PC shows a QR code containing its addresses, its device ID and a one-time token valid for 5 minutes. The same text is a `kinrelay://pair?...` link: the **Copy pairing link** button puts it on the clipboard, for a phone whose camera can't scan the code (the user sends the link to the phone and pastes it on the first-run screen).
2. The phone scans the code, or takes the pasted link, connects, and checks that the PC's key matches the device ID in the QR or link.
3. The phone sends the token along with its own certificate. The PC checks the token and saves the phone's device ID as trusted.
4. Both sides show "Paired". The PC also shows a "New device paired" notice with a way to remove it, in case someone else scanned the code.

## Finding each other

- The PC advertises the mDNS service `_kinrelay._tcp` with TXT records `id=<short ID>` (the 8-character short ID, no hyphen) and `v=1`. It listens on port 47821, or a random port if that's taken. The exact contract is in [protocol.md](protocol.md#discovery).
- When the phone tries to connect, it goes in this order:
  1. Race all the addresses that worked last time, with a 1.5 s timeout each. This is the instant path.
  2. Do an mDNS lookup for up to 5 s and match the `id` record against the short ID of the paired PC (the full device ID is checked in the TLS handshake).
  3. If both fail, back off: 5 s, 30 s, 2 min, then every 10 min. The backoff resets when the network changes, when the user does something, or when a connection has stayed open with an accepted `hello` for at least 60 s. A connection that merely opens, or closes within a minute, doesn't reset it (see [protocol.md](protocol.md#close-codes)).
- **When it tries:** when the service starts, when a Wi-Fi network becomes available, when the user does something, and when the backoff timer fires. It doesn't try on mobile data in the MVP, because there's no [bridge](#future-the-bridge) yet.

## Connection lifecycle

States: `offline` → `connecting` → `connected` → back to `connecting` when the connection drops.

- The phone opens the WebSocket at `/v1/ws`, and both sides send `hello`.
- **Heartbeat:** the phone sends a WebSocket ping every 4 minutes (a tunable number). The PC marks the phone offline after 10 minutes of silence, or immediately if a message isn't acknowledged within 5 s.
- **When the phone sleeps:** timers pause during deep sleep, which is fine. An incoming message from the PC wakes the phone.

## Sending

- **Small things** (text, clipboard, file offers, acks) go as JSON messages over the WebSocket.
- **File bytes** go over separate HTTPS requests, so a large file never blocks the control channel.
- **Every message except `ack` and `error` has an ID, and seven types are acknowledged:** `text`, `clipboard`, `offer`, `complete`, `cancel`, `unpair` and `ping`. The receiver acks each of them. The sender retries until it gets an ack or the send is canceled. The receiver ignores duplicate IDs, so retries are safe. `hello` has an `id` too, but isn't acked. `ack` and `error` have no `id` of their own: they point at another message with `ref`.

## Queue and resume

- Every outgoing item is saved to the local database as `queued` before it's sent. On reconnect, the queue is sent in order.
- Files resume from the byte offset the receiver already has. Partial files (`.part`) are kept for 24 hours.

## Storage

Both sides use the same tables:

| Table | Holds |
|---|---|
| `devices` | Device ID, name, platform, paired time, last seen, last working addresses |
| `transfers` | ID, direction, peer, kind (`file`, `text`), status (`queued`, `active`, `done`, `failed`, `canceled`), created and finished times, total bytes, error code |
| `transfer_files` | Transfer ID, file ID, name, MIME type, size, bytes done, local path or URI |

Automatic clipboard sync isn't stored in history, for privacy. Explicit sends are.

## Security model

**Protects against:**

- Someone on the same Wi-Fi reading traffic: all traffic uses TLS 1.3.
- Someone on the same Wi-Fi pretending to be your PC: the phone pins the PC's key.
- Someone connecting to your PC: they need a paired certificate or a valid, unused, unexpired pairing token.
- A photographed QR code: the token works once, expires after 5 minutes, and pairing shows a notice with a way to remove the device.
- Malicious file names: names are cleaned on receipt (see [protocol.md](protocol.md#file-names)).
- A received file running a program on the PC: every received file carries the Windows Mark of the Web (SmartScreen and Office Protected View apply to it), and a click on a toast opens only an allow-list of safe types (pictures, audio, video, PDF, text, macro-free Office files); any other file is only shown in its folder (see [windows.md](windows.md#what-a-received-file-may-do)).

**Doesn't protect against (MVP):** malware already on either device, or someone who has physically unlocked your phone or PC.

## Battery rules for the phone

- No wake locks while idle. During an active transfer, a partial wake lock is held only while bytes are moving.
- No polling and no scanning while connected.
- mDNS lookups happen only while reconnecting, for at most 5 s.
- No Bluetooth.

## Future: the bridge

The bridge is a server in the cloud that connects the phone and PC when they can't reach each other directly. We call it a bridge because it only joins the two sides and never sees what crosses it.

The bridge is only a different way to get a socket. The phone and PC still run the same TLS, HTTP and WebSocket protocol through it, end to end. Nothing above the transport changes. That's why it can be added in Phase 2 without changing the protocol.
