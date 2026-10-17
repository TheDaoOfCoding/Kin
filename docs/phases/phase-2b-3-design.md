# 2B.3 design: media controls in both directions

Written 2026-10-17 by the design agent. It is the contract for the desktop coder and the Android coder, who build 2B.3 in parallel without talking to each other. The plan it implements is "2B.3" in [phase-2b-plan.md](phase-2b-plan.md); the permission it reuses is the notification access of [phase-2b-2-design.md](phase-2b-2-design.md). **No product code was written for this document.** Nothing in the repo was touched except this file.

How to read it:

- Section 3 (protocol) is the contract between the two sides. The desktop coder copies it into `docs/protocol.md` (changelog 57) and writes the vectors. The Android coder implements it from here until that lands.
- Section 4 is Android only, section 5 is Windows only. They meet only through section 3.
- Section 6 says how every test makes its own silent media session and never touches the user's.
- **Evidence marks**, as in the 2B.2 design. **[V]** = verified here: code in this repo, the Android 37 SDK stubs (`javap`), or a **throwaway probe I built and ran on this PC** (section 2). **[D]** = quoted from official Microsoft, Android or AOSP documentation I read while writing (URLs in section 12). **[M]** = from memory of the API, not re-checked: the coder confirms it in the first hour. **[U]** = unverified and needs a real phone or PC, or a visual check: marked so nobody builds on it blindly.

## 1. Decisions in one table

| # | Decision | Why (short) |
|---|---|---|
| 1 | **Only one session per direction is shown: the "active" one**, chosen by the *source* with a fixed rule (3.5). `media.state` is a full snapshot that replaces the previous one; a changed `sid` means "another session". | Windows' flyout and the phone's carousel both work from one current session; a list would need a picker UI nobody asked for. `sid` is already on every message, so a later "several sessions" feature is not a protocol break |
| 2 | **Time is never sent as a clock value.** The source sends `positionMs` **as of the moment it sends**, plus `rateMilli`; the receiver anchors it to its own monotonic clock at arrival and extrapolates (3.4). | The two clocks differ (protocol.md already says so for `sentAt`). No streaming, no timer on either side. The error is half the network delay (milliseconds on a LAN) |
| 3 | **Everything that flows source → viewer is ephemeral** (like `notif.*`): no ack, no queue, no de-duplication, never re-sent. `media.cmd` too: it is not acked, **the result is the next `media.state`**; a failure is an `error` with `ref`. | A stale "now playing" or a replayed "next track" later is worse than none |
| 4 | **Capability `media` means "I understand `media.*`"** and is listed in every `hello` of a 0.2.0 build. The four switches (each side: "show the other's media here", "share my media") travel in a new **`media.status`** message, not in a new `hello`. | Four booleans and a UI line like "Pixel 8 is not sharing" need a status message anyway; this avoids two `hello`s per toggle. This differs on purpose from `notif` (where the PC's capability followed the switch) |
| 5 | **Kin's own sessions are never read back.** The PC's reader skips sessions whose `SourceAppUserModelId` equals this process's AUMID; the phone's reader skips sessions of its own package. | Without this, phone media shown on the PC is read as "PC media" and sent back to the phone: an echo loop. **[V]** the PC reader really sees the app's own session, under the process AUMID (section 2) |
| 6 | **Never use a `MediaPlayer`, never play audio, no `mediaPlayback` foreground service.** The PC registers SystemMediaTransportControls through `ISystemMediaTransportControlsInterop::GetForWindow` on a **dedicated hidden top-level window**; the phone uses a `MediaSession` plus a `MediaStyle` notification posted from the process `LinkService` already holds. | **[V]** the interop works for an unpackaged exe with no audio at all. **[V]** it **fails with a message-only window** (so Kin's clipboard window can't be reused). A `mediaPlayback` FGS must play media; Kin plays none (4.5) |
| 7 | **Both directions are off by default**, per side, each with its own switch. Now-playing data is **never stored and never logged**: not in the history, not in the database, not in `status.json`; only counts. Art is kept in memory only. | Privacy rule of the project. A paused session is dropped after 30 min |
| 8 | **Volume is a property of the source device and travels in `media.state` / `media.volume`**, set with `media.cmd`. A dev or test Kin **never writes the PC's master volume and never sends a command to a session it didn't create** (6.1). | Plan rule 7. The real master volume can only be set in the manual test of the user |
| 9 | **Lock screen:** phone: the PC media notification is `VISIBILITY_PRIVATE` unless the user opts in (the same rule as 2B.1 link previews). PC: the SMTC session is disabled while the Windows session is locked. | Now-playing is personal. **[D]** Android's own sample comment says `VISIBILITY_PUBLIC` is what shows the controls "even when user hides sensitive content" |
| 10 | The Windows **tray menu and the drop window** show the phone's media and its buttons as well, so the feature works when SMTC can't be registered (Windows N editions, a failed registration). | The plan asks for the tray popup too; it is also the fallback |

## 2. What I verified on this PC (read-only, silent)

Built a throwaway Rust probe (`windows` 0.62.2, the version Kin already uses) in a temp directory **outside the repo** (deleted at the end). Windows 11 Pro 10.0.26300. It never played audio, never called a `Set*` function of the volume API and never sent a command to a session it did not create. Only a Steam helper was running (no game, no anti-cheat). The user had one real session (a video in Edge, playing); I read it and printed **lengths and flags only, never a title, artist or name**.

| Question | Result | Evidence |
|---|---|---|
| **GSMTC from an unpackaged Win32 app: list sessions, read properties, timeline, thumbnail, controls, events?** | **Works.** `GlobalSystemMediaTransportControlsSessionManager::RequestAsync()` then `GetSessions()`, `GetCurrentSession()`, `GetPlaybackInfo()`, `GetTimelineProperties()`, `TryGetMediaPropertiesAsync()`, thumbnail `OpenReadAsync()`; no package identity, no capability declared. The real session: `SourceAppUserModelId` = `MSEdge`, status Playing, rate 1.0, `Controls` = pause, toggle, next, prev, stop and seek enabled (play **disabled while playing**), timeline `start 0`, `end 285.5 s`, **`position 0` with `lastUpdated` 103.9 s ago**, thumbnail 2,419 bytes `image/png` | **[V]** probe `read` |
| Events | `MediaPropertiesChanged`, `PlaybackInfoChanged`, `TimelinePropertiesChanged` (per session), `SessionsChanged`, `CurrentSessionChanged` (manager) registered fine. **0 events in 8 s and again in 3 s while the video played**: nothing ticks, so there is no polling and no position stream; position is `position + (now − lastUpdated) × rate` (here 0 + 103.9 s, a plausible value) | **[V]** |
| Min version | GSMTC: Windows 10 1809 (build 17763); the doc lists an `globalMediaControl` capability that is for packaged apps (the unpackaged probe ran without it) | **[D]** + **[V]** |
| **Master volume, read-only** | `IMMDeviceEnumerator` → default render endpoint → `IAudioEndpointVolume`: scalar 1.000, not muted, range −65.25..0 dB, **50 of 51 steps** (2 % each), 2 channels; `RegisterControlChangeNotify` with an `IAudioEndpointVolumeCallback` registered and removed fine (0 events: nothing changed). No `Set*` call exists in the probe | **[V]** |
| **SMTC for an unpackaged app: `GetForWindow` on a hidden top-level window, no audio** | **Works** (STA and MTA threads). Registered `IsEnabled`, play/pause/next/previous, `PlaybackStatus = Paused`, `PlaybackRate`, `DisplayUpdater` type Music with title/artist/album and a thumbnail from an in-memory stream, `UpdateTimelineProperties`; `Update()`. Seen from the outside through GSMTC within 1.5 s: a **second session** with all of it (title, artist, album, 70-byte PNG thumbnail, timeline start 0, end 200 s, position 42 s, controls play/next/prev/seek). It vanished when the process exited | **[V]** probe `smtc top sta|mta` |
| **`GetForWindow` on a message-only window (`HWND_MESSAGE`, what Kin's clipboard listener uses)** | **Fails: `E_INVALIDARG` (0x80070057).** The doc says it: "The `appWindow` parameter must refer to a top-level window that belongs to the calling process" | **[V]** probe `smtc msg` + **[D]** |
| Needs a `MediaPlayer`? | **No.** (A `MediaPlayer` with `CommandManager.IsEnabled = false` and no source also works and makes no sound, but pulls in Media Foundation for nothing.) | **[V]** probe `mediaplayer` |
| Commands back to our own session | `TryTogglePlayPauseAsync`, `TrySkipNextAsync` reached our `ButtonPressed` handler (Play, then Next); `TryChangePlaybackPositionAsync` reached `PlaybackPositionChangeRequested`. Only the session whose title was this run's random nonce got them | **[V]** |
| What name does the source show under? | `SourceAppUserModelId` = **the process's explicit AUMID** when set (`SetCurrentProcessExplicitAppUserModelID`; Kin sets it in `integration.rs`), else **the exe name** (`kin-2b3-probe.exe`) | **[V]** |
| **Trap found: self-deadlock** | Reading our own SMTC's thumbnail through GSMTC **from the STA thread that owns the SMTC** and blocking on it hangs forever (the system needs that thread's message pump). Do all SMTC work on the window's own thread and never block it on GSMTC calls | **[V]** (the probe hung twice before I moved the calls to a worker thread) |
| Not verifiable here | How the flyout *looks* (name, icon, whether the album line shows); the lock screen; Windows N editions (no Media Feature Pack); Windows 10 | **[U]** |

Android could not be run here (the emulator belongs to another agent). Its API facts come from the SDK stubs and AOSP source: **[V]** (`javap` on `android-37.0`) `MediaSessionManager.getActiveSessions(ComponentName)`, `addOnActiveSessionsChangedListener(listener, ComponentName, Handler)`, `MediaSession.setPlaybackToRemote(VolumeProvider)`, `MediaController.getExtras()`, `getPlaybackInfo()`, `setVolumeTo`, `adjustVolume`, `Notification.MediaStyle.setMediaSession`, `AudioManager.setStreamVolume/adjustStreamVolume/isVolumeFixed`, `Notification.EXTRA_MEDIA_SESSION`; and **[V]** that **`Notification.EXTRA_SUBSTITUTE_APP_NAME` and the `SUBSTITUTE_NOTIFICATION_APP_NAME` permission are not in the public SDK**, so the carousel's app name can't be changed to "PC · Spotify" (4.5).

## 3. Protocol (the contract)

Everything in this section goes into `docs/protocol.md` as **changelog 57**, with vectors in `protocol/` (3.12). Next free number: 56 is `notif`.

### 3.1 Roles, capability and status

- **Source** = the device whose media is shown elsewhere. **Viewer** = the device that shows it and sends commands. Both directions can run at once: phone as source and PC as viewer, and PC as source and phone as viewer, with different `sid`s.
- `hello.capabilities` gains **`media`** (case-sensitive). Every 0.2.0 build lists it. It means "I understand `media.*`".
- **`media.status`** (both directions, ephemeral): `{id, show, share, access?}`. `show` boolean (required): "I am showing your media". `share` boolean (required): "I am willing to share mine". `access` optional, `"granted"` or `"missing"`, phone only: notification access, needed to read other apps' sessions (4.2). Sent after each accepted `hello` and whenever a value changes, at most once per 5 s.
- **A source sends `media.state`, `media.image`, `media.clear` and `media.volume` only if** its own `share` is on, the peer's latest `hello` lists `media`, **and the peer's latest `media.status` says `show: true`** (before any status arrives it sends nothing). **A viewer sends `media.cmd` only to a peer whose latest `media.status` says `share: true`.**
- **Switch off while connected:** the side sends a new `media.status`. A viewer that turned `show` off removes its own session at once, and a source that sees `show: false` simply sends nothing more. A source that turned `share` off sends one `media.clear` and then nothing.
- **Races are silent.** A viewer whose `show` is off drops `media.state/image/clear/volume` silently (no `error`, counted). A source whose `share` is off answers `media.cmd` with `error` `disabled` (with `ref`; an `error` is never answered, so no loop). A peer without `media` in its `hello` would answer `unsupported_type`, which is harmless because nothing is sent to it.
- **Per connection.** Nothing about a peer's status or session is remembered across connections. When a connection closes (or the peer's `hello` is gone) a viewer removes its session at once.

