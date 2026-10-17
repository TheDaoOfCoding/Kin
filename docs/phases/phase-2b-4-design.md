# 2B.4 design: presence and quick wins

Written 2026-10-17 by the design agent. It is the contract for the desktop coder and the Android coder, who build 2B.4 in parallel without talking to each other. The plan it implements is "2B.4" in [phase-2b-plan.md](phase-2b-plan.md). It follows the format and the conventions of [phase-2b-2-design.md](phase-2b-2-design.md) and [phase-2b-3-design.md](phase-2b-3-design.md): capabilities in `hello`, ephemeral messages, per-connection state, guards so a dev instance can't hurt the user. **No product code was written for this document.** Nothing in the repo was touched except this file and [phase-2b-5-design.md](phase-2b-5-design.md), which builds on it (2B.5 adds two fields to the `presence` message defined here, and reuses the low-battery state machine of 3.6).

How to read it:

- Section 3 (protocol) is the contract between the two sides. The desktop coder copies it into `docs/protocol.md` (**changelog 58**) and writes the vectors. The Android coder implements it from here until that lands.
- Section 4 is the per-feature settings model, which both sides implement the same way. Section 5 is Android only, section 6 is Windows only. They meet only through sections 3 and 4.
- **Evidence marks**, as in the 2B.2 and 2B.3 designs. **[V]** = verified here: code in this repo, the Android 37 SDK stubs (`javap`), or a read-only check I ran on this PC. **[D]** = quoted from official documentation I read while writing (URLs in section 13). **[M]** = from memory of the API, not re-checked: the coder confirms it in the first hour. **[U]** = unverified and needs a real phone or PC, or a visual check.

## 1. Decisions in one table

| # | Decision | Why (short) |
|---|---|---|
| 1 | **One new message family, `presence`: a full snapshot of "what is true about me right now"** (battery, pause), sent by both sides, ephemeral, replaces the previous snapshot. A field that is absent means "not shared or not applicable". 2B.5 adds `dnd` and `focus` to the same message. | One channel for all "state of the other device" data. No `*.status` message and no new `hello` per switch: **a side shares a field by putting it in the snapshot, and shows what it gets or not by its own switch** (nothing needs to be negotiated) |
| 2 | Capabilities: **`presence`** (both sides list it in every 0.2.0 `hello`) and **`ring`** (the phone lists it only while "Let my PC ring this phone" is on, and sends a new `hello` when that changes: the `notif` pattern of 2B.2). | `ring` can be switched off by the phone's owner; the PC must know before it offers the button |
| 3 | **Pause is a local, persisted state on each device, told to the peer inside `presence`**. It is a master gate checked next to each feature's own switch, **at the sender and at the receiver** (so a race, or a peer that ignores it, still changes nothing on screen). Pause gates new content, never cleanup (`notif.remove`, `notif.sync`, `media.clear`) and never explicit sends, which arrive **quietly** (3.5). | A pause that only the sender honours is a pause that fails when the peer is old or buggy. The gate being "an `if` at each entry" means nothing has to be started or stopped when pausing, and **no timer is needed to resume locally**: expiry is a wall-clock comparison. (The only timer is a one-shot `delay` that tells the peer when the pause ends, 3.4 c.) |
| 4 | **Quiet means: stored in the history, and nothing visible happens by itself** — no notification or toast, **no clipboard write, no auto-opened link, no image to the clipboard**. Files are still saved. | The plan says "no notification or toast"; opening a browser or overwriting the clipboard would be louder than a toast, so they are part of "quiet" |
| 5 | **Ring my phone works while paused** and ignores Do Not Disturb except "total silence". | A lost phone is the reason the feature exists; a pause set an hour ago must not make it useless |
| 6 | **Ring runs inside the existing `LinkService`**: `MediaPlayer` on `USAGE_ALARM`, no new foreground-service type, no audio-focus request. The Stop screen is a **full-screen intent** where Android allows it, and a heads-up notification with a Stop button where it doesn't. | Kin plays no media; a `mediaPlayback` type would misstate the service (2B.3 decision 6). Android 14+ restricts full-screen intents to calling and alarm apps, so the design must work without it (4.4 and 5.4) |
| 7 | **Auto-sent screenshots are never queued**: with no ready connection after the 5-second hold they are dropped and counted. They carry `origin` (`screenshot` or `camera`) in `POST /v1/transfers`, an optional field old PCs ignore. A baseline is taken when the feature is turned on, **so nothing that already exists is ever sent**. | A screenshot sent three days later that overwrites the PC's clipboard is a bug, and a "send everything I have" on turning on is a privacy accident |
| 8 | **A screenshot waits 5 seconds on the phone with a Cancel button** before it leaves. | Screenshots capture passwords, codes and balances. This is the only chance to say no |
| 9 | **Unlinking: one word, "Unlink", on both sides, with a confirmation that says what happens.** No new wire message (`unpair` exists). Work that *is* needed: the device list is written **durably** (the v0.1.1 WAL setting can lose a removal in a power cut), and **in-flight transfers of a removed device are aborted** (today they are not, **[V]** `core.rs:638-647`). | "Its key stops working immediately" must be true for a thief's phone in the middle of an upload, and after a power cut |
| 10 | **Per-feature settings go through one resolver, `enabled(feature, peer)`** = per-peer override, else the default. Today only defaults exist (the flat keys already in `settings.json` and DataStore); the override store exists, is empty, and has no UI. | 2D (several devices) then adds UI and nothing else. No data migration now |
| 11 | **Battery data is shown, never stored.** It lives in memory until Kin quits; an offline phone shows its last known level ("last seen 14:32 · 12%"). Not in the database, not in `status.json` beyond numbers, not in logs. | Privacy rule of the project |

## 2. What I verified (read-only, no setting changed, no sound)

| Question | Result | Evidence |
|---|---|---|
| What does this PC report for power? | **No battery** (`Win32_Battery` empty, `PowerStatus.BatteryChargeStatus = NoSystemBattery`, line `Online`). So the laptop-battery path **can only be tested with a fake here**; a real laptop is the user's manual test | **[V]** probe, Windows 11 10.0.26300 |
| Public power API | `GetSystemPowerStatus` (`ACLineStatus`, `BatteryFlag` bit 128 = no battery, `BatteryLifePercent` 255 = unknown) is the documented call; no extra permission, no window | **[D]** + **[M]** |
| `SHQueryUserNotificationState` | Works unpackaged: returned `QUNS_ACCEPTS_NOTIFICATIONS` (5). Already used by `quiet.rs` | **[V]** probe and `desktop/src-tauri/src/quiet.rs` |
| The v0.1.1 database settings | `PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;` (`storage.rs:124`). In WAL mode with `NORMAL`, a commit is not synced to disk until a checkpoint, so **a power cut or OS crash can undo the last commits**. `remove_device` is one such commit | **[V]** code, **[D]** SQLite WAL documentation (secondary) |
| Does removing a device stop what it is doing? | `remove_device` removes the row, the pending cancels and the mirror state, and `connections.revoke` closes the WebSocket (`unpair` message or 4003). **Nothing in it touches an HTTP upload or download in flight** | **[V]** `core.rs:638-647` |
| Existing unlink UI | Android: **"Unpair"** on Home and in Settings, with a confirmation, and "Remove this PC" in the `PairAgain` state. Windows: a **"Unpair"** button per device in the Settings window, with a "Yes, unpair" confirmation. No tray item | **[V]** `HomeScreen.kt:102`, `SettingsScreen.kt:153`, `settings.ts:153` |
| Android 37 SDK | `NotificationManager.canUseFullScreenIntent()`, `getCurrentInterruptionFilter()`, `ACTION_INTERRUPTION_FILTER_CHANGED`; `TileService.ACTION_QS_TILE_PREFERENCES`, `isLocked()`, `unlockAndRun`; `StatusBarManager.requestAddTileService`; `MediaStore.MediaColumns.RELATIVE_PATH`, `IS_PENDING`, `OWNER_PACKAGE_NAME`, `DATE_ADDED`; permissions `READ_MEDIA_IMAGES`, `READ_MEDIA_VISUAL_USER_SELECTED`, `USE_FULL_SCREEN_INTENT`; `AudioAttributes.USAGE_ALARM` | **[V]** `javap` on `android-37.0` |
| Kin's target | `targetSdk = 37`, `minSdk = 29` | **[V]** `app/build.gradle.kts` |
| Full-screen intents, Android 14+ | "The default permission to send full-screen notifications can be granted only to apps that provide calling and alarm functionality." Google Play **revokes** it at install for other apps; the user can grant it in Settings → Special app access ("Manage full screen intents", `canUseFullScreenIntent()`, `Settings.ACTION_MANAGE_APP_USE_FULL_SCREEN_INTENT`). **When it is denied the notification appears as a heads-up for 60 seconds with its buttons** (a locked screen lists it first; an off screen turns on Always-on display). When granted: on an unlocked screen a persistent heads-up, on a locked or off screen the full-screen intent launches | **[D]** source.android.com FSI limits. The page says nothing about Do Not Disturb: **[U]** |
| Photo permissions, Android 13/14 | `READ_MEDIA_IMAGES` on 33+, `READ_EXTERNAL_STORAGE` up to 32. On 14+ an app that asks for `READ_MEDIA_IMAGES` and does not declare `READ_MEDIA_VISUAL_USER_SELECTED` runs in a **compatibility mode: if the user picks "Select photos", the grant is for the session only and is revoked once the app is in the background**. The page doesn't cover `ContentObserver` under partial access | **[D]** developer.android.com partial photo access |

## 3. Protocol (the contract)