### 3.2 Ephemeral class

`media.status`, `media.state`, `media.image`, `media.clear`, `media.volume` and `media.cmd` carry a valid `id` (so an `error` can `ref` it) and are **not acked, not de-duplicated, not stored, not queued and not re-sent after a reconnect** (the class of `notif.*`, protocol.md "Message IDs and acks"). A repeated `id` is processed again. After a reconnect the source sends its current state afresh (3.7). A `media.cmd` that gets no `state` change is simply not visible to the user; the viewer never sends it again.

### 3.3 Messages

| Type | Direction | Fields | Receiver does |
|---|---|---|---|
| `media.status` | both | `id`, `show`, `share`, `access` (optional) | Remembers it for this connection and for the Settings line |
| `media.state` | source → viewer | `id`, `sid`, `app`, `state`, optional `title`, `artist`, `album`, `durationMs`, `positionMs`, `rateMilli`, `actions`, `artHash`, `iconHash`, `volume` | Replaces the shown session |
| `media.image` | source → viewer | `id`, `hash`, `kind` (`art` or `icon`), `data` | Verifies and keeps the image in memory |
| `media.clear` | source → viewer | `id` | Removes the shown session |
| `media.volume` | source → viewer | `id`, `level`, `muted` | Updates the volume shown |
| `media.cmd` | viewer → source | `id`, `sid`, `cmd`, then by `cmd`: `seekMs`, `level`, `delta`, `mute` | Does it on the source's session or device; the effect comes back as `media.state` / `media.volume` |

**Field types.** Strings are cleaned by the **receiver** with the five steps of `link.title` (bidi controls → `_`, control characters → space, collapse spaces, cut to N code points, empty → absent). Integers follow the JSON conventions (no fraction, 0 to 2⁵³−1): **there are no floating-point fields**.

| Field | Type and rules |
|---|---|
| `sid` | String, exactly 32 characters from `0-9a-f` = first 32 hex of SHA-256 of the UTF-8 bytes of the source app's identity (Android: the **package name**; Windows: the **`SourceAppUserModelId`**). It names "the session of this app"; the raw package or AUMID never goes on the wire or into a log |
| `app` | **Required** string, the source app's display name, cut to **40**. Empty after cleaning is `bad_request` |
| `state` | **Required**, one of `playing`, `paused`, `buffering`, `stopped`. Another value is `bad_request` |
| `title` | Optional, **150**. `artist`, `album`: optional, **100** each |
| `durationMs` | Optional integer 1 to 86,400,000 (24 h). Absent = unknown or live (no seek, no progress bar) |
| `positionMs` | Optional integer 0 to 86,400,000: the position **at the moment the source sent this message** (it has already extrapolated: 3.4). With a `durationMs`, a larger value is cut to it. Absent = unknown (no progress bar) |
| `rateMilli` | Optional integer 0 to 4,000, default 1000 (1.0×). 0 behaves as paused. Over 4,000 is cut to 4,000 |
| `actions` | Optional array of at most 8 strings from `play`, `pause`, `toggle`, `next`, `prev`, `seek`: what the source's session offers right now. **Unknown entries are ignored** (a later version adds more). `seek` is only believed with a `durationMs` |
| `artHash`, `iconHash` | Optional, 32 characters from `0-9a-f`: the `hash` of a `media.image` sent earlier **on this connection** |
| `volume` | Optional object `{level, muted}`: `level` integer 0 to 100, `muted` boolean. Absent = the source can't set its volume (a fixed-volume device, a remote cast session) |
| `hash`, `kind`, `data` (image) | `hash` = first 32 hex of SHA-256 of the image bytes. `kind` is `art` or `icon`, anything else is `bad_request`. `data` = base64url (no padding) of a **PNG or JPEG, at most 24,576 bytes, longest side at most 256 pixels for `art` and 128 for `icon`, at least 16 pixels** |
| `level`, `muted` (volume) | As in `volume` above |
| `cmd` | One of `play`, `pause`, `toggle`, `next`, `prev`, `seek`, `volume` |
| `seekMs` | Integer 0 to 86,400,000. **Required if and only if** `cmd` is `seek` |
| `level`, `delta`, `mute` (in `media.cmd`) | Only with `cmd` = `volume`, **exactly one of them**: `level` integer 0 to 100 (absolute); `delta` integer, −100 to 100, not 0 (percentage points, relative; the source moves **at least one native step** in that direction); `mute` boolean. `sid` is optional for `cmd` = `volume` (the volume belongs to the device), required for every other `cmd` |

A `media.state` whose **frame** is over **16 KiB** is not sent (senders cut `album`, then `artist`, then `title`); a received one over **32 KiB** is `error` `too_large`. A `media.image` frame over **48 KiB** (24,576 bytes is 32,768 base64url characters) is `too_large`. A `media.image` with a wrong hash, a non-PNG/JPEG, a bad size or over the caps is **dropped silently and counted** (an image can't start an error storm); the next `media.state` just has no art. `bad_request` (with `ref`, connection stays open, `id` not remembered) for: a missing or invalid `id`, `sid`, `app`, `state`; a wrong type; `cmd` fields that break "exactly one"; `seekMs` missing for `seek`.

### 3.4 Position and clocks (both sides use the same rule)

```
anchor            = the viewer's monotonic clock when the message arrived
shown(now)        = state == playing  ? min(duration, positionMs + (now - anchor) * rateMilli / 1000)
                                       : positionMs            (paused, buffering, stopped)
sent positionMs   = what the source computes for "now" (below), then clamped to 0..duration
```

- **Source on Android:** `PlaybackState.position` is the position at `lastPositionUpdateTime` (both on `SystemClock.elapsedRealtime()`); if the state is `STATE_PLAYING`, `now = position + (elapsedRealtime − lastPositionUpdateTime) × playbackSpeed`. `PLAYBACK_POSITION_UNKNOWN` (−1) → no `positionMs`. **[D/M]**
- **Source on Windows:** `GetTimelineProperties()` has `Position` at `LastUpdatedTime` (a `DateTime`, 100 ns ticks since 1601). If the status is Playing: `now = Position + (now_filetime − LastUpdatedTime) × PlaybackRate` (`PlaybackRate` absent = 1). **[V]** on the real Edge session: position 0, last updated 103.9 s ago, nothing ever refreshed it: extrapolation is the only way.
- **When a source re-sends** (3.6): on any change of `state`, `title`, `artist`, `album`, `durationMs`, `rateMilli`, `actions`, art or volume, **on a seek** (a position change the extrapolation does not explain: the new extrapolated position differs from the last sent one by more than **1,500 ms**), and never on a timer.
- **Viewer on Android:** `PlaybackState.Builder.setState(state, positionMs, rateMilli / 1000f, SystemClock.elapsedRealtime())`; System UI then draws the moving progress bar itself. **Viewer on Windows:** `UpdateTimelineProperties` with the position at arrival (**[U]** whether the flyout's bar keeps moving by itself while `PlaybackStatus` is Playing; the Microsoft page for `UpdateTimelineProperties` says nothing about it. If it doesn't, the fallback is to call `UpdateTimelineProperties` every 5 s while the status is Playing: a timer in the PC app costs nothing compared with the phone's battery.)
- A `playing` state whose extrapolated position passes `durationMs + 5 s` with no new message is shown as paused at the end (a lost message can't leave a bar running forever).

### 3.5 Which session is "active" (the source decides)

1. **Candidates**: sessions that are not Kin's own (decision 5), have a title, and whose state is `playing`, `paused` or `buffering`. (`stopped` and "none" sessions are not shown.) In a test run only the allowed test session is a candidate (6.1).
2. If the current active session is still a candidate **and playing**, it stays (no flip-flop between two players).
3. Otherwise the **first candidate that is playing** in the platform's own priority order: Android `getActiveSessions` returns controllers "in priority order with the most important controller at index 0" **[D: AOSP javadoc]**; Windows: `GetCurrentSession()` ("the session the system believes the user would most likely want to control" **[D]**) first, then `GetSessions()` order.
4. Otherwise the current one if it is still a candidate, else the first candidate in that order.
5. No candidate: `media.clear`, **debounced 1.5 s** (a track change often destroys and recreates a session) and cancelled if a candidate comes back.
6. A session that has been `paused` for **30 minutes** is dropped (one `delay` armed on pause, not a poll) and re-sent when it plays again.

### 3.6 Rates, limits, ordering

- **Source, `media.state`:** at most one per **250 ms** (trailing: the last state of the burst goes out); a message identical to the last one sent (same fields, position within 1,500 ms of the extrapolation) is not sent; token bucket capacity 10, refill 1 per 250 ms, over budget → dropped and counted.
- **Source, `media.volume`:** trailing 250 ms (a slider drag or held key is one stream); identical value not sent.
- **Source, `media.image`:** a `media.image` goes out **before the first `media.state` that uses its hash on this connection**; once per hash per connection; at most 20 per 60 s.
- **Viewer, `media.cmd`:** `seek` and `volume` are coalesced on the viewer (trailing 150 ms, last value wins); at most 20 commands per 10 s from a viewer; **a source accepts at most 20 per 10 s** (bucket capacity 20, refill 2 per second) and answers `error` `rate_limited` (with `ref`, at most one per 10 s). A source that finds `sid` ≠ its active `sid` (or an action not in its `actions`) answers `error` `stale`; no active session or the app is gone: `not_found`; the system refuses: `action_failed`; its `share` is off: `disabled`. **Success has no reply**, and the viewer's own display only changes with the next `media.state`.
- **Viewer (receiver) limits:** a bucket per connection for `media.state` of capacity 20, refill 5 per second; over → dropped and counted, one `error` `rate_limited` (no `ref`) per 10 s at most.

### 3.7 Offline

- No connection, no accepted `hello`, a peer without `media`, or a peer whose status is not `show: true`: **drop**; nothing is stored; each side counts it.
- On every connection, after both `hello`s **and** the peer's `media.status` with `show: true`, a source sends its current session (`media.image` first, then `media.state`), or nothing if there is none. There is no "sync": the state is a full snapshot.
- A viewer removes everything it shows **when the connection closes**. (Limit: a link that dies silently is noticed only by the 240 s WebSocket ping, so a stale card can last minutes: risk R30.)

### 3.8 New error codes

None. `stale`, `not_found`, `disabled`, `rate_limited`, `action_failed`, `too_large`, `bad_request` are all already in protocol.md. (`stale` here means: "that session or button is not what you thought".)

### 3.9 Cleaning and validation

As 3.3. **Over-long strings are cut, never rejected** (except the frame limits). `actions` entries that are unknown are ignored. Booleans and integers are strict (`"true"` and `1.0` are `bad_request`); `null` counts as absent for optional fields.

### 3.10 What changes in the existing docs and code (desktop coder does these)

- `docs/protocol.md`: Messages table (6 rows), Field types, a "Media: `media.*`" section holding 3.1 to 3.9, "Message IDs and acks" (the ephemeral list), Versioning (`media`), changelog 57. Validation step 6c: `media.*` is dropped silently when the sender's latest `hello` does not list `media`, then the viewer-side rule of 3.1; frame limits.
- `desktop/src-tauri/src/messages.rs`: `CAPABILITIES` becomes `["link", "notif"?, "media"]` (the `notif` one is already dynamic: keep that). `ws.rs`/`core.rs`: send and keep `media.status`.
- Android `Messages.kt`/`MessageReceiver.kt`: `hello(... capabilities = listOf(link, notif, media))`.

### 3.11 What the viewer shows (rules both sides follow)

Title line = `title` (else `app`); second line = `artist`, plus the **device**: phone viewer shows `<artist> · <PC name>` and the notification's sub text `<app> on <PC name>`; PC viewer puts `<phone name> · <app>` in the album line **[U: whether the flyout shows the album line]** and always shows it in the tray panel. No viewer makes anything in `title`, `artist` or `album` clickable. A viewer shows a control only if the source listed the action (`actions`).

### 3.12 Vectors to add (the desktop coder owns `protocol/`)

New file **`protocol/media.json`**, new `media-*` cases in `messages.json`, a `scope-2b.3` group in `check-vectors.ts`, a section in `protocol/README.md`.

| Array | What it pins | Used by |
|---|---|---|
| `sid[]` | identity → `sid`: package names (`com.spotify.music`), AUMIDs (`Spotify.exe`, `MSEdge`, a packaged `SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify`), a non-ASCII one | both |
| `clean[]` | per field: bidi, newlines, long, emoji at the cut → output, with the limits of 3.3 | both (receivers clean; senders must not exceed) |
| `state[]` | valid and invalid `media.state` objects → accepted or `bad_request` + reason (bad `state`, `rateMilli` over 4,000 cut, position over duration cut, unknown action ignored, `seek` without duration) | both |
| `cmd[]` | valid and invalid `media.cmd` objects (exactly one of `level`/`delta`/`mute`, `seekMs` rule, `sid` optional for volume only) | both |
| `position[]` | `{state, positionMs, rateMilli, durationMs, elapsedMs} → shown`, including paused, buffering, rate 0, rate 2000, past the end, no duration | both |
| `sourcePosition[]` | the source's own extrapolation: Android `(position, updateTime, speed, now)` and Windows `(position, lastUpdated filetime, rate, now)` → `positionMs`; unknown position | both |
| `pick[]` | a list of sessions (own flag, title, state, order, current) → the chosen `sid` or `clear`; the 30-minute pause rule; hysteresis | both |
| `androidMap[]` | `PlaybackState` (state, actions bit mask, speed) ↔ wire (`state`, `actions`, `rateMilli`) both ways | Android |
| `windowsMap[]` | GSMTC `PlaybackStatus` (0 to 5) and the `Controls` flags ↔ wire; wire → SMTC status and button enables | PC |
| `volume[]` | native index ↔ `level` (Android `index/max`, Windows scalar), `delta` minimum one native step, mute | both |
| `image[]` | hash, kind, size rules, PNG/JPEG magic, the 24,576-byte and 256/128 pixel caps | both |
| `rate[]` | a timeline `(t_ms, kind)` → accepted/dropped for the source's state, volume, image limits and the viewer's command limit; trailing coalescing | both |
| `messages.json` | `media-state-minimal/full/bad-*`, `media-cmd-*`, `media-status-*`; sequences: `media-not-sent-before-peer-status`, `media-dropped-when-viewer-off-is-silent`, `media-cmd-to-non-sharing-source-is-disabled`, `media-cmd-stale-sid-is-stale`, `media-status-change-needs-no-new-hello`, `media-cmd-is-not-resent-after-reconnect`, `media-is-not-in-the-dedup-memory`, `media-image-before-state` | both |

## 4. Android

### 4.1 Pieces (names are suggestions; package `media/`)

| Piece | Type | Job |
|---|---|---|
| `MediaSourceController` | Android-bound | **Phone → PC.** Registers `MediaSessionManager.OnActiveSessionsChangedListener` and a `MediaController.Callback` per candidate; builds `MediaSnapshot`s |
| `MediaSnapshot`, `MediaPick`, `MediaWire`, `MediaPosition` | pure | The snapshot data class, 3.5, the wire fields and limits, 3.4. **JVM-testable without Android classes** (the 2B.2 pattern `NotifSnapshot`) |
| `ArtworkEncoder` | Android-bound, thin | Bitmap → ≤ 24,576-byte JPEG (4.3) |
| `MediaSender` | pure + fake clock | The only writer of `media.*` to the socket: coalescing, budgets, image-before-state, peer-status gate, debounced clear (3.6, 3.7) |
| `PcMediaSession` | Android-bound | **PC → phone.** One framework `MediaSession`, the `PlaybackState`, the metadata, the `VolumeProvider` |
| `PcMediaNotification` | Android-bound | The `MediaStyle` notification and its channel `kin_pc_media` |
| `MediaCommandRunner` | Android-bound | Runs a `media.cmd` on the chosen controller or `AudioManager`, with the error codes of 3.6 |
| `MediaPolicyStore` | DataStore (its own, `kin_media`) | `share` (default false), `show` (default false), `showOnLockScreen` (false), `useVolumeKeys` (false). **No content** |
| `MediaScreens` | Compose | Settings rows, a Home card, the access hint |

Wire into the existing link like 2B.2: `LinkSession` sends `media.status` after each accepted `hello` and on a setting change; `IncomingHandler` handles `media.*`; `Messages.kt`/`MessageReceiver.kt` get the types. **`MediaSender` is the only thing that writes `media.*`**, and it checks the peer's capability and status per message. **It never starts `LinkService`** (a background start of a foreground service isn't allowed on newer versions); with no connection it drops.

### 4.2 Phone → PC: reading the phone's media

- **Permission: the 2B.2 notification access, nothing new.** **[D: AOSP `MediaSessionManager` javadoc]** "This requires the `MEDIA_CONTENT_CONTROL` permission be held by the calling app. You may also retrieve this list if your app is an enabled notification listener." `MEDIA_CONTENT_CONTROL` is a signature permission, so Kin uses the listener route: `getActiveSessions(ComponentName(context, MirrorListenerService::class.java))` and `addOnActiveSessionsChangedListener(listener, thatComponent, handler)`. **[V]** both signatures are in the API 37 stubs. A missing access throws `SecurityException` (the check is in the system service, not in the client class): catch it, set `media.status.access = "missing"`, and show the same "turn on notification access" card as 2B.2 (**sharing media does not need "Show notifications on your PC" to be on**; it needs the same *access*, and the restricted-settings help of 2B.2 4.2 applies unchanged). The first thing the coder does is call it on `kin_api37` and `kin_api29` with and without access **[U: Android 15/16 changes**]; record it.
- **Only while it can be used.** Register the listener and the controller callbacks **only while** `share` is on, access is granted, a connection is ready and the peer's status says `show: true`; unregister the moment any of those goes away, and release every controller (no reference outlives it). **No polling, no timers, no wake locks**: the only `delay`s are the 250 ms trailing window, the 1.5 s clear debounce and the 30 min paused expiry.
- **Callbacks:** `MediaController.registerCallback(callback, handler)`: `onPlaybackStateChanged`, `onMetadataChanged`, `onSessionDestroyed`, `onAudioInfoChanged` (volume of a remote/cast session). `onSessionEvent`, `onExtrasChanged`, `onQueue*` are ignored. Every callback only queues an event; **one coroutine** takes them in order (the pattern of `MirrorListenerService`), so a state and its clear can't swap places.
- **What is read:** `packageName` (only to compute `sid` and the label), `metadata` title, artist, album, duration (`METADATA_KEY_DURATION`, ≤ 0 = unknown), the art bitmap, `playbackState` state, position, speed, `lastPositionUpdateTime`, `actions`, `playbackInfo` volume. **Never read:** `METADATA_KEY_*_URI` (a URI from another app is never fetched: no network, no `ContentResolver` read on its behalf), the queue, the session's extras (except the debug marker, 4.9), media-id strings, the user's ratings. App label: `PackageManager.getApplicationLabel` through `getApplicationInfo` (visible because `getActiveSessions` hands the package; **[U]** whether it throws for an app with no launcher icon, as 2B.2 4.3 noted: fall back to the label of the session `tag`, then to "Media").
- **Own sessions skipped:** `controller.packageName == context.packageName` is not a candidate (this is what stops the PC → phone session from echoing back). Debug builds make one exception: a session whose extras carry `kin.debug_media = true` (4.9).
- **Active session:** 3.5. Android's priority order is the system's.
- **Position and actions:** 3.4 and the table `androidMap[]`. `PlaybackState.actions` bit → wire: `ACTION_PLAY` (4) → `play`, `ACTION_PAUSE` (2) → `pause`, `ACTION_PLAY_PAUSE` (512) → `toggle`, `ACTION_SKIP_TO_NEXT` (32) → `next`, `ACTION_SKIP_TO_PREVIOUS` (16) → `prev`, `ACTION_SEEK_TO` (256) → `seek` (only with a duration). States: `STATE_PLAYING` → `playing`; `STATE_PAUSED` → `paused`; `STATE_BUFFERING` and `STATE_CONNECTING` → `buffering`; `STATE_NONE`, `STATE_STOPPED`, `STATE_ERROR` → not a candidate.
- **Commands (PC → phone `media.cmd`):** `controller.transportControls.play()`, `pause()` (`toggle` → `pause()` if playing else `play()`), `skipToNext()`, `skipToPrevious()`, `seekTo(seekMs)`. **[M]** a notification listener's controller may use `transportControls` (every system media UI and wear companion does); the coder checks it first on the emulator with the debug test session (4.9) and records it. Before running, the runner compares `sid` and checks the action is in the controller's current `actions` (`stale` otherwise), then catches any exception → `action_failed`.

### 4.3 Artwork

- `metadata.getBitmap(METADATA_KEY_ALBUM_ART)`, else `METADATA_KEY_ART`, else `METADATA_KEY_DISPLAY_ICON`. No URIs (4.2).
- Re-encode **only when the art changed**: when the bitmap reference, its size or the `sid` changed, or the title changed; on a background dispatcher; the bitmap is never retained after encoding.
- **Downscale** (`Bitmap.createScaledBitmap`) so the longest side is **at most 256** (the source may hand over 1,000 × 1,000 px or more; **[U]** how large Spotify, YouTube Music and Poweramp bitmaps are: measure and write it in the results), then JPEG at quality 80, then 70, 60, 50, then side 192, 128, until the result is **≤ 24,576 bytes**; if even 128 px/50 fails, send no art. **Hash** = first 32 hex of SHA-256 of the JPEG. Keep the last 8 results in memory (not on disk: album art is personal, section 7).
- App icon: reuse the 2B.2 icon code (`AndroidIcons`: the launcher icon at 96 px, PNG ≤ 12,000 bytes) and send it as `media.image` `kind = icon`, once per connection, only if the peer is a PC (it always is). The PC uses it only in the tray panel.

### 4.4 Phone media volume (phone → PC direction: the PC's buttons change the phone's volume)

- **Which volume:** if `controller.playbackInfo.playbackType == PLAYBACK_TYPE_REMOTE` (a cast or other remote route) the volume is the session's own: `playbackInfo.currentVolume/maxVolume`, set with `controller.setVolumeTo(index, 0)` and `adjustVolume(direction, 0)`, and only if `volumeControl != VOLUME_CONTROL_FIXED`. **[M]** Otherwise (local playback) it is **`STREAM_MUSIC`**: `AudioManager.getStreamVolume/getStreamMaxVolume/isStreamMute`, set with `setStreamVolume(STREAM_MUSIC, index, 0)` (no `FLAG_SHOW_UI`: the PC is where the user is looking; the Debug screen may add it). `AudioManager.isVolumeFixed()` true → send no `volume` at all. **[V]** all of these exist in the API 37 stubs.
- **Permission: none for `STREAM_MUSIC`.** The documented DND rule is about volume changes that would toggle Do Not Disturb: **[D: AOSP via search, secondary]** "From Android N onward, volume adjustments that would toggle Do Not Disturb are not allowed unless the app has been granted Do Not Disturb access", and apps report `SecurityException: Not allowed to change Do Not Disturb state` when they push `STREAM_MUSIC` **to zero** on some versions (VLC). **[U]** the exact condition on the test images. Therefore: **wrap every call in `try/catch (SecurityException)` → `action_failed`; the lowest level Kin sets by `level`/`delta` is index 1 (never 0) unless `mute` is used, and `mute` uses `adjustStreamVolume(ADJUST_MUTE)`**; if that throws, `action_failed`. Kin does **not** ask for `ACCESS_NOTIFICATION_POLICY` (that would be a second special access for a convenience), and never touches ringer mode, `STREAM_RING`, `STREAM_NOTIFICATION`, `STREAM_ALARM` or DND. What a phone in "total silence" does is recorded in the results.
- **Mapping:** `level = round(index × 100 / max)`, `index = round(level × max / 100)`; a `delta` moves the index by at least one step in its direction (`volume[]`).
- **Reporting it:** `volume` in each `media.state`; changes in between: `onAudioInfoChanged` for a remote session; for local playback **[U]** there is no public callback for `STREAM_MUSIC` changes that isn't a broadcast (`android.media.VOLUME_CHANGED_ACTION` is a hidden constant). **Decision: v1 sends the phone's volume with each `media.state` and after a `media.cmd volume` of its own; it does not watch for changes made on the phone** (no receiver, nothing to leak). The PC's slider is therefore only as fresh as the last state; a note in R33.

### 4.5 PC → phone: a Kin `MediaSession` and a `MediaStyle` notification

**What the system does with it [D]:** the quick-settings carousel lists "sessions from multiple apps"; to be there "you must create a `MediaStyle` notification with a valid `MediaSession` token"; for apps targeting Android 13+ the system derives the buttons from the `PlaybackState` **actions** (up to five; the first three in compact mode), and for apps with no `PlaybackState` it falls back to the notification's own actions. The same page's sample sets `VISIBILITY_PUBLIC` with the comment "Show controls on lock screen even when user hides sensitive content." **The pages do not mention a foreground service as a requirement for being in the carousel** (they only name `MediaSessionService`, which publishes the notification for you). **[U]** whether a session with no audio and a *plain* (non-foreground) notification gets the carousel card on Android 13, 14, 15, 16 and the AOSP emulator: **the Android coder checks this first (spike S3, section 10) and records it.** The fallback if it doesn't is to attach the notification to the existing `LinkService` foreground notification group; if even that fails, show the PC's media as an ordinary notification with buttons, and record the carousel as a limit.

**Build (framework classes only; minSdk 29 has all of them):**

1. `MediaSession(context, "kin-pc-media")`, `setCallback(Callback)`: `onPlay`, `onPause`, `onSkipToNext`, `onSkipToPrevious`, `onSeekTo(pos)` each send **one `media.cmd`** (`play`, `pause`, `next`, `prev`, `seek`) with the current `sid`; **no local effect**, the state comes back from the PC. `setSessionActivity(PendingIntent → MainActivity)`. `setActive(true)` only while a session is shown.
2. `setMetadata(MediaMetadata)`: `TITLE`, `ARTIST` (= `<artist> · <PC name>`, 3.11: the app name of the card can't be substituted, **[V] `EXTRA_SUBSTITUTE_APP_NAME` isn't in the public SDK**), `ALBUM`, `DURATION`, `ALBUM_ART` and `ART` (the decoded `media.image`, `BitmapFactory` with `inJustDecodeBounds` first, rejected over 512 px or 2 MiB decoded). `METADATA_KEY_DISPLAY_SUBTITLE` is also set to the same line **[M: System UI prefers it]**.
3. `setPlaybackState(PlaybackState.Builder().setActions(mask).setState(state, positionMs, rateMilli / 1000f, SystemClock.elapsedRealtime()).build())` with `mask` from `actions` (reverse of 4.2). Re-set only on a `media.state`, never on a timer.
4. The notification: channel **`kin_pc_media`** (`IMPORTANCE_LOW`, no sound, no vibration, no badge), one fixed id, `Notification.Builder(context, channel).setSmallIcon(ic_stat_kin).setStyle(Notification.MediaStyle().setMediaSession(token).setShowActionsInCompactView(...))`, title and text from the metadata, large icon = the art, `setCategory(CATEGORY_TRANSPORT)`, `setOnlyAlertOnce(true)`, `setLocalOnly(true)` (not bridged to a watch), `setOngoing(state == playing)` **[U: Android 13+ already decides this for playing sessions]**, `setVisibility(VISIBILITY_PRIVATE)` with a `publicVersion` that says only "Media from <PC>" (or `VISIBILITY_PUBLIC` when the user turned on "Show on the lock screen"), `setSubText("<app> on <PC name>")`, and (for the pre-13 and no-`PlaybackState` fallback) the same buttons as notification actions (`PendingIntent` broadcasts to a not-exported receiver). **Posted from the process `LinkService` already holds**: no new foreground service.
5. `deleteIntent` → a not-exported receiver: the user swiped it away → remember `dismissedSid`, don't show that `sid` again until `playing` resumes or the `sid`/title changes (don't fight the user).
6. **Volume (opt-in, decision Q3):** only if the setting "Use the phone's volume keys for the PC" is on: `setPlaybackToRemote(object : VolumeProvider(VOLUME_CONTROL_ABSOLUTE, 100, level) { onSetVolumeTo(v) → media.cmd volume level v; onAdjustVolume(d) → media.cmd volume delta ±5 })` and `setCurrentVolume` on each `media.volume`. **[M/U]** Android sends the hardware volume keys to the active remote-playback session; that is the documented route for "a session that controls a remote volume", and it also shows a PC volume slider where the system supports it. With the setting off (default) no `VolumeProvider` is set, and the volume is a slider on the Kin Home card and the Debug screen.
7. **Remove everything** (`setActive(false)`, `release()`, `cancel(id)`) on `media.clear`, on connection close, on `show` off, on unpair, when the notification permission is denied.

**Foreground service type and Play policy.** Kin already holds `LinkService` with type `connectedDevice` (android.md). **A `mediaPlayback` FGS type is for "continue audio or video playback from the background" [D]**; Kin plays nothing, so it **does not** declare `FOREGROUND_SERVICE_MEDIA_PLAYBACK`, doesn't start a media FGS, and adds no manifest permission for 2B.3. **[D]** "If your app targets Android 14 or higher, you'll need to declare your app's foreground service types in the Play Console": Kin's one declared type (`connectedDevice`) is unchanged. Declaring `mediaPlayback` without playing would misstate the service. **[U]** whether Play review objects to a `MediaSession` that plays no audio (I found no policy text that forbids it); the reviewer notes it in R32 next to R26.

**Audio focus and media keys [U, real device only].** Kin never requests audio focus. But while Kin's session is `playing`, the system may route **headset and car media buttons** (and Bluetooth AVRCP metadata) to it, so a headset button could pause the PC while the user wanted the phone's player. Mitigations: the feature is off by default; the notification says "<app> on <PC name>"; open question Q2 (mirror `playing` honestly vs a paused look). R31.

### 4.6 Skipping the media notifications of 2B.2 (the coordination rule)

**Nothing changes in 2B.2:** `MirrorFilter` rule 7 already skips `CATEGORY_TRANSPORT`, `MediaStyle` and `EXTRA_MEDIA_SESSION` notifications as `media` (**[V]** `MirrorFilter.kt:62`, `SbnMapper.kt:56`), and Kin's own package is skipped by rule 1 (the debug exception is only for channels `kin_debug_mirror*`; `kin_pc_media` must **not** match it). 2B.3 adds a test that the PC media notification and a debug `MediaStyle` test notification are never mirrored.

### 4.7 The user toggles, the lock screen

- Settings → **"Media"**: "Show this phone's media on your PC" (`share`, default off), "Show your PC's media on this phone" (`show`, default off), "Show PC media on the lock screen" (default off), "Use the phone's volume keys for the PC" (default off, Q3). A Home row while either direction is on, with the state in words ("Pixel 8 isn't showing your media: turn it on in Kin on your PC" comes from the peer's `media.status`, "Notification access is off" from `access`). Turning `share` on with no access opens the 2B.2 access screen.
- **Lock screen:** `VISIBILITY_PRIVATE` plus a public version ("Media from <PC>") by default; **Android applies it only when the phone's own lock-screen setting hides sensitive content** (as for 2B.1 link previews); the Settings text says so. The user-visible media on a locked phone is whatever the phone's setting decides (**[U]** how each OEM treats a media card with `VISIBILITY_PRIVATE`).

### 4.8 Battery

Callbacks only, nothing between calls. No receiver for volume changes (4.4). Listener and callbacks exist only while there is a place to send to (4.2). Artwork is encoded once per track change, on a background thread, with an in-memory cache of 8. A state is 0.3 to 1 KiB on the open WebSocket; a position never streams. The viewer side holds one `MediaSession`; System UI draws the progress. **A battery number is a real-phone item** (TESTING.md: `dumpsys batterystats` over an hour with a playing PC session); an emulator can't give one.

### 4.9 The debug build (test hygiene, section 6.2)

- **`DebugMediaSession`** (debug source set only): **a `MediaSession` of Kin's own with no `MediaPlayer`, no `AudioTrack`, no audio focus**, tag `kin_debug_media`, `setExtras(kin.debug_media = true)`, a callback that only logs the commands it receives (action, `seekMs`; **no titles**), metadata with a **generated** bitmap (a 640 × 640 gradient, so the downscale is exercised), `PlaybackState` set from the commands below. The reader accepts it despite the own-package rule (4.2), release builds contain none of this.
- **`DebugCommandReceiver` gets** (each prints a `CMD …` line): `MEDIA_SET [--ez share true|false] [--ez show true|false] [--ez lockscreen …] [--ez volkeys …]`; `MEDIA_TEST_START [--es title <t>] [--es artist <a>] [--ei durationMs <n>]`; `MEDIA_TEST_STATE --es state playing|paused|buffering|stopped [--ei positionMs <n>] [--ei rateMilli <n>] [--es actions play,pause,next,prev,seek]`; `MEDIA_TEST_STOP`; `MEDIA_STATE` (counts and the last command each way, **no content**: `share show access listener peerShow peerShare sent images dropped={…} cmdsReceived={play:n,…} lastCmd pcSession={sid-prefix,state} sdk`); `MEDIA_PC_CMD --es cmd play|pause|next|prev|seek|volume [--ei …]` (**calls the callback on Kin's own PC-media `MediaSession` through its own `MediaController`**: exactly what the carousel buttons do, with no input injection); `MEDIA_VOLUME --ei level <n>` (phone `STREAM_MUSIC` on the emulator, which is not the user's device).
- **Granting access in the emulator:** as 2B.2 (`adb shell cmd notification allow_listener …`).
- **Verify from outside, read-only:** `adb shell dumpsys media_session` (is Kin's `kin-pc-media` active, its `PlaybackState`, its metadata title), `adb shell dumpsys notification --noredact` for the `MediaStyle` notification **(titles of test sessions only; the emulator holds none of the user's data)**. A screenshot of the quick-settings carousel is taken by the **integration agent** (the one that owns the emulator). **Do not** use `adb shell cmd media_session dispatch …` against anything but the debug session.

### 4.10 Permissions and manifest

**No new permission and no manifest change** apart from nothing: the listener service is already declared (2B.2), `POST_NOTIFICATIONS` is already requested, and no `FOREGROUND_SERVICE_MEDIA_PLAYBACK`. `docs/android.md` gets a "Media" section and the Pieces table rows; its permission table gets no new row.

### 4.11 What the phone stores and logs

Stores: the four DataStore booleans, the in-memory art cache (never on disk), the icon cache of 2B.2 is **not** reused on disk for media. **No title, artist, album, app name, `sid`, art or position is stored or logged.** Logs: once per 60 s while active, one line with counts: `media: sent N states, N images, N cmds in, dropped {offline: N, rate: N, peer_off: N}`; settings changes (`media: share on`). `ExportRedactor` needs no new rule if this holds; a test greps a run's log for the test strings.

## 5. Windows

### 5.1 Pieces (new, desktop coder; names are suggestions; `media/` next to `mirror/`)

| Piece | Job |
|---|---|
| `media/wire.rs`, `media/position.rs`, `media/pick.rs`, `media/limits.rs` | Pure: validation and cleaning of `media.*`, 3.4, 3.5, the token buckets (the 2B.2 `mirror/wire.rs`, `limits.rs` pattern; `rate[]` vectors) |
| `media/reader.rs` | **PC → phone.** GSMTC reader on its own thread (MTA): sessions, events, thumbnails, `Snapshot`s. Behind a trait with a fake for tests |
| `media/art.rs` | Thumbnail stream → bounded decode (`images.rs`' WIC decoder) → ≤ 256 px → JPEG ≤ 24,576 bytes (the WIC JPEG encoder is already used in `images.rs` tests, **[V]** `images.rs:543`); a thumbnail that already fits (Edge's is a 2,419-byte PNG **[V]**) passes through |
| `media/volume.rs` | `IAudioEndpointVolume` behind a trait: read, set, step, mute, change notifications. **The real `Set*` calls are reachable only through the guard of 6.1** |
| `media/surface.rs` | **Phone → PC.** The SMTC surface on a dedicated hidden window (5.3), behind a trait with a fake |
| `media/mod.rs` | The core's view: settings, `media.status`, which messages go where, the viewer state in memory, counters; feeds the tray and the drop window |

### 5.2 PC → phone: reading the PC's media (GSMTC)

- **Verified [V]** (section 2): works unpackaged, read-only, event-driven; `Controls` flags, `PlaybackRate`, timeline and thumbnail all readable; **zero events while a video plays**.
- **Thread:** `CoInitializeEx(MTA)` on a thread of its own; `RequestAsync().join()`; keep the manager and the per-session event tokens. **All handlers only post to a channel**; the core debounces **300 ms** (a track change fires properties, playback and timeline events one after the other, and the thumbnail can arrive after the title **[M]**: when a snapshot has no thumbnail, read it again once after 1.5 s).
- **Own session excluded:** compare `SourceAppUserModelId` with **`integration::process_app_user_model_id()`** (the running process's AUMID, **[V]** the function exists; the installed Kin is `app.kinrelay.desktop`, every other instance `app.kinrelay.desktop.dev-<key>`; never a constant). **[V]** the probe's own session appeared in `GetSessions()` under exactly that string.
- **Mapping (`windowsMap[]`):** `Playing` → `playing`; `Paused` → `paused`; `Changing` → `buffering`; `Stopped`, `Opened`, `Closed` → not a candidate. `Controls`: `IsPlayEnabled` → `play`, `IsPauseEnabled` → `pause`, `IsPlayPauseToggleEnabled` → `toggle`, `IsNextEnabled` → `next`, `IsPreviousEnabled` → `prev`, `IsPlaybackPositionEnabled` (and an `EndTime − StartTime` > 0) → `seek`. **[V]** the real Edge session while playing: play disabled, pause, toggle, next, previous, stop, seek enabled. Duration = `EndTime − StartTime` (0 or negative = unknown), position as 3.4, `rateMilli` = `PlaybackRate × 1000` (null = 1000).
- **App name:** `SourceAppUserModelId` is an exe name (`Spotify.exe`, here `MSEdge` **[V]**) or a package id (`SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify`). Display name: for a package id `Windows.ApplicationModel.AppInfo.GetFromAppUserModelId(id).DisplayInfo.DisplayName` **[M: confirm it works unpackaged]**; for an exe name a small table (Spotify, Chrome, Edge, Firefox, VLC, foobar2000, Brave, Opera, YouTube Music, Apple Music…); else the id with `.exe` removed. Cut to 40.
- **Commands (phone → PC `media.cmd`):** `session.TryPlayAsync()`, `TryPauseAsync()`, `TryTogglePlayPauseAsync()`, `TrySkipNextAsync()`, `TrySkipPreviousAsync()`, `TryChangePlaybackPositionAsync(ticks)` (**100 ns ticks**: `seekMs × 10,000`). **[V]** all of these reached a session's handler in the probe (on our own session). Applied only to the session whose `sid` is the active one and whose current `Controls` allow it (else `stale`); a `false` result → `action_failed`. **Never** applied in a test run to a session other than the allowed test one (6.1).
- **Volume (`media/volume.rs`):** `IMMDeviceEnumerator` → `GetDefaultAudioEndpoint(eRender, eMultimedia)` → `Activate::<IAudioEndpointVolume>` **[V]**; `level = round(scalar × 100)`; the slider of Windows is the scalar and has **51 steps of 2 %** on the dev PC **[V]**. Set: `SetMasterVolumeLevelScalar(level / 100, &KIN_CONTEXT_GUID)`, `delta` via the same with a minimum of one native step (`GetVolumeStepInfo`), `SetMute`. Kin's own GUID is passed as the event context so the change notification can be recognised in the log counts. Changes made by the user elsewhere reach Kin through `RegisterControlChangeNotify` **[V: registers and unregisters]** → `media.volume` (trailing 250 ms). The default device can change (headphones plugged in): re-acquire on `IMMNotificationClient::OnDefaultDeviceChanged` **[M]**. **A setting "Let the phone change this PC's volume" is not added**: `share` on is the consent; the phone cannot go past what Windows' slider does.
- **Settings keys** (`settings.json`, defaults off): `mediaShare` (this PC's media to the phone), `mediaShow` (the phone's media on this PC); the Settings window gets a "Media" section with the two switches and the phone's `media.status` line ("Pixel 8 isn't showing your media: turn it on in Kin on the phone" / "Update Kin on your phone" when it never sent a status).

### 5.3 Phone → PC: the phone's media on the PC (SMTC)

**The question and the answer.** Can an unpackaged Win32 app put its own session in the Windows media flyout, with no audio and no `MediaPlayer`? **Yes [V]**: `ISystemMediaTransportControlsInterop::GetForWindow` on a **top-level hidden window** returns a working `SystemMediaTransportControls`; setting `IsEnabled`, the button enables, `PlaybackStatus`, `DisplayUpdater` and `UpdateTimelineProperties` made a session visible to GSMTC with title, artist, album, thumbnail, timeline and controls within 1.5 s, and commands sent to it came back as `ButtonPressed` and `PlaybackPositionChangeRequested`. **It fails for a message-only window [V, E_INVALIDARG, and the doc says "top-level"], so Kin's `KinClipboardListener` window cannot be used, and `HWND_MESSAGE` must not be used for this.**

**Recipe (the desktop coder implements; see the appendix for the verified calls):**

1. A **dedicated thread** (`CoInitializeEx` MTA or STA: both worked **[V]**) creates a window of its own class (`KinMediaControls`), **no parent, style 0, never shown** (the single-instance plugin's window is the same kind: `instance.rs`), then `windows::core::factory::<SystemMediaTransportControls, ISystemMediaTransportControlsInterop>()?.GetForWindow::<SystemMediaTransportControls>(hwnd)`. Features to add to `windows`: `Media`, `Storage_Streams`, `Win32_System_WinRT` (all else is already there; `Media_Control` for 5.2; `Win32_Graphics_Gdi` is needed by `RegisterClassW`).
2. The thread runs a message loop and **does every SMTC call itself** (core → thread by a channel and `PostMessage`). **Never block that thread on a GSMTC call about its own session: it deadlocks [V]** (it is also why the reader of 5.2 lives on another thread and skips Kin's own session).
3. On `media.state`: `IsEnabled = true`; `IsPlayEnabled`/`IsPauseEnabled`/`IsNextEnabled`/`IsPreviousEnabled` from `actions` (`toggle` enables both play and pause); `PlaybackStatus` from `state` (playing → `Playing`, paused → `Paused`, buffering → `Changing`, stopped → `Stopped`); `PlaybackRate`; `DisplayUpdater.Type = Music`, `MusicProperties` title/artist/album(+device line, 3.11), thumbnail from `RandomAccessStreamReference::CreateFromStream(InMemoryRandomAccessStream)`; `Update()`; then `UpdateTimelineProperties` (start 0, end = duration, position as 3.4, `MinSeekTime`/`MaxSeekTime` = 0/duration when `seek` is offered). Seek needs `IsEnabled` and an end time.
4. **Events → `media.cmd`**, posted to the async runtime at once (the activator's pattern): `ButtonPressed` Play → `play`, Pause → `pause`, Next → `next`, Previous → `prev` (other buttons are not enabled, so never arrive); `PlaybackPositionChangeRequested` → `seek` with `RequestedPlaybackPosition` (**[V]** the event fired). The viewer then waits for the phone's `media.state` to move the display; it does not guess.
5. On `media.clear`, connection close, `show` off, unpair, **or the Windows session locking**: `DisplayUpdater.ClearAll()` and `IsEnabled = false` (**[V]** the session was gone from GSMTC after the process ended; **[U]** that `IsEnabled = false` removes it from the flyout at once). Lock detection: `WTSRegisterSessionNotification` on this window, `WM_WTSSESSION_CHANGE` `WTS_SESSION_LOCK`/`UNLOCK` **[M]**; Windows can show media controls on the lock screen **[M]**, and a phone's now-playing title must not.
6. **What the flyout shows [V/U]:** the source is named from the process AUMID (`app.kinrelay.desktop`; **[V]** with no AUMID it would be the exe name), so the flyout says **"Kin"** with the app's icon where the Start Menu shortcut carries that AUMID **[U: look at it]**. A separate flyout name ("Kin: phone media") would need a second AUMID for the window; **not tried**, not needed.
7. **Never `Playing` in a test run** (6.1). The mapping `playing → Playing` is covered by the fake surface in unit tests and by the user's manual check.
8. **Windows requirements [D]:** GSMTC 1809; SMTC and the interop are older. **[U]** Windows N editions without the Media Feature Pack, and Windows 10: if `GetForWindow` fails, the code logs one count line, sets the status line "Windows can't show media here", and the tray panel (5.4) carries on.

### 5.4 The tray menu and the drop window (the fallback, and the place the device is always named)

- **Tray menu** (`tray.rs`): while a phone session is shown, three items appear under the status line, **"▶ Play/Pause phone media"**, **"⏭ Next"**, **"⏮ Previous"**, with the title **not** in the menu or the tooltip (they're visible to anyone near the PC; only the device and "playing" are: "Kin: Pixel 8 connected, media playing"). They send the same `media.cmd`.
- **Drop window** (left-click on the tray): a "Phone media" strip at the top while a phone session is shown: art, title, artist, `<app> on <phone>`, previous/play-pause/next, a seek bar from the extrapolated position (a UI timer in the webview, nothing in Rust), and a volume slider if `volume` is present. The window is destroyed when closed (windows.md process model), so nothing runs while it is closed. Art is held in memory by the core and handed to the webview as a `data:` URL (the CSP already allows `img-src 'self' data:`).
- The same strip shows "PC media → phone: <title> — <artist>" read-only with the PC's own master volume slider, so the user can see what is being shared (the transparency rule T15 of 2B.2).

### 5.5 What the PC stores and logs

Stores: two booleans in `settings.json`. **Nothing else**: no table, no file, no art, no `sid`. Logs: counts and a keyed tag (the 2B.2 `privacy.rs` HMAC tag), no title, artist, album, app name, AUMID of another app, or art; `status.json` gets `media: {share, show, state: "none|playing|paused", counts: {...}}` (numbers and an enum, no content). Tests grep the log, the data directory and `status.json` for the test strings.

## 6. Safety for tests: how each side makes its own silent session

### 6.1 The two guards in the product code (so a test *cannot* hurt the user)

1. **`may_control_real_sessions`** (the pattern of `registry::may_register_release`): only the installed Kin (a release run the way a user runs it, no developer setting, no `--data-dir`) reads and controls the user's sessions and writes the master volume. **Every other instance** (a copy beside it, every test, a debug build) runs in **test-media mode**:
   - the GSMTC reader looks only at the session whose `SourceAppUserModelId` equals `--dev-media-aumid <id>`: it compares that string **before** reading any property of a session, so a real session's metadata, thumbnail and timeline are never even requested; with the option missing it reads **no** session;
   - `media.cmd` is applied only to that session;
   - the master volume is a **fake** (a trait implementation that remembers a number); the real `Set*` is reachable only with `--dev-media-real-volume`, which **no agent ever passes** (only the user's manual test, TESTING.md);
   - SMTC `PlaybackStatus` is never `Playing` (a `Playing` Windows session takes the keyboard media keys): `playing` is shown as `Paused`; `--dev-media-quiet` is implied.
2. The phone has no such risk for the user's PC. On the emulator the Debug build reads only sessions whose extras carry `kin.debug_media`, **unless** the debug command `MEDIA_READ_ALL` is sent (used only by the integration agent when it needs to prove that a real third-party session is read, on the emulator where there is none of the user's).

### 6.2 Recipes

**PC side: "a silent test session of its own" (`desktop/src-tauri/examples/media_test_session.rs`, from the appendix).** A small exe that sets the process AUMID `app.kinrelay.dev.mediatest-<pid>`, creates the hidden top-level window, registers SMTC as in 5.3 with a **random nonce title**, `PlaybackStatus = Paused` (never `Playing`), a generated thumbnail, a timeline, prints `READY aumid=<id> title=<nonce>`, reads commands on stdin (`title X`, `artist X`, `state paused|stopped`, `pos <ms>`, `art`, `actions play,pause,next,prev,seek`, `exit`), and appends each `ButtonPressed` and `PlaybackPositionChangeRequested` it receives to a file. Run the **dev Kin** with `--dev-media-aumid app.kinrelay.dev.mediatest-<pid>` (the reader sees nothing else). The phone then shows that session; a command from the phone arrives at the test exe's handler; the test checks the file. No sound, nothing of the user's touched. The exe ends by `IsEnabled = false` and exiting; a test that fails must still kill its own process (`finally`).

**PC side: "what did Kin put in the flyout?" (`media_test_session.rs read <aumid>` or a `toast_probe`-style mode).** A read-only GSMTC reader that opens **only** the session whose AUMID equals the one it is given (the dev Kin's own, `app.kinrelay.desktop.dev-<key>`, from `status.json`), prints title, artist, state, position, controls, thumbnail size, and may call `TryPlayAsync`/`TrySkipNextAsync` **on that session only** (that is Kin's own dev session: the command goes back through Kin to the emulator's test player). It never lists or opens any other session. (My probe did the same with a nonce title instead of an AUMID.)

**PC volume.** The real endpoint is **read-only** in every automated test (`media_test_session.rs volume-read`, the `read` mode of the probe). The set path is exercised through the fake (unit tests and the e2e). A **real** volume change is a manual step of the user (TESTING.md).

**Phone side: "a silent test session of its own".** `DebugMediaSession` (4.9): a `MediaSession` with no audio at all, driven by `MEDIA_TEST_*`. The commands the PC sends to it are logged by its own callback (`cmdsReceived`). The reverse direction is checked with `dumpsys media_session` and the carousel screenshot. The emulator is started with its audio muted/off for the integration run, and the e2e never calls a player.

**Never:** `adb shell cmd media_session dispatch` or `volume` against anything but the debug session; a real music app on the emulator; `Playing` on the PC test session; any call to a `Set*` of the real endpoint; `nircmd`-like tools; reading the user's session metadata (the reader guard makes it impossible by default).

## 7. Privacy and security

| # | Threat | Mitigation | Residual |
|---|---|---|---|
| T1 | Now-playing data (title, artist, album, art, what app) ends up in logs, the history, the database, `status.json` or the debug export | Never stored, never logged (4.11, 5.5); counts and a keyed tag only; art in memory only; tests grep for it | None known |
| T2 | The phone's media shows on a PC screen others can see, or on a locked PC | Off by default; SMTC disabled while locked (5.3); the tray menu and tooltip carry no title; the drop window shows it only while open | Windows' flyout and the title bar of a player are outside Kin |
| T3 | The PC's media shows on a locked phone | `VISIBILITY_PRIVATE` + a public version by default; `setLocalOnly(true)` (not bridged to a watch) | The OEM lock-screen setting decides |
| T4 | A paired but compromised peer sends hostile text or images | Strings cleaned and cut by the receiver (3.3); images checked by hash, magic bytes, size, dimensions, 24,576 bytes; decoded with bounded decoders (`BitmapFactory` with bounds first; the WIC decoder of `images.rs`); no URL from any metadata is ever fetched | A paired peer can show any title |
| T5 | A peer makes Kin press things it shouldn't | `media.cmd` is limited to six verbs and two volume forms, applies only to the active `sid` and only to actions the session lists, 20 per 10 s; a `sid` the source never announced is `stale`; **no** command can start an app, change a setting or reach another session | A paired PC can pause the phone's music and change its media volume: the trust of pairing |
| T6 | Volume used to annoy or hurt (volume to 100 % in a quiet room) | The user's own switch; `level` ≤ 100 only; no faster than 20 commands per 10 s; the phone can go to 100 % only because the user's own hardware can | A paired device can make the PC or the phone loud. Documented. (R34) |
| T7 | Echo loop between the two directions | Own-session filter on both sides (decision 5) | None |
| T8 | A test or an agent touches the user's sessions or volume | The guards of 6.1 are in the product code | The installed Kin does what the user asked it to |
| T9 | Reading other apps' media is a broad capability (it needs notification access) | Same discipline as 2B.2: nothing read until the switch is on, the peer wants it and a connection is ready; no URI fetching; the queue, extras and ratings never read | Notification access is broad by nature |
| T10 | A stale display after a silent disconnect | Cleared on close; the end-of-track rule (3.4); the 30-min paused expiry | Minutes on a dead link (R30) |

**Unit-testable invariants:** a hostile `title` (markup, bidi, 100 KB) never changes anything but displayed text; a `media.image` of 24,577 bytes, 257 px, a wrong hash or a GIF is dropped; a `media.cmd` for a `sid` that isn't active is `stale`; no code path logs a title (a test with a logger that fails on the test strings).

## 8. New risks for `docs/risks.md` (the reviewer adds them at 2B.6; 2B.2 used R19–R27, R18 is reserved)

| # | Risk | Likelihood | Impact | What we do about it | When we find out |
|---|---|---|---|---|---|
| R28 | A `MediaSession` with no audio and a plain notification doesn't get the quick-settings carousel card (or loses it) on some Android versions or OEM System UIs | Medium | Medium | Spike S3 first; fallback to an ordinary notification with the same buttons; the Home card works regardless | S3, then a real phone |
| R29 | Notification access is needed to read other apps' media, so the "Restricted setting" and Android 15+ hurdles of R19 now also block media sharing | High | Medium | The 2B.2 setup and help unchanged; `media.status.access` tells the PC and the Home card says why | Real hardware |
| R30 | A link that dies silently leaves a stale "now playing" for up to ~4 min (the 240 s WebSocket ping); the PC side has the same | Medium | Low | Cleared on close, the end-of-track rule, the paused expiry | Real phone, Wi-Fi off test |
| R31 | While Kin's session is `playing`, the headset or car media buttons and Bluetooth metadata go to Kin's session, so a button could pause the PC instead of the phone's player | Medium | Medium | Off by default; "<app> on <PC>" label; open question Q2; one real-phone test with headphones and a car/Bluetooth device | Real phone |
| R32 | Play review questions a `MediaSession` that plays no audio, or the extra use of notification access for media (R26) | Low | Medium | No `mediaPlayback` FGS; no new permission; the description says what it does; prepared with R4/R26 | Phase 2 Play submission |
| R33 | The phone's `STREAM_MUSIC` volume can change on the phone without Kin knowing (no public callback), so the PC's slider can be stale; and `setStreamVolume` may throw a DND `SecurityException` | Medium | Low | Volume sent with each state and after each command; `try/catch` → `action_failed`; floor of index 1 | Emulator and real phone |
| R34 | A paired device can make the other side loud | Low | Low | Opt-in switches, rate limit, documented (T6) | — |
| R35 | The SMTC registration depends on Windows behaviour not visible to scripts: flyout look, lock-screen exposure, `IsEnabled=false` removal, Windows N editions, Windows 10; and a Windows update could change the "message-only fails" behaviour | Medium | Low | The tray menu and the drop window always work; the code logs a count and carries on; `GetForWindow` failure is not fatal | User's real PC (TESTING.md) |
| R36 | Spotify, YouTube, browser and OEM players differ: some give no position or duration, no art bitmap (URI only), no `seek` action, a session that vanishes between tracks, a title that lags the art | High | Low | Every field optional on the wire; debounces (300 ms, 1.5 s); art re-read once; no art is acceptable; list the apps tried in TESTING.md | Real phone and PC |

## 9. Test plan (lean)

Follows the user's lean-testing rule: unit tests once, an emulator e2e only for what changed, no full regression.

### 9.1 Unit tests

**Android (JVM, no Robolectric; Android classes behind `MediaSnapshot`):** `MediaPick` (`pick[]`: hysteresis, 30-min pause, debounced clear), `MediaPosition` (`sourcePosition[]`, `position[]`), `MediaWire` (limits, cleaning, 16 KiB shrink, `actions` mapping `androidMap[]`), `MediaSender` with a fake clock (`rate[]`: trailing 250 ms, identical state skipped, bucket, image before state, nothing before the peer's status, silent drop when the peer has `show` off, per-message capability check), volume mapping `volume[]` (min one step, floor of index 1), `MediaCommandRunner` error codes with fake controllers (`stale`, `not_found`, `disabled`, `action_failed`, `rate_limited`), the viewer side `PlaybackState` and metadata builders as data (the mask), parsing `media.*` against `messages.json`, the filter test of 4.6, the log grep.

**Desktop (`cargo test`):** `media/wire.rs` parsing and cleaning (vectors), `windowsMap[]`, `pick[]`, `position[]`, `rate[]`, image checks (bad hash, size, GIF, 257 px, odd PNG), the viewer state machine with the **fake surface** (clear on close, lock, `show` off; `playing` → `Paused` in test mode), `media.cmd` → fake session calls (`stale` when the control isn't enabled), the volume fake (`delta` min step), the own-AUMID filter and the test-mode guards of 6.1 as pure functions (a real session AUMID is never opened; the real `Set*` is unreachable without the flag), settings defaults and `media.status` re-sends, logs grep.

**Vectors:** 3.12; `bun protocol/check-vectors.ts` stays green.

### 9.2 Emulator e2e (short; the integration agent writes `tools/e2e-2b-3.ps1`)

`kin_api37` (and `kin_api29`), the real desktop build with `--dev-bind 127.0.0.1`, a throwaway data dir, `--dev-media-aumid <test aumid>`, `--dev-toast-quiet`. **No audio anywhere, no clicks, no input injection.**

1. **Setup:** `cmd notification allow_listener …`; `MEDIA_SET share=true show=true`; start `media_test_session.exe`; `MEDIA_STATE` and `status.json` show both statuses seen.
2. **Phone → PC:** `MEDIA_TEST_START` + `MEDIA_TEST_STATE paused`; the PC reader of the test exe (`read <dev Kin's aumid>`) sees the title, artist, state, controls from `actions`, thumbnail size **> 0 and ≤ 24,576 bytes** (the generated 640 px bitmap was downscaled), position; update the state/position → changes appear; **no `Playing`** on the PC.
3. **PC → phone commands from the PC side:** the reader calls `TryTogglePlayPauseAsync`, `TrySkipNextAsync`, `TryChangePlaybackPositionAsync` **on the dev Kin's own session** → the debug session logs `play/pause`, `next`, `seek N` (`cmdsReceived`).
4. **PC → phone:** drive the PC test session (`title`, `state paused`, `pos`, `art`); `dumpsys media_session` shows `kin-pc-media` active with that title and state; the notification is a `MediaStyle` one; the carousel screenshot (integration agent). `MEDIA_PC_CMD play|next|seek` → the PC test exe's handler file shows `ButtonPressed`/`PlaybackPositionChangeRequested`.
5. **Volume:** `volume` in the state; `MEDIA_VOLUME`; a phone-side `media.cmd volume` from the fake PC changes the **emulator's** `STREAM_MUSIC` and the `media.volume` comes back; the PC side `volume` goes to the **fake** endpoint only (the read-only real one is read once and compared to "unchanged" before and after).
6. **Echo:** both directions on at once: no `sid` ping-pong (state counts stay flat).
7. **Offline and clear:** stop the dev PC → the phone's `kin-pc-media` is gone within the close; start it → the current state reappears after the statuses; the phone test session stopped → the PC's SMTC disabled after 1.5 s.
8. **Rate:** 200 state changes in 3 s → at most the bucket reaches the PC.
9. **Skips and logs:** the 2B.2 listener does not mirror the media notification; grep the phone and PC logs, the data dir and `status.json` for the test strings (none).
10. **Cleanup:** the test exe and its window, `MEDIA_TEST_STOP`, `disallow_listener`, no HKCU key was written by this feature (none is needed).

### 9.3 What can't be tested without a person or hardware

Real sound-free *look* of the Windows flyout (name, icon, album line, whether the bar moves while Playing); the lock screen on both devices; `Playing` on the PC session and the media keys; a **real** PC volume change and mute through the phone; real players (Spotify, YouTube Music, a browser, VLC; local and cast sessions); headset and car button routing (R31); the carousel on real Pixel and Samsung phones, Android 13 to 16; the `VolumeProvider` and the phone's volume keys; OEM lock-screen behaviour; DND and `STREAM_MUSIC`; battery. All of these go in `docs/TESTING.md` as the user's real-hardware checklist at v0.2.0.

## 10. Work split and first steps

**Desktop coder** owns: `protocol/` (vectors, `check-vectors.ts`), `docs/protocol.md` changelog 57 (section 3 copied in), the Windows code (5), `docs/windows.md`, `examples/media_test_session.rs`, `examples/fake_phone.rs` (a `--media` mode that plays the phone: status, state, image, commands it receives), the dev switches `--dev-media-aumid`, `--dev-media-real-volume`, `--dev-media-quiet`.

1. **Spike S2 (first hour, inside Kin):** the dedicated window + SMTC thread inside the Tauri process (apartment, the single-instance plugin, a Tauri tray); the GSMTC reader thread inside Kin; the own-AUMID filter; write the outcome in `phase-2b-3-desktop.md`.
2. `protocol/media.json`, the `messages.json` cases and the protocol.md section **as early as possible**: the Android coder runs them as soon as they exist.
3. `wire`, `pick`, `limits`, `position`, the fake surface and fake reader, then `reader`, `art`, `volume`, `surface`, tray and drop window, settings and status.

**Android coder** owns: the Android code (4), `Messages.kt`/`MessageReceiver.kt` for `media.*`, the debug tools (4.9), `docs/android.md`, `android/README.md` (the new `CMD`s). A spec problem goes in its results file; it doesn't edit `protocol/` or `docs/protocol.md`.

1. **Spike S3 (first hours, `kin_api37` and `kin_api29`, shared emulator rule: one agent at a time):** (a) `getActiveSessions(listener component)` with and without access; (b) a `MediaSession` with no audio and a plain notification: does the carousel card appear, how does a `playing` vs `paused` session behave, is it dismissable (R28); (c) `transportControls` from a listener controller on the debug session; (d) a `VolumeProvider` session and the hardware volume keys (`adb shell input keyevent KEYCODE_VOLUME_UP` on the emulator); (e) `STREAM_MUSIC` `setStreamVolume` to 0 and to 1 under DND on the images; (f) the art sizes real apps hand over, if any real player exists on the image (it doesn't need to).
2. Pure parts first (pick, position, wire, sender, volume mapping) against this document's tables until `protocol/media.json` exists, then against it.
3. The source controller, the PC-media session and notification, then screens, then the debug commands, so the integration agent can start.

**Integration agent** (after both): `tools/e2e-2b-3.ps1` and `docs/results/phase-2b-3-integration.md` per 9.2, with the emulator used by that one agent only.

**Results files:** `docs/results/phase-2b-3-{desktop,android,integration}.md`.

## 11. Open questions for the user

| # | Question | Recommendation |
|---|---|---|
| Q1 | Phone → PC: share media from **every app** that has a media session (what this design does, behind one switch), or only from apps the user picked (a per-app list like notification mirroring)? | **Every app, one switch, off by default.** A media session is an explicit "I am a player" signal, and a per-app list is a screen of its own; add a short "don't share these apps" list later if anyone asks. The PC → phone direction has no such question (the PC chooses the active session, the user decides on the phone) |
| Q2 | While the PC is playing, should the phone's Kin session say **playing** (honest; the carousel shows a pause button; but a headset or car button may then pause the PC), or **look paused** (no button capture; but the carousel shows a play button for a song that is playing)? | **Playing**, with the feature off by default and "<app> on <PC>" in the notification. Honest display is the point of the feature; test the headset and car case once on the real phone (R31) and change it if it surprises |
| Q3 | Should the phone's **hardware volume keys** control the PC's volume while a PC session is shown (`VolumeProvider`)? | **Off by default, a switch** ("Use the phone's volume keys for the PC"): taking the phone's own volume keys is surprising, so it's the user's choice; the volume is a slider on the Home card either way |

## 12. Sources

- Android: AOSP [`MediaSessionManager.java`](https://android.googlesource.com/platform/frameworks/base/+/refs/heads/main/media/java/android/media/session/MediaSessionManager.java) (the `getActiveSessions` permission and notification-listener alternative, "priority order with the most important controller at index 0", the system-side check); [Media controls (quick settings, Android 13+)](https://developer.android.com/media/implement/surfaces/mobile) (carousel, `MediaStyle` with a session token, `PlaybackState`-derived actions, `VISIBILITY_PUBLIC` comment); [Android 13 behavior changes: media controls derived from `PlaybackState`](https://developer.android.com/about/versions/13/behavior-changes-13); [Foreground service types](https://developer.android.com/develop/background-work/services/fgs/service-types) (`mediaPlayback`, `connectedDevice`, the Play Console declaration); the Android 37 SDK stubs read with `javap` (`platforms/android-37.0/android.jar`).
- Secondary, via search: the DND rule for volume changes (AudioManager reference text, the VLC Android commit and the B4X thread that show `SecurityException: Not allowed to change Do Not Disturb state`); the pages themselves for `AudioManager.setStreamVolume` were too long for my fetch, so the exact condition is **[U]**.
- Windows: [`GlobalSystemMediaTransportControlsSessionManager`](https://learn.microsoft.com/en-us/uwp/api/windows.media.control.globalsystemmediatransportcontrolssessionmanager) (Windows 10 1809; `GetCurrentSession` meaning; the `globalMediaControl` capability); [`ISystemMediaTransportControlsInterop::GetForWindow`](https://learn.microsoft.com/en-us/windows/win32/api/systemmediatransportcontrolsinterop/nf-systemmediatransportcontrolsinterop-isystemmediatransportcontrolsinterop-getforwindow) ("top-level window that belongs to the calling process"); [`SystemMediaTransportControls.UpdateTimelineProperties`](https://learn.microsoft.com/en-us/uwp/api/windows.media.systemmediatransportcontrols.updatetimelineproperties) (no remark about extrapolation: **[U]**).
- In this repo and on this PC: the throwaway probe described in section 2 (deleted afterwards); `desktop/src-tauri/src/{integration,instance,clipboard,images,tray,mirror/*}.rs`; `android/.../notifmirror/{MirrorListenerService,MirrorFilter,SbnMapper}.kt`, `link/LinkService.kt`; `docs/results/phase-2b-2-spike-s1.md`.

## Appendix: the verified calls (reference, not product code)

The probe's core, in the `windows` 0.62.2 crate, as it ran. Cargo features: `Win32_Foundation`, `Win32_System_Com`, `Win32_System_LibraryLoader`, `Win32_System_WinRT`, `Win32_UI_WindowsAndMessaging`, `Win32_Graphics_Gdi`, `Win32_UI_Shell`, `Media`, `Media_Control`, `Storage_Streams`, `Foundation`, `Foundation_Collections`, `Win32_Media_Audio`, `Win32_Media_Audio_Endpoints`, `Win32_System_Com_StructuredStorage`, `Win32_System_Variant`; `windows-core = "=0.62.2"` if `#[implement]` is used. Async operations are blocked on with **`.join()`** (windows-future 0.3), not `.get()`.

```rust
// Top-level hidden window: CreateWindowExW(.., parent = None, style 0). With Some(HWND_MESSAGE) GetForWindow returns E_INVALIDARG.
let interop = windows::core::factory::<SystemMediaTransportControls, ISystemMediaTransportControlsInterop>()?;
let smtc: SystemMediaTransportControls = unsafe { interop.GetForWindow(hwnd)? };
smtc.SetIsEnabled(true)?; smtc.SetIsPlayEnabled(true)?; smtc.SetIsPauseEnabled(true)?;
smtc.SetIsNextEnabled(true)?; smtc.SetIsPreviousEnabled(true)?;
smtc.SetPlaybackStatus(MediaPlaybackStatus::Paused)?; smtc.SetPlaybackRate(1.0)?;
let du = smtc.DisplayUpdater()?; du.SetType(MediaPlaybackType::Music)?;
let mp = du.MusicProperties()?; mp.SetTitle(&HSTRING::from(title))?; mp.SetArtist(..)?; mp.SetAlbumTitle(..)?;
let stream = InMemoryRandomAccessStream::new()?; let dw = DataWriter::CreateDataWriter(&stream)?;
dw.WriteBytes(&jpeg_or_png)?; dw.StoreAsync()?.join()?; dw.DetachStream()?; stream.Seek(0)?;
du.SetThumbnail(&RandomAccessStreamReference::CreateFromStream(&stream)?)?; du.Update()?;
let tl = SystemMediaTransportControlsTimelineProperties::new()?;          // TimeSpan { Duration: 100 ns ticks }
tl.SetStartTime(..)?; tl.SetEndTime(..)?; tl.SetMinSeekTime(..)?; tl.SetMaxSeekTime(..)?; tl.SetPosition(..)?;
smtc.UpdateTimelineProperties(&tl)?;
smtc.ButtonPressed(&TypedEventHandler::<SystemMediaTransportControls, SystemMediaTransportControlsButtonPressedEventArgs>::new(|_, a| { /* a.as_ref()?.Button() */ Ok(()) }))?;
smtc.PlaybackPositionChangeRequested(&TypedEventHandler::<SystemMediaTransportControls, PlaybackPositionChangeRequestedEventArgs>::new(|_, a| { /* RequestedPlaybackPosition() */ Ok(()) }))?;
// ... message loop on this thread (PeekMessageW/DispatchMessageW). To stop: du.ClearAll()?; smtc.SetIsEnabled(false)?; DestroyWindow.
```

Reading (GSMTC) and the volume endpoint, read-only:

```rust
let mgr = GlobalSystemMediaTransportControlsSessionManager::RequestAsync()?.join()?;   // MTA thread
for s in &mgr.GetSessions()? {
    let aumid = s.SourceAppUserModelId()?.to_string();         // compare with the process AUMID BEFORE reading anything else
    let info = s.GetPlaybackInfo()?;                           // PlaybackStatus (Closed 0, Opened 1, Changing 2, Stopped 3, Playing 4, Paused 5), PlaybackRate (IReference<f64>), Controls()
    let tl = s.GetTimelineProperties()?;                       // StartTime, EndTime, Position, LastUpdatedTime (DateTime: 100 ns ticks since 1601)
    let p = s.TryGetMediaPropertiesAsync()?.join()?;           // Title, Artist, AlbumTitle, Thumbnail() -> OpenReadAsync()
}
let en: IMMDeviceEnumerator = unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? };
let vol: IAudioEndpointVolume = unsafe { en.GetDefaultAudioEndpoint(eRender, eMultimedia)?.Activate(CLSCTX_ALL, None)? };
// GetMasterVolumeLevelScalar / GetMute / GetVolumeStepInfo / RegisterControlChangeNotify: read-only. SetMasterVolumeLevelScalar / SetMute: product code only, behind the guard of 6.1.
```