Everything in this section goes into `docs/protocol.md` as **changelog 58** (57 is `media`; 2B.5 is 59), with vectors in `protocol/` (3.11).

### 3.1 Capabilities

- `hello.capabilities` gains **`presence`** and **`ring`** (case-sensitive).
- **`presence`**: every 0.2.0 build lists it. It means "I understand `presence`".
- **`ring`**: **only the phone** lists it, **only while** its setting "Let my PC ring this phone" is on (default on), and it sends a new `hello` on the open connection when the setting changes (the 2B.2 rule for `notif`; protocol.md already allows a later `hello`). It means "I understand `ring.start` and `ring.stop`". The PC lists nothing for `ring`: it only ever *receives* `ring.status`, which only a phone that listed `ring` sends.
- **A side sends `presence` only to a peer whose latest `hello` lists `presence`, and `ring.start`/`ring.stop` only to a peer whose latest `hello` lists `ring`**, checked when the message goes on the wire. Without the capability **nothing is sent** and the UI says why ("Update Kin on your phone" when there is no `presence`/`ring` at all, "Turn on ringing in Kin on your phone" otherwise: the PC can't tell the two apart and says "Ringing isn't available: turn it on in Kin on <phone>, which needs Kin 0.2.0").
- **A receiver that understands `presence` drops one silently** (no reply, counted) when its own or the sender's latest `hello` doesn't list it (validation step 6b, extended).

### 3.2 Classes

- **`presence` and `ring.status` are ephemeral** (the class of `notif.*`): carry a valid `id`, never acked, never de-duplicated, never stored, never queued, never re-sent. After a reconnect each side sends a fresh snapshot (3.4).
- **`ring.start` and `ring.stop` are acked, de-duplicated, never queued and never re-sent** (the class of `notif.action`: the user pressed something and wants to know). With no `ack` in 5 seconds the PC tells its user "no answer" and does **not** send again; on reconnect nothing is re-sent. **Ringing a phone ten minutes late is worse than not ringing it.**
- **Auto-sent images are not a message**: they are the existing file upload with one optional field (3.7).

### 3.3 Messages

| Type | Direction | Fields | Receiver does |
|---|---|---|---|
| `presence` | both | `id`, then optional `battery`, `pause` (2B.5: `dnd`, `focus`) | Replaces the previous snapshot of this connection |
| `ring.start` | PC → phone | `id` | Starts ringing (or restarts the 30 s), then `ack`; or `error` |
| `ring.stop` | PC → phone | `id` | Stops ringing, then `ack` (always, also when nothing rang) |
| `ring.status` | phone → PC | `id`, `state`, `reason` (optional), `audible` (optional) | Updates what the PC shows |

```json
{ "type": "presence", "id": "0193…", "battery": { "level": 62, "charging": true }, "pause": { "remainingMs": 3540000 } }
{ "type": "ring.status", "id": "0193…", "state": "stopped", "reason": "timeout" }
```

| Field | Type and rules |
|---|---|
| `battery` | Optional object `{level, charging, saver?}`. `level` integer 0 to 100 (**required**; over 100 or a non-integer is `bad_request`). `charging` boolean (**required**): **plugged in** (charging, or full while plugged). `saver` optional boolean: the device's power-saving mode is on. Absent = not shared, or this device has no battery (a desktop PC) |
| `pause` | Optional object with **exactly one** of `remainingMs` (integer 1 to 691,200,000: 8 days) or `untilResumed` (`true`). Both, neither or `untilResumed: false` is `bad_request`. Absent = **not paused**. A `remainingMs` over 8 days is cut to 8 days |
| `state` (ring.status) | **Required**, `ringing` or `stopped`; anything else is `bad_request` |
| `reason` (ring.status) | Optional, with `stopped`: `timeout` (30 s), `user` (Stop on the phone), `pc` (a `ring.stop`), `limit` (the 2-minute cap of 5.4), `error`. Unknown values are shown as "stopped" |
| `audible` (ring.status) | Optional boolean with `ringing`: `false` when Android will play nothing (Do Not Disturb "total silence", a volume-fixed device). Default `true` |

A **frame** over 4 KiB is `error too_large` (`ring.*` over 2 KiB). A `presence` with an unknown field ignores it (v1 rule). `null` for an optional field counts as absent; a wrong type is `bad_request` with `ref`, the connection stays open, the `id` is not remembered.

### 3.4 `presence` rules

- **Snapshot, not delta.** A received `presence` **replaces** everything that peer's previous `presence` said on this connection. `battery` absent → "battery unknown, show nothing"; `pause` absent → "not paused".
- **When it is sent.** (a) After both `hello`s are accepted, immediately (even if it carries nothing: `{}` tells the peer "not paused"). (b) When anything in it changes. **Battery-only changes** are sent when the level moved by **at least 2 points**, or crossed 15, 20 or 100, or `charging` or `saver` flipped, and **at most once per 30 s** (trailing). **A pause change is sent within 1 s** (trailing) and is never held back by the battery limit. A side never sends `presence` on a periodic timer; the only timer is the one-shot in (c). (c) **At the moment a pause expires**, by the pausing side (an alive side) so the peer doesn't have to guess.
- **Receiver bucket:** per connection, capacity 10, refill 1 per 3 s; over it → dropped and counted, one `error rate_limited` (no `ref`) per 10 s at most.
- **Per connection, remembered in memory only.** When the connection closes the viewer **forgets the peer's pause** (the peer is not heard from, so nothing is paused as far as this side knows) and keeps only the **last known battery** for the "last seen" line, until Kin quits.
- **The peer's pause as this side knows it:** `expiry = arrival (monotonic) + remainingMs`. The peer's *own* gate is a wall-clock comparison; the two differ by the network delay. This side resumes sending automatic things at `expiry`, **or when a `presence` without `pause` arrives, whichever is first**. A message sent just before the peer's expiry is dropped by the peer, which is harmless.

### 3.5 Pause: what stops, what still arrives

"I am paused" = this device's own state. The **gate** `paused()` is true while `now < pausedUntil` (or `untilResumed`). It is checked **in addition to** a feature's own switch.

| Feature | While **I** am paused, I… | While **my peer** is paused, I… |
|---|---|---|
| Automatic clipboard (`clipboard`, PC → phone) | (phone) drop incoming `clipboard` silently; (PC) send none | (PC) send none |
| Notification mirroring (`notif.post`) | (phone) post none; (PC) drop posts silently. **`notif.remove`, `notif.sync`, `notif.dismiss` still flow** (cleanup is not content) | (phone) post none |
| Media (`media.state`, `media.image`) | (source) send none and send `media.clear` once; (viewer) remove the shown session and drop the rest | (source) as for me |
| Auto-sent screenshots and photos | (phone) don't send, **and advance the baseline** so what was taken while paused is never sent after the pause | (phone) don't send |
| Presence | **keep sending** (the peer needs `pause` to show "Paused until…") | keep showing it |
| Low-battery nudge, other Kin toasts of automatic features | suppressed | — |
| **Ring** | **still works** (decision 5) | still works |
| `unpair`, `ping`, `ack`, `error`, file transfers already running | unchanged | unchanged |
| **Manual sends** (`text`, `link`, `offer`/uploads, 2B.5 `alert`) | **still arrive; handled quietly** (below) | still sent (a manual send is the user's own act: the sender is not paused by the peer's pause) |

**Quiet handling of a manual arrival** (the receiver is paused): the item is recorded in the history as usual; **no notification or toast; the clipboard is not written; a link is not opened; an image is not put on the clipboard; no "Open"/"Copy" action is shown anywhere**; files are saved to their folder. The receiver counts them ("3 received while paused", in memory, reset on resume) and shows the count on Home / in the drop window. A transfer's **progress** notification on the phone is the one thing that stays (it is a low-importance ongoing notification of something the user started); its final "arrived" notification is suppressed.

**Pause does not stop the connection, the queue, or transfers in flight.** It is not "offline".

**Choices** (both devices): **1 hour**, **until tomorrow** (= the next 08:00 local time that is at least 2 hours away: at 07:30 it means tomorrow 08:00, at 05:00 today 08:00; one constant in one place), **until resumed**. A second pause replaces the first. **Resume** clears it. State is persisted (`pausedUntil` as epoch ms in `settings.json` and DataStore; "until resumed" is a flag) and survives a restart and a reboot. A `pausedUntil` more than 8 days ahead of the current clock is treated as expired (a clock set backwards must not freeze Kin for a year). Unlinking the **last** peer clears it.

### 3.6 Low-battery nudge (a pure state machine, both sides)

The **receiver** of a `presence` decides, per peer, in memory until Kin quits (not per connection):

```
input: (level, charging) of each snapshot with a battery
armed starts true
if charging or level >= 20      -> armed = true
else if level <= 15 and armed   -> NUDGE, armed = false
```

So one nudge per discharge episode, even across reconnects, re-armed by charging or by reaching 20. A first snapshot after a connect that is already at 14 % and not charging nudges (if armed). **Suppressed while the receiver is paused** (and, in 2B.5, held while the receiver's phone DND is on: that is the phone's own notification system). The PC shows a plain toast "Pixel 8 is at 14% and not charging", no buttons. 2B.4 builds the PC's nudge for the phone; 2B.5 reuses the same machine on the phone for a laptop.

### 3.7 Auto-sent screenshots and photos: `origin`

`POST /v1/transfers` gains an **optional** field `origin`: `"screenshot"` or `"camera"` (anything else is ignored as absent). It is the only wire change, and old PCs ignore unknown fields (protocol.md JSON conventions), so **no capability** is needed: an old PC just shows its ordinary "Photo from <phone>" toast and puts one image on its clipboard.

What a **0.2.0 PC** does with `origin`:

| `origin` | Toast | Clipboard | Windows clipboard history |
|---|---|---|---|
| `screenshot` | "Screenshot from <phone>" with the image if the toast layer can show it | the image, **if `imagesToClipboard` is on** (default), with the **sensitive formats** (`ExcludeClipboardContentFromMonitorProcessing`, `CanIncludeInClipboardHistory` = 0, `CanUploadToCloudClipboard` = 0) so Win+V and the cloud clipboard don't keep it | not kept |
| `camera` | "Photo from <phone>" | **never** | n/a |
| absent (manual) | as today | as today | as today |

While the PC is paused it saves the file quietly (3.5) whatever the `origin`. The PC **never trusts `origin` for anything but wording and the clipboard rule**: a hostile phone can already put an image on the clipboard with an ordinary upload.

The phone sends these uploads with the existing transfer engine (one file, queue **not** used: 5.6). `origin` is on the **first** `POST` and on resumes (the PC keeps the first value).

### 3.8 Ring

1. PC → phone `ring.start`. The phone checks, in order: `ring` switch off → `error disabled`; more than **3 starts in 60 s** → `error rate_limited` (with `ref`); otherwise it starts (or, when already ringing, restarts the 30 s) and answers **`ack`** once the player is running (or, when nothing can be heard, once the notification is posted), then sends `ring.status {state: "ringing", audible}`.
2. The phone stops after **30 s** (`timeout`), when the user taps Stop (`user`), on `ring.stop` (`pc`), or at the **2-minute continuous cap** (`limit`: repeated `ring.start`s can't ring for ever; a 60 s pause follows before a new ring starts, `rate_limited` meanwhile). Each stop sends `ring.status {state: "stopped", reason}`.
3. `ring.stop` is always answered with `ack`, and sends `ring.status stopped` only if it stopped something.
4. The PC allows **1 `ring.start` per 10 s**.

### 3.9 Unlinking

No wire change. `unpair` stays as protocol.md says. What changes is behaviour on the PC (6.5) and the phone (5.7); the protocol document gets one sentence in the Unpair section: "a removal is **durable** before the removal is acted on, and a removed device's requests in flight are refused from then on."

### 3.10 Old peers and offline

- **0.1.x peer:** has no `presence`/`ring` in `hello`, so none of them is sent. The PC's tray shows no battery for it; "Ring my phone" is disabled with the text of 3.1; the pause toggle still works locally, and the PC tells the user "<phone> doesn't know about pausing: update Kin there" in Settings; a paused PC still **drops** automatic messages by itself (the receiver-side gate), which a 0.1.x phone (clipboard sync) will send. Auto-sent screenshots work with any PC (an old PC just handles them as photos, with the clipboard overwrite, which Settings on the phone says when the PC's `hello` has no `presence`: "Your PC needs Kin 0.2.0 to keep screenshots out of its clipboard history"; decision: **the phone refuses to turn on auto-send for a PC without `presence`**, because the clipboard-history protection is part of the feature).
- **Offline:** `presence`, `ring.*` and auto-sent images are dropped, nothing stored. The one exception to "nothing across connections" is the PC's own persisted pause, which is local.

### 3.11 Vectors to add (the desktop coder owns `protocol/`)

New file **`protocol/presence.json`**, new `presence-*`/`ring-*` cases in `messages.json`, a `scope-2b.4` group in `check-vectors.ts`, a section in `protocol/README.md`.

| Array | What it pins | Used by |
|---|---|---|
| `presence[]` | valid and invalid `presence` objects → accepted or `bad_request` + reason (level 101, `pause` with both/neither, `untilResumed: false`, `remainingMs` over 8 days cut, a string level, null fields, unknown field) | both |
| `pauseUntil[]` | `{localNow, zone, choice}` → expected local wall time, for `1h`, `tomorrow` (07:30, 05:00, 08:00 exactly, 23:59, across a DST change in `Europe/Berlin` and `America/New_York`) | both |
| `pauseGate[]` | `{self paused, peer paused, message type, manual or auto}` → `send`, `drop`, `quiet`, `flows` for each row of the 3.5 table | both |
| `pauseExpiry[]` | `(arrival, remainingMs, now, presenceWithoutPause?)` → resumed or not; a `pausedUntil` 9 days ahead → expired | both |
| `lowBattery[]` | a timeline of `(t, level, charging)` → `NUDGE` events (one per episode, re-armed at 20 or by charging, a reconnect at 14 %, paused receiver) | both (the PC in 2B.4, the phone in 2B.5) |
| `batteryMap[]` | Android `ACTION_BATTERY_CHANGED` extras (`level`, `scale`, `status`, `plugged`) and Windows `SYSTEM_POWER_STATUS` (`ACLineStatus`, `BatteryFlag`, `BatteryLifePercent`) → wire `battery` or none (flag 128, 255 unknown, scale 0, full while plugged) | both |
| `presenceRate[]` | a timeline → sent or dropped for the sender's 2-point / 30 s / pause-1-s rules and the receiver bucket | both |
| `ring[]` | a timeline of `ring.start`/`ring.stop`/timer events → phone replies and `ring.status` (30 s, restart, 3 per 60 s, 2-minute cap and its 60 s pause, `disabled`, idempotent stop) | Android (the PC replays it for its UI state) |
| `shot[]` | a MediaStore row snapshot `{id, relativePath, mime, isPending, dateAdded, now, ownerPackage, size}` plus `{baselineId, enabled, cameraEnabled, paused, own folder}` → `send` or the skip reason (see 5.6) | Android |
| `origin[]` | the PC's rule table of 3.7 → toast wording, clipboard yes/no, sensitive formats yes/no | PC |
| `features[]` | `(feature, peer, defaults, overrides)` → enabled (per-peer override, default, unknown peer) | both |
| `messages.json` | `presence-minimal`, `presence-full`, `presence-bad-*`, `ring-start-ack`, `ring-start-disabled`, `ring-start-rate-limited`, `ring-stop-idempotent`; sequences: `presence-not-sent-without-capability`, `presence-dropped-when-feature-off-is-silent`, `presence-is-not-acked-or-deduplicated`, `presence-snapshot-replaces-pause`, `presence-forgotten-on-close`, `ring-not-sent-without-capability`, `ring-capability-follows-the-switch`, `ring-start-is-not-resent-after-reconnect`, `ring-start-is-acked-once`, `ring-status-is-ephemeral` | both |

### 3.12 What changes in the existing docs and code (desktop coder does these)

- `docs/protocol.md`: Messages table (4 rows), Field types, a "Presence and ring" section holding 3.1 to 3.8, Message IDs and acks (the two classes), validation step 6b (`presence` and `ring.status` are dropped silently when the receiver's or the sender's latest `hello` doesn't list `presence`; **`ring.start`/`ring.stop` are different**: they are acked, so one that reaches a phone whose switch is off in a race answers `error disabled` with `ref` instead of being dropped, and the PC never sends one without the `ring` capability), Versioning (`presence`, `ring`), the Transfers `POST` body (`origin`), the Unpair sentence, changelog 58. No new error code.
- `desktop/src-tauri/src/messages.rs`: `CAPABILITIES` becomes `["link", "notif"?, "media", "presence"]`. Android `Messages.kt`/`MessageReceiver.kt`: `capabilities = [link, notif, media, presence, ring?]`.
- `docs/extension.md` already reserves `status.paused` (2B.4): the desktop coder fills it in `nativehost/api.rs` (`paused: true` while the PC is paused) and the extension keeps sending: a manual send is never blocked by a pause.

## 4. The per-feature settings model (both sides, the same shape)

**Goal:** when 2D adds several devices, "clipboard sync on for my phone, off for my tablet" is a UI change, not a data change.

- **A feature id** is a stable string. Each device stores, per feature, **a default** (today's flat keys, unchanged) and an optional **override per peer device ID**. The only function features call is `Features.enabled(featureId, peerDeviceId?) -> Boolean` = override if present, else default. A feature acting on a connection passes **that connection's peer ID**; one acting on nothing in particular passes none.
- **Storage:** desktop `settings.json` gets `"featureOverrides": {}`; Android DataStore `kin_features` gets one key `feature_overrides` (a JSON string). Both are empty and have **no UI in 2B.4**. Unlinking a peer **deletes its overrides** (in the same step that forgets it). Device IDs are not secret.
- **The ids and where today's default lives:**

| Feature id | Meaning | Today's default (unchanged) |
|---|---|---|
| `clipboard.sync` | PC → phone automatic clipboard | PC `autoClipboardSync` |
| `links.autoOpen` | PC opens links from the phone | PC `openLinksAutomatically` |
| `text.autoCopy` | phone copies received text | phone DataStore `copy_received_text_automatically` |
| `notif.share` | phone mirrors its notifications | phone `kin_mirror` master switch |
| `notif.show` | PC shows the phone's notifications | PC `mirrorNotifications` |
| `media.share`, `media.show` | 2B.3, both sides | `mediaShare`, `mediaShow` / `kin_media` |
| `presence.share` | send my battery | **new**: PC `presenceShareBattery`, phone `presence_share_battery` (default **on**) |
| `presence.show` | show the peer's battery | **new**: PC `presenceShowPhone`, phone `presence_show_pc` (default **on**) |
| `presence.nudge` | low-battery nudge | **new**: PC `lowBatteryNudge` (default **on**) |
| `ring.allow` | let the peer ring me | **new**: phone `ring_allow` (default **on**); the PC has no such switch |
| `shots.screenshots`, `shots.camera` | auto-send | **new**: phone `shots_screenshots`, `shots_camera` (default **off**, both) |

**What stays global, by design:** *Pause* (one state for the device; per-peer pause is a 2D question), the device name, the hotkey, the received folder. **UI in 2B.4:** a plain list of switches per feature (Settings on both sides), exactly as today plus the new rows; **no peer picker**.
- **Test:** `features[]` vectors (3.11) and a test that every new switch reads through the resolver (a grep-style test: no feature code reads its flat key directly).

## 5. Android

### 5.1 Pieces (package `presence/`, `ring/`, `shots/`, `pause/`; names are suggestions)

| Piece | Type | Job |
|---|---|---|
| `PresenceWire`, `PresenceSender`, `LowBatteryNudge`, `PauseGate`, `PauseChoice` | pure | The message and its limits, 3.4's sender rules with a fake clock, the 3.6 machine (JVM-testable), the gate of 3.5, `pauseUntil[]` |
| `BatteryReader` | Android-bound, thin | The sticky `ACTION_BATTERY_CHANGED` read, plus a dynamic receiver **only while a session is ready and `presence.share` is on** |
| `PauseStore` | DataStore (`kin_pause`) | `paused_until_ms` (0 none), `until_resumed`. No content |
| `PauseTile`, `PauseSettingsActivity` | `TileService`, Compose | 5.3 |
| `RingController`, `RingPlayer`, `RingActivity`, `RingActionReceiver` | Android-bound | 5.4 |
| `ShotWatcher`, `ShotPicker`, `ShotHold` | `ContentObserver`, pure, Compose-free | 5.6 |
| `Features` | pure + DataStore | The resolver of section 4 |

Wire into the existing link like 2B.2/2B.3: `LinkSession` sends `presence` after each accepted `hello` and on change, `IncomingHandler` handles `presence`, `ring.start`, `ring.stop`; `Messages.kt`/`MessageReceiver.kt` get the types. **`PresenceSender` and `RingController` are the only writers** of `presence` and `ring.status`; they check the peer's capability per message and **never start `LinkService`**.

### 5.2 Presence (phone)

- **Sending.** Battery: `registerReceiver(null, IntentFilter(ACTION_BATTERY_CHANGED))` once at each ready session, then a dynamic receiver while the session is ready and the switch is on, unregistered otherwise (**no polling, no timer; the system broadcasts it**). `level = EXTRA_LEVEL * 100 / EXTRA_SCALE` rounded; `charging = EXTRA_PLUGGED != 0`; `saver = PowerManager.isPowerSaveMode`, updated through the `ACTION_POWER_SAVE_MODE_CHANGED` broadcast on the same receiver. **[M]** both actions deliver to a dynamically registered receiver; the broadcast fires on each 1 % change, which `PresenceSender` collapses with the 2-point/30 s rule.
- **Showing the PC's presence.** The persistent notification text becomes `Connected to Desk PC · 82% · charging` (battery only if the PC sent one and `presence.show` is on) and `… · Kin paused on the PC until 14:30` / `… · paused on the PC`. **Updates are throttled** by the existing `TrailingThrottle` (at most one per 30 s; pause changes at once). Home shows the same words. Nothing is stored.
- **Low-battery nudge for the PC (2B.5)** uses `LowBatteryNudge`; 2B.4 only builds and tests the machine.
- **Battery:** no wake lock, no alarm, no timer; two dynamic receivers and a few hundred bytes per update on the open WebSocket.

### 5.3 Pause on the phone

- **Tile:** `PauseTile` (`TileService`, `BIND_QUICK_SETTINGS_TILE`, the pattern of `SendClipboardTile`). **Tap** with Kin running: if not paused → **pause 1 hour**; if paused → **resume**. **Long-press** opens `PauseSettingsActivity` (`ACTION_QS_TILE_PREFERENCES`) with the three choices and Resume. State `STATE_ACTIVE` while paused, subtitle "Until 14:30" or "Until resumed"; `STATE_UNAVAILABLE` and subtitle "Not paired" with no PC. **`isLocked` → `unlockAndRun`** (a thief's tap on a lock-screen tile does nothing). The tile listens to `PauseStore` (`requestListeningState`), so it follows a pause set elsewhere.
- **Adding the tile:** Settings has a row "Add the Pause tile" using `StatusBarManager.requestAddTileService` on API 33+ **[V in the stubs, M for behaviour]**; below that, one line of text "Swipe down twice, edit, drag Pause Kin in".
- **Notification action:** the persistent notification gets **"Pause 1 h"** (or **"Resume"** while paused) as its second action (a broadcast to a not-exported `PauseReceiver`). Total actions stay at two.
- **Home** shows a **Pause** card (the choices) and, while paused, "Paused until 14:30 · 3 received while paused · Resume".
- **Gates:** `PauseGate.paused()` is read by `MirrorEngine` (post only), `MediaSender`, the clipboard receiver, `ShotWatcher`, `LowBatteryNudge` and **the quiet-arrival path** of 3.5: in `Notifications.textFrom`/`linkFrom`/arrival and `TextOutbox`'s receive side (**no clipboard write**, no `LinkOpen`). Expiry is a wall-clock check on each use. **No component is stopped or started** by pausing, and **no alarm is scheduled** to resume.
- **After a reboot or process death** the state is read from DataStore at service start; the peer is told in the first `presence`.

### 5.4 Ring my phone

**Where it lives.** `RingController` is created by `AppGraph`, driven by `IncomingHandler` and `RingActionReceiver`, and runs inside the existing `LinkService` process. **No new foreground-service type**, no `FOREGROUND_SERVICE_MEDIA_PLAYBACK` (Kin plays no media; same reasoning as 2B.3 section 4.5). **[M]** Playing audio from a `connectedDevice` foreground service is allowed.

**Audio.**
- `MediaPlayer` with `AudioAttributes(USAGE_ALARM, CONTENT_TYPE_SONIFICATION)`, `isLooping = true`, `setVolume(1f, 1f)`; source = `RingtoneManager.getDefaultUri(TYPE_ALARM)`, else `TYPE_RINGTONE`, else a small bundled tone (`res/raw/kin_ring`, an Ogg of a few KB). **`STREAM_ALARM` is independent of the ringer mode**, so a phone on silent or vibrate rings. **[D]** (AOSP audio attributes), **[U]** per OEM.
- **Volume.** The user's alarm volume is often low. Before starting, **save** `getStreamVolume(STREAM_ALARM)` to DataStore (`ring_saved_alarm_index`) **and only then** raise it to the maximum (setting "Ring volume": *Loud (maximum)*, default; *Use my alarm volume*); **restore on every stop**, and **at every `LinkService` start if the key is still set and nothing is ringing** (a crash mid-ring must not leave the alarm volume at maximum: R40). Not done when `AudioManager.isVolumeFixed()`. No `FLAG_SHOW_UI`. **[M]** raising `STREAM_ALARM` needs no permission; the DND `SecurityException` of 2B.3 4.4 concerns volume changes that would toggle Do Not Disturb, which raising does not.
- **No audio focus request**: Android 15 only grants focus to the top app or an audio-related foreground service **[D: Android 15 behavior changes, secondary]**, and an alarm plays without it.
- **Vibration:** `Vibrator` waveform (800 ms on, 400 ms off, repeating) with `AudioAttributes(USAGE_ALARM)`; `VIBRATE` is a normal permission (manifest line).
- **Do Not Disturb:** `NotificationManager.getCurrentInterruptionFilter()` (**no permission** [M]) `== INTERRUPTION_FILTER_NONE` ("total silence") → Android plays nothing: the controller says `audible: false` and still shows the notification. **Priority only and Alarms only both allow alarms** by default **[D/U]**. Kin never changes the filter for this.
- **Wake lock:** one `PARTIAL_WAKE_LOCK` with a 35 s timeout while ringing, released on stop; nothing when idle.

**The Stop screen.**
- Notification channel **`kin_ring`** (`IMPORTANCE_HIGH`, no sound and no vibration of its own: the player does it; created once), `CATEGORY_ALARM`, `setOngoing(true)`, a **Stop** action (`PendingIntent` to the not-exported `RingActionReceiver`), `setFullScreenIntent(RingActivity, highPriority = true)`, `VISIBILITY_PUBLIC` (a stop button must be reachable on the lock screen; the text says only "<PC> is ringing your phone").
- `RingActivity`: not exported, `showWhenLocked`, `turnScreenOn`, `taskAffinity=""`, `excludeFromRecents`, keeps the screen on while ringing, one huge **Stop** button, the PC name and a countdown; it finishes itself on stop.
- **Android 14+ restriction (verified above): `USE_FULL_SCREEN_INTENT` is a normal manifest permission, granted at install for sideloaded apps and revoked by Google Play for apps that aren't calling or alarm apps.** Kin declares it. `NotificationManager.canUseFullScreenIntent()` false → **the notification still posts, as a heads-up with its Stop button for 60 s**, and Settings shows a row "Show the Stop screen on a locked phone" that opens `Settings.ACTION_MANAGE_APP_USE_FULL_SCREEN_INTENT`. **Ringing never depends on it**: the sound is the service's; Stop is reachable from the heads-up, the notification, the app, and the PC. Which of these applies on a given phone is a real-phone test (R37). Ringing over Do Not Disturb filters **[U]**: whether `SUPPRESSED_EFFECT_FULL_SCREEN_INTENT` hides the screen.
- **Permission `POST_NOTIFICATIONS` denied:** the ring still sounds, `ring.status` says `audible: true`, the PC is told "the phone can't show the Stop screen".
- **The phone never starts an activity from the background by itself** (Android blocks it); the full-screen intent is the exempt route.

**State machine** (`ring[]` vectors): `idle → ringing (30 s timer) → idle`; `ring.start` while ringing restarts the 30 s but **the continuous total is capped at 2 minutes** (then `limit`, stop, a 60 s lock-out answering `rate_limited`); 3 starts per 60 s; the timer is a coroutine `delay` (the service is alive), and a service that dies takes the player with it.

**Debug build:** the ring can run with **`RING_SILENT`**: no `MediaPlayer`, no `Vibrator`, **the `STREAM_ALARM` volume is not touched** (the save/raise/restore logic is exercised on a **fake `AudioManager`** in unit tests, and with real values by `RING_VOLUME_TEST` on the emulator, which has no sound). The headless emulator already runs with `-no-audio` (`tools/emulator/start.ps1`); the real `MediaPlayer` may be run there, never on a emulator started with `-Window` while the user may be present. A real sound is the user's manual test.

### 5.5 Permissions and manifest (the only changes of 2B.4)

| Entry | Why |
|---|---|
| `USE_FULL_SCREEN_INTENT` | The Stop screen on a locked phone (5.4). Normal permission, no prompt; Play may revoke it (R37) |
| `VIBRATE` | Ring vibration. Normal permission |
| `READ_MEDIA_IMAGES` (33+), `READ_EXTERNAL_STORAGE` with `android:maxSdkVersion="32"` | **Asked at runtime only when the user turns auto-send on** (5.6). Declared in the manifest, never requested at start |
| **Not** `READ_MEDIA_VISUAL_USER_SELECTED`, **not** `ACCESS_MEDIA_LOCATION`, **not** `READ_MEDIA_VIDEO`, **not** `MANAGE_EXTERNAL_STORAGE` | See 5.6 and T8 |

`docs/android.md` gets "Presence, Pause, Ring, Screenshots" sections and the permission rows. Services: `PauseTile` (declared like `SendClipboardTile`), activities `RingActivity` and `PauseSettingsActivity`, receivers `RingActionReceiver` and `PauseReceiver` (not exported).

### 5.6 Auto-send screenshots (opt-in)

**Turning it on** (Settings → "Send screenshots to your PC", with a second switch "Also send new camera photos"): (1) the screen says what is read ("Kin watches for new screenshots; it sends only ones taken after you turn this on, to your PC only"); (2) it asks for the permission **now** (never at start-up): `READ_MEDIA_IMAGES` on 33+, `READ_EXTERNAL_STORAGE` on 29 to 32; (3) it **takes a baseline** (the highest `_ID`/`date_added` in `MediaStore.Images` at this moment) and stores only that number; (4) it refuses if the PC's `hello` doesn't list `presence` (3.10).

**Android 14 partial photo access.** Kin does **not** declare `READ_MEDIA_VISUAL_USER_SELECTED`. Per the documentation, the system then runs Kin in compatibility mode: "Select photos" gives a **session-only** grant, **which does not work for a background watcher**. So after the request, `checkSelfPermission(READ_MEDIA_IMAGES)` is the truth, **evaluated at the point of use and never cached in DataStore** (the page warns against that); a grant that is partial or revoked puts the switch in a **"Needs access to all photos"** state with a button to the app's settings, and the observer is not registered. The Home card says so too. **[U]** exactly what the permission dialog looks like per Android version and OEM.

**Detection.** `ShotWatcher` registers a `ContentObserver` on `MediaStore.Images.Media.EXTERNAL_CONTENT_URI` (notifyForDescendants = true) **only while** the switch is on, the permission is held, a PC is paired and `LinkService` is running; unregistered otherwise (**no polling, no timer**). `onChange(selfChange, uris, flags)` (API 30+) hands URIs; below that it is a bare signal and Kin queries "newer than the baseline". Each event is **coalesced 500 ms** and handled by one coroutine. `ShotPicker` (pure, `shot[]` vectors) decides per row, **querying only** `_ID`, `RELATIVE_PATH`, `MIME_TYPE`, `IS_PENDING`, `DATE_ADDED`, `SIZE`, `OWNER_PACKAGE_NAME` (never `DATA`, EXIF, location or thumbnails):

| # | Rule | Skip reason |
|---|---|---|
| 1 | `_ID` ≤ the baseline, or already handled (last 50 IDs in memory plus the persisted high-water mark) | `old` |
| 2 | `IS_PENDING = 1` (the observer fires again when it clears) | `pending` |
| 3 | `MIME_TYPE` not `image/*`, or `SIZE` = 0 | `not_image` |
| 4 | `DATE_ADDED` older than **15 s** before now (a gallery re-index or a restore, not a screenshot) | `stale` |
| 5 | `RELATIVE_PATH` starts with Kin's own folders (`Pictures/Kin/`, `Download/Kin/`): **received files must never be sent back** | `own` |
| 6 | **Screenshot** if `RELATIVE_PATH` contains `screenshot` (case-insensitive: `Pictures/Screenshots/`, `DCIM/Screenshots/`, `Pictures/Screenshot/` on some OEMs), **or** `OWNER_PACKAGE_NAME` is `com.android.systemui` (stock screenshots) → candidate `screenshot`, only if `shots.screenshots` is on | `off` |
| 7 | **Camera** if `RELATIVE_PATH` starts with `DCIM/Camera` → candidate `camera`, only if `shots.camera` is on | `off` |
| 8 | Anything else (downloads, WhatsApp images, memes, edited copies) is **never** sent | `other` |
| 9 | Paused (3.5) → skipped **and the baseline moves up to this `_ID`** | `paused` |
| 10 | Rate: bucket capacity 5, refill 1 per 20 s, over → dropped and counted | `rate` |

**The hold.** A candidate shows a **silent, low-importance notification** "Sending screenshot to Desk PC in 5 s · **Cancel**" (channel *File transfers*, one tag per `_ID`). Cancel drops it (the baseline still moves: it will not come back). After 5 s it is uploaded through `TransferEngine.enqueueUpload` for that one URI with `origin`. **If no ready connection exists at that moment, or the `POST` gets no answer in 5 s, it is dropped and counted** (not queued; the file is still in the gallery and the Share sheet works). The notification then shows the result for a moment ("Sent" / "Not sent: PC away") and goes. The upload reads the file through `ContentResolver.openFileDescriptor` on an **unstable client** like every shared file (android.md R5), **without `ACCESS_MEDIA_LOCATION` so Android redacts GPS from the EXIF of a camera photo it hands over [M]**.

**Not covered here and said so in the Settings text:** a screenshot of a `FLAG_SECURE` screen can't exist; apps that save screenshots elsewhere; Samsung's "Smart capture" edits (a second file may follow: `old`/`other` rules handle it).

### 5.7 Unlinking (phone)

- Rename **"Unpair" → "Unlink"** on Home, Settings and the `PairAgain` card ("Remove this PC" stays as is). Confirmation: **"Unlink from Desk PC? This phone and the PC stop working together right away: no sending, notifications or ringing. Your history stays on this phone. You can pair again with a new code."** (`HomeScreen.kt`, `SettingsScreen.kt`).
- `LinkController.unpair` is unchanged in behaviour (**[V]** it removes the row first, sends `unpair` if connected, stops the service). New: it also **clears `pause` state of this peer, the peer's overrides, the saved baseline and any ring state**, releases a held DND (2B.5), and **restores the alarm volume** if a ring was active.
- **Durability:** Room's `devices().delete`/insert must survive a power cut. The coder reads `PRAGMA synchronous` on the two images (**[M]** Android's WAL default is `NORMAL` on some builds) and, if it is not `FULL`, calls `wal_checkpoint(FULL)` (or sets `synchronous=FULL` for this write) after pairing and after unpairing. Record the value in the results.
- **`onPeerUnpaired`** (the PC removed this phone) already deletes the row and stops the service **[V]**; the `PairAgain` state after a `401` stays, with "Remove this PC" as the one button.

### 5.8 What the phone stores and logs

Stores: DataStore keys `kin_pause` (2), `kin_features` (1 JSON string, empty), `presence_*`, `ring_*`, `shots_*` booleans and the **baseline number**, `ring_saved_alarm_index` (only while ringing). **No battery level, no pause state of the peer, no screenshot name, path or image.** Logs: **counts only**: `presence: sent N, received N, dropped {rate: N, offline: N}`; `ring: started N, stopped {timeout: N, user: N, pc: N}`; `shots: seen N, sent N, skipped {old: N, own: N, other: N, paused: N, rate: N, offline: N}`; setting changes. **Never** a level, a path, a file name, a `RELATIVE_PATH` or the PC's pause time. A test greps a run's log for them.

### 5.9 Debug build (test hygiene, section 7)

`DebugCommandReceiver` gets (each prints a `CMD …` line): `PAUSE_SET --es mode off|1h|tomorrow|resume [--el untilMs N]`; `PRESENCE_STATE` (counts, the PC's last battery and pause, `sent`, `dropped`; **no content**); `PRESENCE_SET --ez share true|false --ez show true|false`; `RING_SET --ez allow true|false`; `RING_STATE` (`state`, `audible`, `count`, `lastReason`, `savedAlarmIndex`, `canFullScreen`); `RING_STOP_LOCAL`; `SHOTS_SET --ez screenshots … --ez camera …`; `SHOTS_STATE` (baseline, counts, skip reasons, **no names**); `SHOTS_INSERT --es kind screenshot|camera|other [--ei ageSec N]` (inserts a **generated** PNG into the emulator's MediaStore at the matching `RELATIVE_PATH`, so no screen capture is needed); `BATTERY_*` is done by the shell (`adb shell dumpsys battery set level 14`, `set ac 0`, `reset`: it changes the **emulator's** battery only).

## 6. Windows

### 6.1 Pieces (new, desktop coder; `presence/`, `ring/`; names are suggestions)

| Piece | Job |
|---|---|
| `presence/wire.rs`, `limits.rs`, `pause.rs`, `nudge.rs` | Pure: validation (vectors), the sender and receiver rules of 3.4, the `PauseChoice → until` function (`pauseUntil[]`; uses the local zone through a trait so tests pass a zone), the gate of 3.5, the 3.6 machine |
| `presence/battery.rs` | A trait `PowerSource` with the real one (`GetSystemPowerStatus`, a **60 s poll only while a phone is connected, `presence.share` is on and the PC has a battery**: a PC with `BatteryFlag & 128` never polls) and a fake. **Not event driven in 2B.4** (`RegisterPowerSettingNotification` needs a window and can't be tried on this PC, which has no battery: **[U]**; a later improvement) |
| `presence/mod.rs` | The core's view: settings, snapshot per connection, last known battery, counts; feeds the tray and the drop window |
| `ring/mod.rs` | Client state (`idle`, `requested`, `ringing`, `stopped`), the 5 s ack timer, the 1-per-10 s limit, the texts of 3.8 |
| `features.rs` | The resolver of section 4 |

`ws.rs` sends `presence` after each accepted `hello` and on change, `core.rs` holds the state. **Only the presence and ring modules write `presence`/`ring.*`** and they check the peer's capability per message.

### 6.2 Tray, popup, tooltip

- **Tooltip** (≤ 127 characters, `tooltip_with` gains the numbers): `Kin: Pixel 8 connected · 62% charging`, plus ` · paused until 14:30` / ` · paused` when the **PC** is paused and ` · phone paused` when the **phone** is. An offline phone: `Kin: Pixel 8 offline, last seen 14:32 · 12%` (the last known level from memory).
- **Status line** (first, greyed menu line): the same words (`status_line_with`).
- **Icon:** a **grey** dot while this PC is paused (the connected/offline dots otherwise), so the state is visible without hovering.
- **Menu** gains, between "Send clipboard to phone" and "Pair new phone": **"Pause Kin ▸"** (a submenu: *1 hour*, *Until tomorrow*, *Until I resume*; replaced by **"Resume Kin"** while paused), **"Ring my phone"** (becomes **"Stop ringing phone"** while ringing; disabled with the reason as a tooltip text in the status line when the capability is missing or the phone isn't connected), and **"Unlink phone…"** (6.5). `tray.rs` uses `tauri::menu::Submenu`.
- **Drop window** (left click): a "Phone" strip: name, battery with a charging mark, "paused until 14:30" chips for both devices, **Ring my phone** and **Pause** buttons. Built from a `presence` view in the same `Snapshot` the tray uses; nothing runs while the window is closed (windows.md process model).
- **Low-battery toast** (3.6): a Phase 1 plugin toast "Pixel 8 is at 14% and not charging", no buttons, honouring `presence.nudge`, suppressed while paused and by `quiet::UserState::holds_back_windows()` (a game, a presentation) so it never steals a game's focus.
- **Command line (single-instance hand-off, like `--send-clipboard`):** `Kin.exe --pause 1h|tomorrow|resume`, `--ring`, `--ring-stop`. They give a script (and the integration agent) a way to act **without any input injection**. Results go to `status.json` (`pause`, `ring.state`, counts) and a toast.

### 6.3 Pause (PC)

State in `settings.json`: `"pause": {"until": 1760000000000}` or `{"untilResumed": true}` or absent. The gate `paused()` is read by: the automatic clipboard sender, `mirror/` (drop `notif.post`, keep `remove/sync`), `media/` (2B.3: act as `show`/`share` off), the shot/arrival code, the nudge, and the **quiet-arrival path**: `arrival.rs`/`phonelinks.rs`/`app.rs` handling of received `text`, `link` and files: **history row yes; toast, clipboard write, `ShellExecute` of a link, image-to-clipboard no** (`phonelinks::decide` gets a `Paused` outcome next to its existing "offered in a toast" ones; one pure function, tested). **Nothing is stopped or started** by pausing and there is no resume timer: `paused()` compares wall-clock time on each use. The extension's `status` carries `paused`.

### 6.4 Ring (PC)

`ring/mod.rs`: **Ring my phone** → needs a connected phone whose `hello` lists `ring` → sends `ring.start` → waits 5 s for the `ack` → state `requested`; on `ring.status ringing` → `ringing` ("Pixel 8 is ringing" in the tray text and the drop window, **Stop ringing**); on `ring.status stopped` or the 35 s local watchdog → `idle`. Texts: `error disabled` → "Ringing is turned off on Pixel 8"; `rate_limited` → "Wait a minute, then try again"; no `ack` → "No answer from Pixel 8. It may be asleep or off Wi-Fi."; no connection → "Pixel 8 isn't connected, so Kin can't ring it."; `audible: false` → "Ringing, but Do Not Disturb is on total silence: it won't make a sound". **The PC makes no sound.**

### 6.5 Unlinking (PC)

- **Rename "Unpair" → "Unlink"** in the Settings window (`settings.ts`) and add the tray item **"Unlink phone…"** (it opens the Settings window with the confirmation already open for the headline phone, `#unlink=<deviceId>`). Confirmation text: **"Unlink Pixel 8? This phone stops working with Kin right away: nothing more can be sent, shown or rung, even if it is offline. You can pair it again later with a new code."**, plus when the phone is offline "It will find out the next time it connects."
- **A "Lost your phone?" line** in the device row: "Unlink it here. It stops working at once, even if it is offline. If it is connected you can ring it first." with a **Ring** button.
- **`core.remove_device`** (the spec): (1) **durable removal** — in `storage.rs` the writes to `devices` (`upsert_device`, `remove_device`) run through a helper that sets `PRAGMA synchronous = FULL` on the connection for that transaction and back to `NORMAL` after (or `PRAGMA wal_checkpoint(FULL)` after the commit); a unit test asserts the pragma is `FULL` inside the helper and `NORMAL` after, and that a removal is visible to a **second connection** to the same file immediately; (2) the existing steps (pending cancels, mirror, `connections.revoke`); (3) **new: abort the device's in-flight transfers**: `transfers.abort_device(id)` stops the writers and readers of its transfers, marks them `failed` with `peer_removed`, drops their `.part` files by the existing rule (only paths a record names), cancels its queued items as `peer_changed`; (4) **new: forget its in-memory presence, pause view, ring state, and delete its `featureOverrides`**.
- **Tests (desktop):** (a) an HTTP `PUT` in flight when `remove_device` is called ends and writes no more bytes; (b) **an already-open TLS connection of the removed device gets `401` on its next request** (per-request authorization, **[M]** from protocol.md "Authorization happens per request": this test proves it); (c) the WebSocket is closed (`unpair` or 4003); (d) after `remove_device` a **fresh** connection with the old key is `401`; (e) the removal survives re-opening the database file.

### 6.6 Auto-sent images (PC side)

`arrival.rs` (the received-file path) reads `origin` from the transfer record (a new nullable column `origin`, added with the repo's `ensure_column` pattern) and applies the table of 3.7: the toast wording, **the clipboard write with the sensitive formats for `screenshot`**, no clipboard for `camera`. The existing "exactly one image in a transfer" rule stays. The clipboard write goes through the same sensitive writer as the 2B.2 code copy (`mirror/otp_clip.rs`'s formats), so **Kin's own automatic clipboard sync does not send it back to the phone** (it is text only, but the marking is also what keeps Win+V out). `images.rs` is unchanged.

### 6.7 Settings window and `status.json`

Settings gets: **Phone** section (the device row with **Unlink**, "Lost your phone?", the phone's battery and pause line), **Pause** (the three choices and Resume), **Presence** ("Show my phone's battery here", "Share this PC's battery with the phone" (disabled with "No battery" on a desktop), "Warn me when the phone's battery is low"), **Ring** (a Ring button). `settings.json`: `presenceShareBattery`, `presenceShowPhone`, `lowBatteryNudge`, `pause`, `featureOverrides`. `status.json` gains `pause: {on, untilResumed}`, `presence: {phone: {level, charging, paused}, counts}` (numbers only), `ring: {state, counts}`.

### 6.8 What the PC stores and logs

Stores: the settings above. **No battery level of the phone, no database row.** Logs: counts and enums (`presence: sent N, received N`, `ring: requested, ringing, stopped(timeout)`, `pause: on until resumed`). **Never** a battery level or a screenshot name.

## 7. Safety for tests: nothing audible, nothing of the user's

- **Dev instances never touch the installed Kin** (D17 / `kin-dev.ps1`): all the new commands run against a dev Kin with its own `--data-dir` beside the installed one. The new CLI hand-offs (`--pause`, `--ring`, `--ring-stop`) must go only to the instance of the **same program file and data dir** (the existing "never hand a command to a Kin of another program file" rule).
- **Fake battery:** `--dev-battery <level> <charging|unplugged>|none` replaces `PowerSource` (the dev PC has no battery anyway). The real power API is read-only and safe.
- **No sound anywhere:** the PC never makes one; the phone-side ring on the emulator uses `RING_SILENT` or runs on a `-no-audio` headless emulator. **Nobody calls `adb shell cmd media_session` or changes the PC's volume.**
- **Phone settings are the emulator's:** `dumpsys battery set …` and `cmd notification set_dnd` change the **emulator** only, and `dumpsys battery reset` ends the test.
- **No input injection on the PC.** A screenshot on the emulator is inserted by `SHOTS_INSERT` (or `adb shell screencap` plus a media scan), not by key events on the PC.
- **Clipboard rule (plan rule 4):** the screenshot e2e writes to the PC clipboard: **save in all formats first and restore afterwards** (`Save-Clipboard`, with the earlier-backup rule), and mark any test text (`Set-QuietClipboardText` convention).
- **Game check** before anything that could pop up a window (the low-battery toast): skip and note if a game or EasyAntiCheat is running; `--dev-toast-quiet` stays on.

## 8. Privacy and security

| # | Threat | Mitigation | Residual |
|---|---|---|---|
| T1 | Battery level or pause state ends up in logs, `status.json` beyond numbers, the database or the debug export | Never stored (memory only), counts and enums in logs, tests grep | None known |
| T2 | **A screenshot with a password, a code or a balance is sent and lands on the PC clipboard, in Win+V history, or on a shared screen** | Off by default and per switch; only screenshots **taken after turning it on** (baseline); **5-second hold with Cancel**; Pause tile; the clipboard write uses the sensitive formats so Win+V and cloud clipboard keep nothing; toast shows the image only when the layer can, never the file path; rate bucket | The PC clipboard holds the image until overwritten; the file stays in `Downloads\Kin`. Documented in the switch's text (R39) |
| T3 | Auto-send reads more photos than needed | Declared `READ_MEDIA_IMAGES` only, asked only when turned on; the picker queries only the columns of 5.6 and sends only rows matching a rule; **no thumbnails, no EXIF read by Kin, no location (`ACCESS_MEDIA_LOCATION` absent)**; videos never; Kin's own folders excluded; nothing leaves the paired PC | The permission is broad by nature: it lets Kin read **every** image. Kin's discipline is the only limit |
| T4 | A lost phone keeps working | Unlinking on the PC is **durable**, closes the WebSocket, **aborts in-flight transfers**, and the next request of the old key is `401` (tests 6.5) | If the thief's phone is unlocked and Kin holds a pairing, and the PC user hasn't unlinked yet, the phone can use the link: unlink promptly. Documented ("Lost your phone?") |
| T5 | Ring is used to annoy (a compromised PC, a same-user program) | Phone switch (default on, so it works for a lost phone), 3 per 60 s, 2-minute cap with a lock-out, always a visible Stop; the PC side is not on the browser-extension pipe, so a web page or extension can't reach it, only a same-user process (the existing `--send-clipboard` trust level) | A paired PC can ring the phone. Equal to the trust of pairing |
| T6 | Ring leaves the alarm volume at maximum | Saved before raised, restored on every stop and at every service start (crash case) | A phone that is killed and never restarted keeps the raised volume until Kin runs again (R40) |
| T7 | A peer ignores the pause, or a race delivers an automatic message after it | **The receiver gates too** (3.5), so nothing visible happens; cleanup still flows | A paused phone still keeps its connection (visible to the PC) |
| T8 | Full-screen intent abuse | One use, ring only; the notification text says only "<PC> is ringing your phone"; `VISIBILITY_PUBLIC` is for the Stop button only | A lock screen shows that Kin is ringing |
| T9 | Quick settings tile on a lock screen | `isLocked → unlockAndRun` | None |
| T10 | `origin` or `presence` abused by a hostile paired phone | `origin` changes only wording and a clipboard rule; `presence` is validated and bounded (4 KiB, integers, no text field at all) | A paired phone can show any battery level |
| T11 | Battery and charging patterns reveal when the user is home | Shared only with the one paired PC, switch on both sides, memory only | A compromised PC could keep a history; the PC does not (T1) |

**Unit-testable invariants:** a `presence` with a 100 KB field or a string level is rejected; a paused receiver changes no clipboard, no toast, no link on any message type; a baseline is monotonic; no code path logs a level or a path (a logger that fails on test strings).

## 9. New risks for `docs/risks.md` (the reviewer adds them at 2B.6; 2B.3 used R28–R36)

| # | Risk | Likelihood | Impact | What we do about it | When we find out |
|---|---|---|---|---|---|
| R18 | **Play Store photo-permission policy** (reserved by the plan, filled here): Play restricts `READ_MEDIA_IMAGES`/`READ_MEDIA_VIDEO` to apps whose **core** function needs broad, persistent access; the picker is expected otherwise. Auto-send screenshots is arguably core to *that feature*, not to Kin | Medium | Medium | The permission is asked only when the switch is turned on and the declaration form says why; if Play refuses, **the feature ships in the sideload/GitHub build only** and the Play build omits the permission and the switch (one build flag) | Phase 2 Play submission |
| R37 | Full-screen intents are limited to calling and alarm apps on Android 14+; Play revokes the permission, so the Stop screen may not appear on a locked phone (and Do Not Disturb filters may hide it) | High (Play build) | Low | Ring never depends on it: sound is the service's; heads-up Stop for 60 s, notification Stop, the app, and the PC; Settings row to grant it | Real phone, Play build |
| R38 | The v0.1.1 `synchronous = NORMAL` can undo a device removal or a new pairing after a power cut (a "removed" phone works again) | Low | High | Durable writes for `devices` (6.5); a test; Android checks `PRAGMA synchronous` | Unit test; a real power cut is not testable |
| R39 | Auto-sent screenshots expose sensitive content (a password, a code, a balance, a private chat) to the PC, its clipboard and a shared screen | Medium | High | 5 s hold with Cancel, Pause tile, baseline, sensitive clipboard formats, off by default, honest text; R18 limits the Play build | Real use; TESTING.md |
| R40 | Ring edge cases: a crash leaves the alarm volume at maximum; "total silence" makes it inaudible; Doze or an OEM kills the socket so `ring.start` never arrives; Bluetooth headphones take the alarm route | Medium | Medium | Save/restore + restore at start; `audible: false` reported; the 5-second "no answer" text; 2A's FCM wake-up is the real fix for the socket; real-phone matrix | Real phone |
| R41 | A silent link death leaves a stale battery or "paused" on the peer for up to ~4 minutes (the 240 s WebSocket ping, as R30) | Medium | Low | Cleared on close; pause expiry is local; the battery is labelled live only while connected | Real phone, Wi-Fi off |
| R42 | Pause state diverges (a lost `presence`, a clock set wrongly, a peer without `presence`) so one side keeps sending, or a quiet arrival hides something the user wanted | Medium | Low | Receiver gates too; snapshot after every `hello`; a bad clock treated as expired after 8 days; Home shows "N received while paused" | Real use |

## 10. Test plan (lean)

Follows the user's lean-testing rule: unit tests once, an emulator e2e for what changed, no full regression.

### 10.1 Unit tests

**Android (JVM, no Robolectric):** `PresenceWire` (`presence[]`), `PresenceSender` with a fake clock (`presenceRate[]`: 2 points, 30 s, pause in 1 s, crossings, snapshot after hello, capability per message, nothing when offline), `LowBatteryNudge` (`lowBattery[]`), `PauseChoice` (`pauseUntil[]` with real `java.time` zones), `PauseGate` (`pauseGate[]`, `pauseExpiry[]`, the 8-day guard), `RingController` with fake player, vibrator, audio manager, DND and clock (`ring[]`: 30 s, restart, 3 per 60 s, the 2-minute cap and lock-out, save-then-raise-then-restore order, restore at start after a crash, `disabled`, idempotent stop, `audible` false under total silence), `ShotPicker` (`shot[]`: every rule and the order, baseline, pending, own folder, stale, paused advances the baseline), `Features` (`features[]`, a grep test that no feature reads its flat key directly), `batteryMap[]`, the message parsing against `messages.json`, the log grep, the **quiet arrival** path (a paused phone: no clipboard write, no link open, history row, counter).

**Desktop (`cargo test`):** `presence/wire` (vectors), `batteryMap[]` with the fake power source (`flag 128` never polls), `pause` (`pauseUntil[]` with explicit zones, DST), the gate over every row of the 3.5 table (`pauseGate[]`), `nudge` (`lowBattery[]`), the sender/receiver limits (`presenceRate[]`), `ring` client state with a fake clock, `origin[]` (toast wording, clipboard rule, sensitive formats on the fake clipboard), `phonelinks::decide` with `Paused`, `remove_device` durability and abort tests (6.5 a to e), `features[]`, tray text builders (`tooltip_with` with battery, paused, offline last level, 127-character cap), settings defaults, the log grep.

**Vectors:** 3.11; `bun protocol/check-vectors.ts` stays green.

### 10.2 Emulator e2e (short; the integration agent writes `tools/e2e-2b-4.ps1`)

`kin_api37` (and `kin_api29` for the storage permission and tile differences), the real desktop build as a dev Kin beside the installed one (`--dev-bind 127.0.0.1`, throwaway data dir, `--dev-toast-quiet`, `--dev-battery`), the clipboard saved and restored. **No sound, no input on the PC.**

1. **Presence:** `adb shell dumpsys battery set level 14` and `set ac 0` → the PC's `status.json` shows the phone at 14, and the **low-battery toast** is requested once (via `--dev-toast-quiet`, history); charging then 80 % re-arms; `--dev-battery 9 unplugged` on the PC → `PRESENCE_STATE` shows it and the phone's notification text carries it (`dumpsys notification`).
2. **Pause, both directions:** `Kin.exe --pause 1h` → `PRESENCE_STATE` shows "paused" within 2 s; the phone sends no automatic `clipboard`/`notif.post`; **a manual `text` from the PC (fake_phone) arrives quietly**: history row, **no notification**, **clipboard unchanged**; `PAUSE_SET mode=1h` on the phone → the PC's `status.json` and tooltip say paused; a manual link from the phone is saved in the PC history with **no browser opened** (`--dev-no-open` proves it) and no toast; resume → everything flows again; `until tomorrow` and `untilResumed` round-trip; **a `pausedUntil` in the past after `--dev-clock-offset`** resumes.
3. **Ring (silent):** `RING_SET allow=true`; `Kin.exe --ring` → `RING_STATE` = ringing, `audible`, `ring.status` on the PC; the notification is `CATEGORY_ALARM` with a Stop action and (screen off via `KEYCODE_SLEEP`) the activity is `RingActivity` if `canFullScreen` (`appops set … USE_FULL_SCREEN_INTENT deny|allow` toggles it: record both); 30 s later it stops with `timeout` and the **alarm volume equals its start value**; `--ring-stop` stops with `pc`; `RING_SET allow=false` → the PC's `ring` capability is gone and `--ring` says why; `adb shell cmd notification set_dnd none` (emulator only) → `audible:false`, then `set_dnd off`; 4 starts in a minute → `rate_limited`. **No `MediaPlayer` on a windowed emulator.**
4. **Screenshots:** `SHOTS_SET screenshots=true` with the permission granted by `pm grant`; `SHOTS_INSERT kind=screenshot` → the "Sending in 5 s" notification, then the PC gets a file with `origin=screenshot`, **the clipboard holds the image with the three sensitive formats** (enumerate formats on the dev Kin's clipboard with the existing helper), the toast says "Screenshot"; `kind=other` and `camera` (camera off) → nothing; `camera` on → a file, **no clipboard**; a file in `Pictures/Kin` → nothing; `ageSec=60` → nothing (`stale`); a Cancel (`adb shell cmd notification` action or the debug command) → nothing and the baseline moves; PC stopped → **dropped, not queued** (`SHOTS_STATE` counts `offline`); paused → nothing, and after resume the old one does not appear; permission revoked (`pm revoke`) → "Needs access", observer unregistered.
5. **Unlink and the lost phone:** start an upload of a large generated file from the phone, call `--dev-set` / the dev hook for `remove_device` on the PC → the upload **stops**, the WebSocket closes, the phone shows the `PairAgain` card; a new connection with the old key is `401`; re-open the PC database → the device stays removed; Unlink from the phone → the PC row disappears and `featureOverrides` are cleared.
6. **Old peers:** `fake_phone` without `presence` → no `presence` sent to it, "Ring" disabled with the text, the paused PC still drops its automatic messages.
7. **Logs and storage:** grep both logs, the data dirs and `status.json` for the test levels, file names and paths: none.
8. **Cleanup:** `dumpsys battery reset`, `set_dnd off`, `appops reset`, permissions revoked, the dev Kin stopped, the clipboard restored.

### 10.3 What can't be tested without a person or hardware

The **audible ring**: loudness, the system alarm tone, vibration, **silent and vibrate modes, Priority-only and Alarms-only, total silence, Bluetooth headphones, a phone asleep in Doze for 30 minutes**; the **full-screen Stop** on real Pixel and Samsung phones, on a Play-installed build (R37), and over Do Not Disturb; the tile on the lock screen and its long-press; the **photo permission dialog** per Android version and OEM, including "Select photos" (5.6); where real phones put screenshots (Samsung, Xiaomi, Oppo) and camera photos; EXIF location in a real camera photo (is GPS redacted?); the **laptop battery** path and the real `GetSystemPowerStatus` values on a laptop on battery, charging, full, and while sleeping; the PC toast look and Win+V not keeping a screenshot; battery cost of the phone's receiver over a day; the unlink-after-a-real-power-cut case (R38). All of these go in `docs/TESTING.md` as the user's real-hardware checklist at v0.2.0.

## 11. Work split and first steps

**Desktop coder** owns: `protocol/` (vectors, `check-vectors.ts`), `docs/protocol.md` changelog 58 (section 3 copied in), the Windows code (6), `docs/windows.md`, `docs/extension.md` (`paused`), `examples/fake_phone.rs` (a `--presence`/`--ring` mode that plays the phone: sends `presence`, answers `ring.start`, prints what it gets), the dev switches (`--dev-battery`), the new CLI commands.

1. **S4a (first hour):** `remove_device` — write the failing tests of 6.5 (in-flight `PUT`, open TLS connection, durability) **before** changing anything, and record what fails today in `phase-2b-4-desktop.md`.
2. `protocol/presence.json`, the `messages.json` cases and the protocol.md section **as early as possible**: the Android coder runs them as soon as they exist.
3. `wire`, `limits`, `pause`, `nudge`, `features`, then the tray, the drop window, Settings, `origin` handling, ring, `status.json`.

**Android coder** owns: the manifest, the Android code (5), `Messages.kt`/`MessageReceiver.kt`, the debug tools (5.9), `docs/android.md`, `android/README.md` (the new `CMD`s). A spec problem goes in its results file; it doesn't edit `protocol/` or `docs/protocol.md`.

1. **Spike S4b (first hours, `kin_api37` then `kin_api29`; one agent on the emulator at a time):** (a) a `fullScreenIntent` notification from `LinkService` on a locked screen with `canUseFullScreenIntent` true and false (record both); (b) `STREAM_ALARM` save/raise/restore and a real `MediaPlayer` on the `-no-audio` emulator; (c) `ACTION_BATTERY_CHANGED` and `ACTION_POWER_SAVE_MODE_CHANGED` to a dynamic receiver; (d) a `ContentObserver` on `MediaStore.Images` with `SHOTS_INSERT` (what `RELATIVE_PATH`/`OWNER_PACKAGE_NAME` a generated insert has, and what a real `screencap` has); (e) `PRAGMA synchronous` on both images; (f) `StatusBarManager.requestAddTileService`; write the outcome in `phase-2b-4-android.md`.
2. Pure parts first (wire, sender, nudge, gate, pick, ring, features) against this document's tables until `protocol/presence.json` exists.
3. Presence and pause, then the tile, ring, screenshots, then the debug commands, so the integration agent can start.

**Integration agent** (after both): `tools/e2e-2b-4.ps1` and `docs/results/phase-2b-4-integration.md` per 10.2, with the emulator used by that one agent only.

**Results files:** `docs/results/phase-2b-4-{desktop,android,integration}.md`.

## 12. Open questions for the user

| # | Question | Recommendation |
|---|---|---|
| Q1 | **Screenshots wait 5 seconds on the phone with a Cancel button** before they are sent (plus the sensitive clipboard marking on the PC). Is a delay acceptable for a feature whose point is "instant"? Without the hold, a screenshot of a password or a bank balance is on the PC the moment it is taken | **Yes, 5 s.** It is the only privacy control that works at the moment of the screenshot, and 5 s is still faster than any manual route. Make the delay a constant; shorten it if you find it annoying |
| Q2 | If Google Play won't allow the photo permission (R18), is it OK that **auto-send screenshots exists only in the sideload/GitHub build** and the Play build has no such switch? | **Yes.** Everything else in 2B.4 needs no sensitive permission; a build flag keeps one codebase |
| Q3 | **Ring my phone ignores Pause** (it works even when Kin is paused) and is **on by default** on the phone. A lost phone is the reason for the feature, and you can't turn a switch on after losing it | **Both as written.** A user who doesn't want it can turn the switch off; Pause never silences a lost phone |
| Q4 | **"Until tomorrow" = the next 08:00** (at least 2 hours away). Do you want a different morning hour, or a setting for it? | **Fixed 08:00 for now**; one constant. Add a setting only if someone asks |
| Q5 | **Presence (battery sharing) is on by default** on both sides, unlike the notification and media features, which are off by default. It only shows a battery number to your own paired device | **On by default**, with a switch on each side. It is the cheapest, least sensitive data in the app, and it is what makes the tray tooltip and the low-battery warning work without setup |
| Q6 | **A camera photo never goes to the PC clipboard** (only screenshots do). Plan text: "also send new camera photos" | **Keep it that way**: a camera photo is large and personal, and the clipboard is the wrong place for it; it is saved in `Downloads\Kin` with a toast |

## 13. Sources

- Android: [Full-screen intent limits (Android 14)](https://source.android.com/docs/core/permissions/fsi-limits) (default only for calling and alarm apps, `canUseFullScreenIntent`, `ACTION_MANAGE_APP_USE_FULL_SCREEN_INTENT`, the 60 s heads-up when denied); [Partial photo and video access](https://developer.android.com/about/versions/14/changes/partial-photo-video-access) (`READ_MEDIA_VISUAL_USER_SELECTED`, the compatibility mode, "check at the point of use"); the Android 37 SDK stubs read with `javap` (`platforms/android-37.0/android.jar`). Secondary: Android 15 audio-focus restriction for background apps (behaviour-changes page, read through search).
- Windows: `GetSystemPowerStatus` (Win32, from memory **[M]**); `SHQueryUserNotificationState` (already in `quiet.rs`).
- SQLite WAL and `synchronous` (secondary, from memory **[M]**: "WAL with NORMAL may lose the last commits after a power loss but never corrupts").
- In this repo and on this PC: `desktop/src-tauri/src/{tray,quiet,core,storage,cli}.rs`, `desktop/src/settings.ts`, `android/.../link/{LinkService,LinkController,LinkRuntime,Notifications}.kt`, `clipboard/SendClipboardTile.kt`, `notifmirror/MirrorListenerService.kt`, `ui/{HomeScreen,SettingsScreen}.kt`, `AndroidManifest.xml`, `docs/extension.md` (the reserved `paused` field); a read-only probe of the power and notification-state APIs (a throwaway script in the temp directory, deleted afterwards).
