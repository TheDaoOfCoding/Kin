# 2B.2 design: notification mirroring, phone → PC

Written 2026-10-17 by the design agent. It is the contract for the desktop coder and the Android coder, who build 2B.2 in parallel without talking to each other. The plan it implements is "2B.2" in [phase-2b-plan.md](phase-2b-plan.md). **No product code was written for this document.**

How to read it:

- Section 3 (protocol) is the contract between the two sides. The desktop coder copies it into `docs/protocol.md` (changelog 56) and writes the vectors. The Android coder implements it from here until that lands.
- Section 4 is Android only. Section 5 is Windows only. They don't depend on each other except through section 3.
- **Evidence marks.** **[V]** = verified here (code in this repo, the source of a crate in `~/.cargo`, or the registry of this PC). **[D]** = quoted from Microsoft or Android documentation that I read while writing this (the URL is in section 11). **[M]** = from my memory of the API, not re-checked: the coder confirms it in the first hour. **[U]** = unverified and probably needs a real phone or PC: marked so nobody builds on it blindly.

## 1. Decisions in one table

| # | Decision | Why (short) |
|---|---|---|
| 1 | Notification messages are **ephemeral**: no ack, no dedup memory, never queued, never re-sent. Only `notif.action` is acked (and never re-sent either). | A stale toast is worse than none (plan). A queue or a dedup table in the database would also write notification traffic to disk |
| 2 | The wire identity of a notification is **`nid` = first 32 hex characters of SHA-256 of the Android notification key**. Raw keys never leave the phone. | Keys contain the package, the id and the tag, and tags are sometimes a contact ID or a phone number. The PC, its logs and Windows' Action Center never see them |
| 3 | New capability **`notif`** in `hello`. The phone always lists it; the PC lists it **only while "Show phone notifications" is on**, and sends a new `hello` when that changes. | No new message for the on/off switch. Reuses the capability mechanism of changelog 54 |
| 4 | The PC shows these toasts with **its own WinRT toast layer** (the `windows` crate), under **its own AUMID** `app.kinrelay.desktop.phone`, and receives clicks and replies through a **COM activator**. `tauri-plugin-notification` stays for the Phase 1 toasts. | The plugin can't do an input box, a tag, a group, a history removal or an update (checked in its source, section 2). Own AUMID = no regression risk for the Phase 1 buttons and a separate Windows switch for these toasts |
| 5 | Allow list is **default off for every app**, except messaging apps that are installed, which the picker pre-selects. Known banking, authenticator, password-manager and wallet apps are flagged, need a confirmation, and are shown on the PC **without a preview** (`sensitive`). | Privacy by default (plan), no `QUERY_ALL_PACKAGES` |
| 6 | **No exemption from the Android 15 OTP redaction is attempted** in 2B.2. Kin detects it and shows "Code hidden by Android". The Companion Device Manager route is a later research item (section 4.9). | The only documented exemptions are companion associations or the `RECEIVE_SENSITIVE_NOTIFICATIONS` roles; claiming a PC is a "watch" is not honest, and the PC profile is unproven |
| 7 | PC → phone dismissal uses **only** the `ToastDismissed` event with reason `UserCanceled`. No polling of the Action Center in v0.2.0. | A wrong guess cancels a real notification on the phone. Coverage is documented as partial until the real-hardware test |
| 8 | Allowed notifications hidden on the lock screen (`VISIBILITY_SECRET`) are mirrored only for apps the user ticked for it, and then as `sensitive`. | Plan: "unless the user allows them" |

## 2. What Phase 1 actually achieved with toasts (R10)

All of this is **[V]** from this repo and from the crate sources in `~/.cargo/registry`:

- Every Phase 1 toast is made by `app.notification().builder()…show()` of `tauri-plugin-notification` 2.6.0 (`desktop/src-tauri/src/app.rs:486, 503, 516, 631`). On Windows that goes through `notify-rust` 4.18.2 to `tauri-winrt-notification` 0.8.1.
- **Buttons work for an unpackaged app, but only while the toast is on screen.** The plugin keeps the click handler as a closure in the running process (`wait_for_response`, one blocked thread per toast); its own docs say "a click is only reported while the toast is on screen, not once it moved to the Action Center" (`desktop.rs`, doc comment of `show`). windows.md and `phase-1-3-desktop.md` record the same finding from the real app (Windows 11).
- **No activator is registered.** The installer's Start Menu shortcut carries `System.AppUserModel.ID` = `app.kinrelay.desktop` only, and `integration.rs` sets the process AUMID. There is no `ToastActivatorCLSID` and no COM class.
- **What the wrapper can't do** (read in `tauri-winrt-notification` 0.8.1 `lib.rs` and `notify-rust` `windows.rs`): no `<input>` (text box), no `Tag` and `Group` on the toast (only on progress toasts), no `ToastNotificationHistory`, no update, no `SuppressPopup`, `ExpirationTime` or `NotificationMirroring`, an inline image only through a path, `close()` is a no-op on Windows (the plugin's own comment: "`notify-rust` cannot remove a shown notification").
- **Phase 1 never tested** an Action Center click, a click after the toast object is gone, or a click when Kin had been restarted.

Consequence: **2B.2 needs a toast layer the plugin can't provide.** The `windows` 0.62.2 crate that is already a dependency has everything [V, from its source]: `Win32::UI::Notifications::INotificationActivationCallback` (IID `53e31837-6600-4a81-9395-75cffe746f94`) with `NOTIFICATION_USER_INPUT_DATA`; `UI::Notifications::ToastNotification` with `SetTag`, `SetGroup`, `SetExpirationTime`, `SetExpiresOnReboot`, `SetSuppressPopup`, `SetNotificationMirroring`, `Dismissed`, `Activated`; `ToastActivatedEventArgs::UserInput`; `ToastNotificationHistory::GetHistory` and `RemoveGroupedTagWithId`. Cargo features to add: `UI_Notifications`, `Data_Xml_Dom`, `Foundation_Collections` and `Win32_UI_Notifications` (feature names read from the crate's `Cargo.toml`).

The Windows App SDK `AppNotificationManager` was considered and **rejected**: it depends on the Windows App SDK singleton MSIX package at run time [D: its `Register` page, "has a dependency on the Singleton package"], which is the opposite of a 3.6 MiB per-machine NSIS installer.

## 3. Protocol (the contract)

Everything in this section goes into `docs/protocol.md` as changelog 56, with vectors in `protocol/` (3.11).

### 3.1 Capability

- `hello.capabilities` gains **`notif`** (case-sensitive, like `link`).
- **Phone (0.2.0 build):** always lists `notif`. It means "I understand `notif.dismiss` and `notif.action`" and "I can mirror".
- **PC:** lists `notif` **only while the setting "Show phone notifications on this PC" is on**. When the setting changes while a phone is connected, the PC sends a new `hello` on that connection with the new `capabilities` (protocol.md already allows a later `hello`: "handled the same way", and a side keeps the capabilities of the latest accepted `hello` of the connection). It means "I'll show `notif.post`".
- **A side sends a `notif.*` message only to a peer whose latest `hello` lists `notif`.** The check is made when the message goes on the wire, as for `link`.
- **Peer without it:** nothing to downgrade to, so nothing is sent. The phone's Settings says "Your PC needs Kin 0.2.0 and the setting turned on"; the PC's Settings disables the switch's phone side with "Update Kin on your phone". A 0.1.x peer that did receive one would answer `unsupported_type` (existing rule), which is harmless: it is never sent. **A receiver that understands `notif.*` but has the feature off drops the message silently** (no `error`, so a race after the switch can't start an error loop), and counts it.

### 3.2 Ephemeral messages

A new class, defined once in protocol.md "Message IDs and acks": **ephemeral messages carry an `id` (valid format, so an `error` can `ref` it) but** are **not acked, not put in the de-duplication memory, not written to any database, not queued, and not re-sent after a reconnect.** They are `notif.status`, `notif.icon`, `notif.post`, `notif.remove`, `notif.sync` and `notif.dismiss`. Validation order is unchanged up to step 7; at step 8 they skip the duplicate check.

`notif.action` is the one exception that is **acked** (the user pressed something and wants to know), and so goes through the normal de-duplication, but it is **never queued and never re-sent**: a reply that arrived twice must never be typed twice. If there is no ack in 5 seconds, the sender tells its user "no answer" and does **not** send it again.

### 3.3 Messages

| Type | Direction | Fields | Receiver does |
|---|---|---|---|
| `notif.status` | phone → PC | `id`, `enabled`, `access`, `apps`, `otpHidden` (optional) | Remembers it for the PC's Settings. Sent after each accepted `hello` and whenever it changes (at most once per 5 s) |
| `notif.icon` | phone → PC | `id`, `hash`, `png` | Verifies, caches `<hash>.png`, no reply |
| `notif.post` | phone → PC | `id`, `nid`, `app`, then optional fields below | Shows a toast, or replaces the toast with the same `nid` |
| `notif.remove` | phone → PC | `id`, `nid` | Removes the toast with that `nid` |
| `notif.sync` | phone → PC | `id`, `nids` | Removes this phone's toasts whose `nid` is not in the list |
| `notif.dismiss` | PC → phone | `id`, `nid` | `cancelNotification` for that notification (setting permitting) |
| `notif.action` | PC → phone | `id`, `nid`, `i`, `label`, `text` (optional) | Fires the action (or the reply with `text`), then `ack`, or `error` |

**Field types.** All strings are cleaned by the **receiver** with the five steps of `link.title` ([protocol.md "Links: link"](../protocol.md), step list: bidi controls → `_`, control characters → space, collapse spaces, cut to N code points, empty → absent). Lengths are in code points.

| Field | Type and rules |
|---|---|
| `nid` | String, exactly 32 characters from `0-9a-f`. = `hex(SHA-256(UTF-8(notification key)))[0..32]`. Example: key `0|com.example.chat|42|null|10123` → `54892d1f759b11f6762ff1bd98cda7df` |
| `app` | Required string, the app's label, cut to **40** |
| `iconHash` | Optional, 32 `0-9a-f`. The `hash` of a `notif.icon` sent earlier on this connection |
| `title` | Optional, **150** |
| `text` | Optional, **500** |
| `bigText` | Optional, **1000**. The PC shows it instead of `text` when it is longer |
| `subText` | Optional, **80** |
| `conversation` | Optional, **100**: the conversation title of a MessagingStyle notification |
| `group` | Optional boolean, default false: a group conversation |
| `messages` | Optional array of **at most 5** objects `{sender?: string (60), text: string (300)}`, oldest first. A missing `sender` means "You" (the user's own message). A message with no text is not sent: the sender writes "[Photo]" (or "[Attachment]") for media-only messages |
| `lines` | Optional array of at most 5 strings of 150 (InboxStyle) |
| `importance` | Optional integer 2 to 5 (Android's `IMPORTANCE_LOW`=2 … `HIGH`=4, 5 = urgent), default 3 |
| `silent` | Optional boolean. The phone says "don't pop up": the PC adds the toast to the Action Center without a popup (`SuppressPopup`) |
| `sensitive` | Optional boolean. No preview on the PC (the existing `sensitive` semantics of `text`) |
| `redacted` | Optional boolean. Android hid the content (4.9). The PC shows "Code hidden by Android" |
| `code` | Optional string, **4 to 10 characters from `0-9A-Z-`**, no other character; anything else is ignored (no button), the notification is still shown. The one-time code found by the phone (4.10) |
| `reply` | Optional object `{i, hint?}`: `i` an integer 0 to 31 (the action's index in the phone's list), `hint` a string of 60 |
| `actions` | Optional array of **at most 3** `{i, label}`: `i` an integer 0 to 31, `label` a string of 30 |
| `enabled`, `apps`, `access`, `otpHidden` (status) | `enabled` boolean: the phone is mirroring now (switch on **and** access granted). `apps` integer: how many apps are allowed. `access` is `"granted"` or `"missing"`. `otpHidden` boolean: Android 15+ will hide codes from Kin |
| `hash`, `png` (icon) | `hash` = `hex(SHA-256(png bytes))[0..32]`; `png` = base64url (no padding, the project's one encoding) of a **PNG of 24 to 128 pixels, square, at most 32,768 bytes** |
| `nids` (sync) | Array of at most 200 `nid` strings |
| `i`, `label`, `text` (action) | `i` as above. `label` is the label the PC was shown (30). `text` is present **if and only if** the action is a reply; 1 to **2000** characters after trimming |

A `notif.post` with no `title`, `text`, `bigText`, `messages`, `lines` and not `redacted` is `bad_request`. A message whose frame is over **32 KiB** is `error too_large` (senders stay under 16 KiB: they drop `bigText`, then extra `lines` and `messages`, to fit).

**What the PC shows for a post.** The PC must never put more in a toast than "title, up to two body lines, the attribution, the icon and the buttons", and must keep the toast XML at or under **4,800 bytes** (the documented limit is 5 KB **[M]**): it cuts `bigText`/`text`, then older messages, until it fits.

### 3.4 `notif.post` semantics

- **Upsert by `nid`.** A post for a `nid` that is on screen **replaces** it (same tag and group on the toast). A post never "adds" a second toast for one `nid`.
- `silent` is how the phone's own priorities carry over: it is true for `importance` 2, for an update of a notification that has `FLAG_ONLY_ALERT_ONCE`, and for a notification the phone's Do Not Disturb filter blocked (`!Ranking.matchesInterruptionFilter()`). The PC also treats `importance` ≤ 2 as silent.
- Buttons: the PC shows the reply box with a **Send** button when `reply` is present, **Copy** when `code` is valid, then `actions`, **at most 5 `<action>` elements in all** (Windows' limit **[M]**). Priority: Send, Copy, then the phone's actions in order. A `sensitive` or `redacted` post shows **no** reply and no actions (only Windows' own dismiss).
- The phone never sends a `reply` or `actions` entry for an action it can't fire (4.7).

### 3.5 Ordering, rate limits, dedupe

All on one WebSocket in order, so:

- **Phone sender (single coroutine, one outgoing queue).** A `notif.icon` goes out before the first `notif.post` that uses its hash **on this connection** (the PC keeps the file, so a repeat is a no-op; icons are re-sent once per connection, at most ~64 × 32 KiB). A `notif.remove` cancels a pending, not-yet-sent post for the same `nid` and always goes out.
- **Per-`nid` coalescing:** at most one post per `nid` per **1 s**; a burst sends the **last** state when the second ends (trailing). A post whose content hash (title, text, bigText, subText, messages, lines, action labels, code, flags) equals the last one sent for that `nid` is **not sent**.
- **Global budget (phone):** token bucket, capacity 10, refill 1 per 500 ms, for posts. Over budget → the post is dropped and counted. Removes are exempt up to 50 per 10 s.
- **Per package (phone):** at most 20 posts per 60 s; over that → dropped and counted (a very chatty app can't use the whole budget).
- **PC receiver:** token bucket per phone (review round 1: it survives a reconnect), capacity 20, refill 1 per second, for posts; over → dropped and counted; one `error` `rate_limited` (no `ref`) per 10 s at most. An identical payload for a `nid` within 2 s is ignored (no new popup). The PC never keeps more than **30** toasts of one phone: it removes the oldest first (Windows has its own per-app cap in the Action Center, 20 by default **[U]**).
- **`notif.action` limits:** at most 5 per 10 s from the PC's side, 10 per 10 s accepted by the phone (`error rate_limited`).

### 3.6 Offline

- No connection, no accepted `hello`, or a peer without `notif`: **drop**. Nothing is stored (no table, no file). The phone counts `dropped_offline`.
- **On every connection, after both `hello`s are accepted** (and the PC lists `notif`), the phone sends `notif.status`, then `notif.sync` with the `nid`s of the notifications that are **currently active, allowed and not skipped** (computed from `getActiveNotifications()`: a one-off call, nothing is cached). It sends **no content**: pre-existing notifications are not shown (a stale toast is worse than none). The PC removes its toasts for that phone that aren't in the list: that cleans up what was removed on the phone while the link was down. **This also works after a Kin restart**, because the toasts, their tags and groups are in Windows' own history (`GetHistory`).
- The phone also sends `notif.remove` for every active mirrored notification when the user turns mirroring off or removes an app from the list.

### 3.7 PC → phone

**Dismiss.** `notif.dismiss {nid}`. The phone resolves `nid` to a key (4.8) and calls `cancelNotification(key)` if the listener is connected, the user's setting "Dismiss on the phone too" is on (default on, a phone setting) and the notification is clearable. No reply either way. The phone remembers `nid` for 5 s so the `onNotificationRemoved` that follows doesn't echo a `notif.remove`.

**Action.** `notif.action {nid, i, label, text?}`:

1. The phone looks the notification up (4.8). Not found → `error not_found`.
2. Mirroring off, access lost, or the app no longer allowed → `error disabled`.
3. `actions[i]` missing, or its cleaned title ≠ `label` → `error stale` (the app changed the notification; never press a different button).
4. A reply (`text` present) needs `actions[i]` to have a free-form `RemoteInput`; otherwise `error bad_request`. A non-reply action with a `RemoteInput` → `bad_request`.
5. It fires the `PendingIntent` (4.7). Sent without an exception → **`ack`**. `CanceledException` or any other failure → `error action_failed`.

New `error` codes (for the table in protocol.md): `stale`, `action_failed`, `disabled`, `rate_limited`. An unknown code is already "a failure" for old peers.

### 3.8 Sensitive handling

- The phone sets `sensitive` in exactly two cases (no other guessing): the app is on the flagged list (decision 5, 4.3), or the notification's effective visibility is `VISIBILITY_SECRET` and the user allowed hidden notifications for that app (4.4, rule 10).
- **A `sensitive` post** is shown as "<app> · <phone>" and the text "Private notification", with no preview, no reply, no buttons, no code button. It is never logged with content (nothing is). Its toast expires after 10 minutes and on reboot (5.5).
- **A one-time code is by nature sensitive**: the toast shows the code (that is the point), so a post with a valid `code` also expires after **10 minutes**, and Copy clears the clipboard after 60 s (5.7).
- The PC's own setting "Hide notification content" (default off) turns every post into the sensitive form.

### 3.9 Cleaning and validation (receivers)

`bad_request` (with `ref`, connection stays open, id not remembered) for: a missing or invalid `nid`, `app`, `id`; a `nid`/`hash`/`iconHash` that isn't 32 lowercase hex; a wrong type; `importance` outside 2 to 5; an `actions`/`reply` index outside 0 to 31; the empty post above. **Over-long** strings are cut (cleaned), never rejected, except the 32 KiB frame rule. `code` that doesn't fit is dropped, not an error. `notif.icon` with a wrong hash, a non-PNG, a bad size or over 32 KiB is dropped silently and counted (so a bad icon can't make an error storm).

### 3.10 What changes in the existing docs and code (desktop coder does these)

- `docs/protocol.md`: Messages table (7 rows), Field types, a "Notifications" section holding 3.1 to 3.9, "Message IDs and acks" (the ephemeral class and the `notif.action` exception), Versioning (`notif`), Error codes (4 new), changelog 56.
- `desktop/src-tauri/src/messages.rs`: `CAPABILITIES` becomes dynamic (a function of the setting). `ws.rs` sends a fresh `hello` when the setting changes.
- Android `Messages.kt`: `hello(... capabilities = listOf(link, notif))`.

### 3.11 Vectors to add (the desktop coder owns `protocol/`)

New file **`protocol/notif.json`** and new `notif-*` cases in `messages.json`, with a `scope-2b.2` group in `check-vectors.ts` and a section in `protocol/README.md`.

| Array | What it pins | Used by |
|---|---|---|
| `nid[]` | `key` → `nid` (include the two examples in this document: `0|com.example.chat|42|null|10123` → `54892d1f759b11f6762ff1bd98cda7df`, `0|com.whatsapp|1|null|10123` → `36460390475e793c9a9e06dd5e2d421f`; keys with a non-ASCII tag and an empty tag) | both |
| `iconHash[]` | PNG bytes (base64url) → `hash` | both |
| `clean[]` | per field: input (bidi controls, newlines, long, emoji at the cut) → output, with the limits of 3.3 | both (PC cleans; the phone's own encoder must not exceed them) |
| `post[]` | valid and invalid `notif.post` objects → accepted/rejected + the reason (`bad_request`, drop) | both |
| `code[]` | `code` strings → valid (the PC's check): `123456`, `G-123`, `AB12-CD`, lowercase, 11 chars, `12 34` (space), empty | both |
| `otpDetect[]` | `{title, text, bigText?, personHasContactUri?, expect: code or null, note}` — the **phone's detector** (4.10). Include every language of the keyword table with at least 1 positive, plus all the negatives of 4.10 | Android (the PC doesn't detect) |
| `redactedText[]` | strings Android uses as the placeholder per locale → `redacted: true`, plus lookalikes that must be `false` | Android |
| `skip[]` | a notification snapshot (flags, category, visibility, importance, own package, allowed or not, summary, ongoing, local-only, media, call, progress, empty) → `send` or the skip reason | Android |
| `pickActions[]` | a list of actions (title, semantic action, remote inputs, `isActivity`) → the chosen `reply` and `actions[]` | Android |
| `rate[]` | a timeline `(t_ms, nid, package, kind)` → accepted/dropped for **both** limiters (phone budget, PC bucket) and the coalescing | both |
| `toastIds[]` | `(phoneShortId, nid)` → `tag`, `group`, and the `launch`/`arguments` strings (5.3); strictly-parsed bad arguments → rejected | PC |
| `messages.json` | `notif-post-minimal`, `notif-post-full`, `notif-post-bad-*`, `notif-action-ack`, `notif-action-error-*`, sequences: `notif-dropped-when-feature-off-is-silent`, `notif-not-sent-without-capability`, `notif-capability-changes-with-a-new-hello`, `notif-action-is-not-resent-after-reconnect`, `notif-sync-removes-the-rest`, `notif-is-not-in-the-dedup-memory` (a repeated `id` is processed twice) | both |

## 4. Android

### 4.1 Pieces

| Piece | Type | Job |
|---|---|---|
| `notifmirror/MirrorListenerService` | `NotificationListenerService` | Declared in the manifest with `android:permission="android.permission.BIND_NOTIFICATION_LISTENER_SERVICE"` and the `android.service.notification.NotificationListenerService` intent filter. `onNotificationPosted`, `onNotificationRemoved(sbn, ranking, reason)`, `onListenerConnected`, `onListenerDisconnected` (calls `requestRebind` once) |
| `NotifSnapshot` | plain data class | Everything the rules need, so they are JVM-testable without Android classes |
| `SbnMapper` | Android-bound, thin | `StatusBarNotification` + `Ranking` → `NotifSnapshot` (and the `Notification.Action` handles kept in memory) |
| `MirrorFilter` | pure | The skip rules of 4.4. Returns `Send` or a reason |
| `MirrorPolicy` / `AllowListStore` | DataStore | Master switch, allowed packages, packages allowed to show hidden-on-lock-screen notifications. **DataStore, not Room**, so this milestone doesn't touch the database version |
| `AppCatalog` | Android | The candidate apps for the picker and their class (messaging, flagged, other) |
| `NotifEncoder` | pure | `NotifSnapshot` → the wire fields of 3.3 (limits, `pickActions`, `code`, flags) |
| `OtpDetector`, `RedactionMatcher` | pure | 4.9 and 4.10 |
| `MirrorSender` | pure + a fake clock | The queue, coalescing, dedupe, budget, icon-before-post of 3.5. Talks to the link through one small interface |
| `ActionRunner` | Android | Resolves a `nid` to a live notification and fires an action or reply (4.7) |
| `MirrorSetupScreen`, `AppPickerScreen` | Compose | Onboarding and the list (4.2, 4.3) |
| `RestrictedSettingsHelp` | Compose + logic | 4.2 |

Wire into the existing link: `LinkSession` sends `notif.status` and `notif.sync` after the accepted `hello`, `IncomingHandler` handles `notif.dismiss` and `notif.action`, `Messages.kt` and `MessageReceiver.kt` get the new types. **`MirrorSender` is the only thing that writes `notif.*` to the socket**, and it checks the peer's capability (3.1) per message.

**Process lifetime.** The system binds the listener once access is granted. It is the same process as `LinkService`; `MirrorSender` uses whatever link session exists and **drops when there is none** (it never starts `LinkService` itself: a background start of a foreground service isn't allowed on newer versions).

### 4.2 Onboarding and "Restricted setting"

**Screens (opt-in, reachable from Settings → "Show notifications on your PC" and offered once after the setup):**

1. **What it does.** Notifications of the apps you pick go to your PC's screen and nowhere else. Kin keeps none of it; replying and dismissing work from the PC. Off by default.
2. **Pick apps** (4.3). Nothing is read before access is granted, and nothing from an app that isn't ticked is ever read.
3. **Give access.** One button, "Open notification access". The intent is `Settings.ACTION_NOTIFICATION_LISTENER_DETAIL_SETTINGS` on API 30+ with `Settings.EXTRA_NOTIFICATION_LISTENER_COMPONENT_NAME` set to the service **[M]**, and `Settings.ACTION_NOTIFICATION_LISTENER_SETTINGS` on API 29 and as the fallback if the first isn't resolvable. On return, the state is read again.
4. **Result.** Granted: done. Not granted: show the restricted-settings help below (the screen always does this when access isn't granted after the user comes back, because Android gives no direct signal for "restricted").

**What "Restricted setting" is.** Android 13 added "restricted settings": an app installed **outside an app store** can't be given the notification-listener (or accessibility) special access until the user allows it **[D: Android Authority, CleanBrowsing, secondary sources; Android doc search]**. Android decides by the **installer**: an app installed through the **session-based** installer API (what stores use) isn't restricted; one installed by a browser, a file manager or a messaging app is **[D]**. Android 15 widened the same protection to more special permissions (notification listener, device admin, display over other apps, usage access, …) **[D: 9to5Google]**. The Companion Device Manager javadoc also says "Side-loaded apps must allow restricted settings before requesting notification access" **[V: AOSP `CompanionDeviceManager.java`]**, so there is no association shortcut around it.

**Exact steps to show the user** (the screen shows them as a numbered card, with a "Open app info" button that opens `ACTION_APPLICATION_DETAILS_SETTINGS` for Kin):

1. In Notification access, tap **Kin** (or the switch). If Android shows **"Restricted setting"**, tap **OK**. This step matters: the menu item of step 3 usually doesn't exist until the first blocked attempt **[M; reported by user guides, to be confirmed]**.
2. Open **Settings → Apps → Kin** (long-press the Kin icon → App info is the same).
3. Tap **⋮** (top right) → **Allow restricted settings**, and confirm with your PIN or fingerprint.
4. Go back to **Notification access** and turn **Kin** on, then confirm Android's own question.

Phones differ (Samsung, Xiaomi, others rename the screens); the text says "the menu is on the app's info page".

**How to detect it.** There is no documented public API for "this app is restricted" [D: the searches found none]. So, in this order:

1. **Granted** = `NotificationManager.isNotificationListenerAccessGranted(ComponentName)` (API 27) is true. This is the only thing the code relies on to **turn mirroring on**.
2. **Likely restricted (a hint only, never a gate)**: API 33+ and `PackageManager.getInstallSourceInfo(packageName)` shows no installer, or a package-installer/file-manager package, rather than a store. Shown as a friendly heads-up on screen 3 ("Android may ask for one extra step"). **[M for the API; U for how well it predicts]**
3. **Optional probe [U]:** `AppOpsManager.unsafeCheckOpNoThrow("android:access_restricted_settings", uid, packageName)` returning `MODE_IGNORED`/`MODE_ERRORED`. The op name is from memory; wrap it in `try/catch` and treat any failure as "unknown". If it works on the test images, use it to choose between "Allow restricted settings" help and "just turn it on".
4. **After the user returns and access is still not granted**: show the steps card. That covers every case where 2 and 3 are wrong.

**Emulator note.** `adb install` uses a session, so the emulator build probably *isn't* restricted: the restricted flow **can't be reproduced there** without extra work. The coder tries `adb shell appops get app.kinrelay.android ACCESS_RESTRICTED_SETTINGS` and `appops set … ignore` **[U]**; if that doesn't work, the flow is checked by the user on a real sideloaded install at v0.2.0 (TESTING.md).

### 4.3 The per-app allow list

**Default: every app off.** The picker **pre-selects** only apps that are installed **and** in the bundled messaging table (WhatsApp and Business, Telegram and Telegram X, Signal, Messenger and Messenger Lite, Google Messages, Samsung Messages, Viber, LINE, WeChat, Discord, Slack, Microsoft Teams, Skype, Google Chat). Everything else is shown unchecked, in a second section.

**Where the list of apps comes from (without `QUERY_ALL_PACKAGES`).**

- Add to `<queries>` one entry for `ACTION_MAIN` + `CATEGORY_LAUNCHER` (`<intent><action android:name="android.intent.action.MAIN"/><category android:name="android.intent.category.LAUNCHER"/></intent>`). It makes every app with a launcher icon visible, which is the usual way to list "the apps the user knows", and is **not** the restricted `QUERY_ALL_PACKAGES` **[M]**. Play may still ask why in review: recorded as R27.
- Apps that posted a notification while access was on (their package name is in the `StatusBarNotification` even when not visible) are added to the list too; their label falls back to the package name if `getApplicationInfo` isn't allowed (**verify on API 29 and 37 whether it throws** — [U] — and record it in the results).

**How an app is classified** (`AppCatalog`, pure table plus rules, so it is unit-testable):

| Class | Rule | Picker |
|---|---|---|
| **Messaging** | Package in the bundled table | Pre-selected, normal |
| **Flagged** | Package in the bundled flagged table (authenticators: Google Authenticator, Microsoft Authenticator, Authy, 2FAS, Aegis, FreeOTP, Duo; password managers: Bitwarden, 1Password, LastPass, Dashlane, KeePass-family, Proton Pass; wallets and payments: Google Wallet, PayPal, Venmo, Coinbase) **or** a word of the label or of a package segment matches `bank`, `banco`, `banque`, `credit`, `wallet`, `vault`, `otp`, `authenticator`, `2fa`, `password`, `passwd`, `keepass`, `crypto`, `invest`, `trading`, `finance`, `pay` (whole words) | **Off, with a ⚠ "Likely sensitive"**. Turning it on asks: "Notifications from <app> may contain codes, balances or passwords, and they'll appear on your PC. Turn it on?" If allowed, **always mirrored as `sensitive`** (open question Q1) |
| **Other** | Everything else | Off |

`ApplicationInfo.category` has no finance value (it has `CATEGORY_SOCIAL`, `NEWS`, `PRODUCTIVITY` … **[M]**), so banking can only be flagged by the heuristic above; the real safety is that **an unknown app is off by default**. The heuristic only adds a warning and the forced `sensitive`.

Each allowed app also has one checkbox, **"Also show notifications I've hidden on the lock screen"** (default off). The list can be changed any time (Settings → Show notifications on your PC → Apps).

Stored: the set of allowed package names and the set of "lock-screen hidden allowed" package names, in DataStore. Nothing else.

### 4.4 Skip rules

`MirrorFilter` applies these **in this order and stops at the first match**. The first three run on cheap fields only, **before any extra of the notification is read** (a notification from a non-allowed app is never inspected):

| # | Rule | Reason code |
|---|---|---|
| 1 | `sbn.packageName` is Kin's own (debug builds: except the debug test channel) | `own` |
| 2 | The package isn't in the allow list | `not_allowed` |
| 3 | `sbn.user` isn't the current user (work profile) | `other_profile` |
| 4 | `FLAG_ONGOING_EVENT`, `FLAG_FOREGROUND_SERVICE` or `FLAG_NO_CLEAR` (`!sbn.isClearable`) | `ongoing` |
| 5 | `FLAG_GROUP_SUMMARY` | `summary` |
| 6 | `FLAG_LOCAL_ONLY` ("relevant to this device only, not recommended to bridge to other devices") | `local_only` |
| 7 | Category `CATEGORY_TRANSPORT`, `MediaStyle`, or `EXTRA_MEDIA_SESSION` present | `media` (2B.3 owns these) |
| 8 | Category `PROGRESS`, `SERVICE`, `SYSTEM`, `STATUS`, `ALARM`, `CALL`, `NAVIGATION`, or a determinate progress bar (`EXTRA_PROGRESS_MAX` > 0, not indeterminate) | `category` |
| 9 | `Ranking.getImportance()` ≤ `IMPORTANCE_MIN` (1), or the ranking says the app is suspended | `importance` |
| 10 | Effective visibility is `VISIBILITY_SECRET` and the app isn't ticked for hidden notifications. Effective = `Ranking.getLockscreenVisibilityOverride()` when it isn't `VISIBILITY_NO_OVERRIDE`, else `Notification.visibility`. **`getLockscreenVisibilityOverride()` is API 31, not 28:** on Android 10 and 11 only `Notification.visibility` is read | `secret` |
| 11 | No title, text, big text, messages or lines, and not redacted | `empty` |

If it passes: the post is `sensitive` when the app is flagged or when rule 10 matched and the user allowed it. `silent` per 3.4. Rules 7 and 8 mean Kin never mirrors calls or alarms in 0.2.0 (recorded in the open questions as Q5).

`onNotificationRemoved`: send `notif.remove` for every `reason` **except** `REASON_LISTENER_CANCEL` (10) and `REASON_LISTENER_CANCEL_ALL` (11) (Kin's own cancel from a PC dismiss) and `REASON_GROUP_OPTIMIZATION` (13) and `REASON_UNAUTOBUNDLED` (16) (the system is regrouping, and re-posts). A tap on the notification (`REASON_CLICK`) removes the toast: the user has read it on the phone.

### 4.5 MessagingStyle and the other styles

- **Messaging:** `NotificationCompat.MessagingStyle.extractMessagingStyleFromNotification(notification)` (androidx.core, already a dependency through `core-ktx`; it reads `EXTRA_MESSAGES`, `EXTRA_CONVERSATION_TITLE`, `EXTRA_IS_GROUP_CONVERSATION`, `EXTRA_SELF_DISPLAY_NAME`) **[M]**. `conversation` = the conversation title; `group` = `isGroupConversation`; `messages` = the **last 5** by timestamp, oldest first; `sender` = `person.name`, omitted for the user's own message. Only the text is used: a `dataUri` is never read, and a media message with no text becomes "[Photo]" (mime `image/*`) or "[Attachment]". `title` is the conversation title or the last sender.
- **Big text:** `EXTRA_BIG_TEXT` → `bigText`. **Inbox:** `EXTRA_TEXT_LINES` → `lines` (last 5). Plain: `EXTRA_TITLE`, `EXTRA_TEXT`, `EXTRA_SUB_TEXT`.
- **Never read or sent:** the large icon (a contact's photo), pictures (`BigPictureStyle`), `EXTRA_PEOPLE`, `Person.uri`, `Person.key`, `Bubble` metadata, custom views.

### 4.6 Icons

- **App icon:** `PackageManager.getApplicationIcon(pkg)` drawn to a **96 × 96** ARGB bitmap (adaptive icons: the layered drawable drawn as is), compressed to PNG; if over 32 KiB it is redrawn at 64 × 64. `hash` = the first 32 hex of its SHA-256. Cached in memory and in `cacheDir/mirror-icons/<hash>.png`, keyed by package and its `longVersionCode` so an app update refreshes it.
- **Fallback when the app isn't visible [U]:** the notification's small icon (`Notification.getSmallIcon().loadDrawable`, a white glyph, **verify that it loads for another package**) drawn white on a circle of `Notification.color` (grey if 0).
- Sent as `notif.icon` per connection before first use (3.5). Never sent: contact avatars (privacy and size).

### 4.7 Reply and actions

**What is offered (`pickActions`).** From `Notification.actions`:

- **Reply:** the first action that has a `RemoteInput` with `allowFreeFormInput` (prefer `SEMANTIC_ACTION_REPLY`). Needs its `PendingIntent`. `reply = {i, hint = remoteInput.label}`.
- **Buttons (at most 3):** other actions with a non-empty title, **no** remote input of any kind, and **whose `PendingIntent` doesn't start an activity**: on API 31+ `PendingIntent.isActivity()` is false (broadcast or service), on API 29 and 30 (no such call) the action is **kept** and the failure is reported if it comes (see below). Dropped: actions that need choices only, contextual/system-generated actions.
- **None of it** for `sensitive`, `redacted` posts.

**Why activity actions aren't offered.** Android restricts starting an activity from the background. The documented exceptions are a visible window, being the IME, a `PendingIntent` sent by the system, `SYSTEM_ALERT_WINDOW`, `START_ACTIVITIES_FROM_BACKGROUND`, a service bound by an app with that permission, the launcher, and core OS parts. **A `NotificationListenerService` is not in the list** **[D: developer.android.com background-starts]**. From Android 14 a sender must opt in for its own privileges, and from Android 15 the creator of the `PendingIntent` must opt in to share its privileges **[D]**, and a blocked start is **silent** (only a log "Background activity launch blocked!"). Kin has no privilege to pass on and doesn't want one ("no display over other apps"). So an action that opens an activity ("Open", "View") is simply not shown on the PC. Reply and mark-as-read are broadcasts or services in practice **[M]**, so they work.

**Firing.**

- **Reply:** `RemoteInput.addResultsToIntent(arrayOf(remoteInput), intent, results)` with `results[resultKey] = text`, and `RemoteInput.setResultsSource(intent, SOURCE_FREE_FORM_INPUT)`, then `action.actionIntent.send(context, 0, intent)`. **[M]**
- **Button:** `action.actionIntent.send()`.
- `PendingIntent.CanceledException` (the notification was cancelled) → `error action_failed`. **API 29/30 or a blocked activity start:** nothing throws, so the reply is "sent" by Kin's `ack` but nothing happens. That is a known limit; the PC doesn't claim success beyond "sent to the phone".
- No `ActivityOptions` opt-in is passed (see above).

**What the PC is told.** `ack` when `send` returned; `error` codes of 3.7.

**No content is logged**: not the label, not the reply text, not its length.

### 4.8 Looking up a notification by `nid`

`MirrorSender` keeps an **in-memory** map `nid → key` for the notifications it announced (cleared on process death, never persisted). `ActionRunner` first looks in the map, and if the entry is missing (the process was restarted, or a PC toast outlived it) it **recomputes**: `activeNotifications` → `sha256(key)[0..32]` for each, and takes the match. `activeNotifications` is a one-off call at action time.

### 4.9 Android 15+ redaction of one-time codes

**What is documented [D].** "Android will stop untrusted apps that implement a `NotificationListenerService` from reading unredacted content from notifications where an OTP has been detected. Trusted apps such as companion device manager associations are exempt from these restrictions." (Android 15 behaviour changes, Security and privacy → OTP redaction.) The page gives no list of fields and no API to detect it.

**What reports add [D: Android Authority, via search; marked secondary].**

- Detection is done by Android System Intelligence before the notification reaches listeners; a detected two-factor notification is marked sensitive.
- Content shown to an untrusted listener is replaced by a localized placeholder (reported text: "Sensitive notification content hidden").
- "Trusted" = the listener holds the permission `RECEIVE_SENSITIVE_NOTIFICATIONS` (`role|signature`). Of the roles that grant it, only **COMPANION_DEVICE_WATCH, COMPANION_DEVICE_GLASSES and HOME** can be held by third-party apps. The same article says Phone Link was hit by this in 2024 and shipped a workaround **that doesn't work on all devices**.
- Android 16 also redacts OTPs **on the lock screen** in higher-risk situations. That is about the lock screen and doesn't change what a listener gets.

**Is there a legitimate exemption for Kin? Not in 2B.2.**

- The role route means impersonating a watch (the user would be asked to link "a watch"). That's not honest, so **no**.
- The Companion Device Manager route: a CDM association is exempt, but it needs a device the system can discover or a **self-managed** association (API 33+, `REQUEST_COMPANION_SELF_MANAGED`) and a device profile. `REQUEST_COMPANION_PROFILE_COMPUTER` exists as a permission **[V: seen in the AOSP javadoc list of `associate`]** but I found **nothing** that says a computer profile grants notification access or the sensitive-content exemption **[U]**. And **sideloaded apps still need "Allow restricted settings" first** **[V]**. So: **a research item after v0.2.0** (Q4), not a 2B.2 task, and **no hacks** (no accessibility scraping, no reading the screen).
- So the product behaviour is: detect it and say so.

**Detecting redaction (`RedactionMatcher`).** API 35+ only (below that, always `false`). There is **no documented flag**, so:

1. The text, big text, or a message of the notification equals (after trimming, case-folded) a known placeholder. The coder takes the translations of the system string for "sensitive notification content hidden" from the AOSP resources (`core/res/res/values-*/strings.xml`; look for the string whose English value is "Sensitive notification content hidden") into `RedactedStrings.kt` **[U: the exact resource name and the full list]**.
2. If so, the post is sent with `redacted: true`, **no `text`/`bigText`/`messages`**, and the `title` only if present (unknown whether Android keeps it **[U]**), and **no** `code`.
3. The phone shows a one-time note in the setup: "On Android 15 and later, Android hides verification codes from apps like Kin. The PC will say 'Code hidden by Android'."
4. `notif.status.otpHidden` = (API ≥ 35).

**Verification on the emulator (the task text asks for it).** The coder tries to produce a redacted notification on `kin_api37` by posting a notification with an obvious OTP through `cmd notification post` and reading what the listener sees. **If the emulator image has no Android System Intelligence, no redaction happens and nothing can be learned there**; in that case the results file says so, and `RedactionMatcher` is covered by fixtures (the string table) only, and the real behaviour is left for the user's Android 15+ phone (TESTING.md, "Pixel with Android 15+").

### 4.10 One-time code detection (`OtpDetector`, phone only)

Pure function `detect(title, text, bigText, hasContactSender) → String?`. The vectors `otpDetect[]` pin it.

**Preparation:** join title, text and big text with `\n`. Fold Unicode decimal digits (`Nd`: Arabic-Indic, Devanagari, fullwidth…) to ASCII `0-9`. Lowercase with `Locale.ROOT` for the keyword match (the CJK keywords are matched as substrings, the rest on word boundaries). **Total length over 400 characters → no code** (codes come in short messages).

**Gate 1, keyword.** At least one keyword from the table must be present. A message with a number and no keyword is never a code. The table lives in one file, `OtpKeywords.kt`; the starting list (extend through vectors, never by removing a negative):

| Languages | Keywords |
|---|---|
| English | code, otp, passcode, pass code, verification, verify, one-time, one time, security code, login code, sign-in code, sign in code, 2fa, two-factor, two factor, pin, confirmation code |
| Spanish / Portuguese | código, codigo, verificación, verificacion, verificação, clave, contraseña de un solo uso, senha, confirmación, confirmação |
| French | code, vérification, verification, confirmation, mot de passe à usage unique |
| German | code, bestätigungscode, verifizierung, sicherheitscode, einmalpasswort, tan |
| Italian / Dutch | codice, verifica, conferma, password monouso / verificatie, bevestigingscode |
| Nordic / Polish | kod, bekräftelsekod, verifieringskod, engangskode, weryfikacyjny, jednorazowy |
| Turkish / Russian / Ukrainian | kod, doğrulama, onay kodu, tek kullanımlık / код, подтверждения, проверочный, одноразовый / код, підтвердження |
| Arabic / Persian / Hindi | رمز, رمز التحقق, كلمة مرور لمرة واحدة / کد, کد تایید / कोड, ओटीपी, सत्यापन |
| Indonesian / Vietnamese / Thai | kode, verifikasi, sandi sekali pakai / mã, xác minh, mã xác thực / รหัส, ยืนยัน |
| Japanese / Korean | 認証コード, 確認コード, 認証番号, 確認番号, ワンタイム, コード / 인증번호, 인증 코드, 확인 코드, 일회용 |
| Chinese | 验证码, 校验码, 动态码, 确认码, 驗證碼, 校驗碼, 動態密碼 |

(`otp`, `2fa` and `pin` always on word boundaries.)

**Candidates.**
- **Numeric:** 4 to 8 ASCII digits, or two groups `NNN-NNN`, `NNN NNN`, `NNNN-NNNN`, `NNNN NNNN` (separator removed). The candidate must not be adjacent to another digit or letter, nor be part of a larger number: not preceded by a digit followed by `.` `,` `:` `/` `-` (so `12:30`, `1,234`, `2024-10-17`, `3.14159` are not candidates), not preceded by a currency symbol (`$ € £ ¥ ₹ ₽ ₩`) or `+`, and not followed by `%`, a currency code (`USD`, `EUR` …) or a unit (`km`, `kg`, `min`, `h`). A phone number (a `+` or `(` before it, or two more digit groups after it) isn't a candidate.
- **Alphanumeric:** 5 to 8 characters `[A-Z0-9]` with at least one letter **and** one digit, all letters uppercase, as its own word. A Google style prefix `G-` + digits yields the digits.
- **A 4-digit number from 1900 to 2099** is dropped **unless it is the only candidate**.

**Choosing.**
- No candidate → no code. Exactly one → that.
- Several → the one whose distance to the nearest keyword is smallest, **only if** it is **more than 10 characters** closer than the next one (exactly 10 is not enough: "Verification code 123456 or 654321" has none); otherwise **no code** (a wrong code is worse than none).

**False-positive guards (all must hold):** the notification isn't from a contact (a `MessagingStyle` whose sender has a contact URI → skip, so a friend saying "what's the code 1234?" doesn't count; an SMS from a short code or an unsaved number does); the app isn't a flagged one **unless** it's allowed (it is then `sensitive` anyway); `redacted` is false.

**Negative examples the vectors must contain:** "Your order #483921 has shipped" (no keyword), "Use code SAVE20 for 20% off" (`SAVE20` is a valid alphanumeric candidate next to the keyword `code`, so a promo rule is needed: if `save`, `off`, `discount`, `promo`, `coupon` or `sale` appears within 30 characters of the candidate, the message has **no** code), "Meeting at 12:30, room 4521" (keyword absent), "Your balance is $1,234.56" (amount), "Call 555 123 4567" (phone), "Your 2024 verification code is 482913" (year plus code → `482913`), "Code: 1234 5678 9012" (more than 2 groups → none), "Verification code 123456 or 654321" (two equals → none).

### 4.11 Battery

- **No polling, no timers, no wake locks.** The listener is a system-bound service that is called when a notification changes; Kin does nothing between calls. The coalescing window is a coroutine `delay`, not an alarm.
- Per notification: a cheap filter first (allow list, flags), then extraction and one small JSON frame (1 to 2 KiB) on the **already open** WebSocket. Icons are drawn once per app version.
- `onNotificationRankingUpdate` and `onNotificationRemoved` for non-mirrored notifications are ignored at once.
- **Cost of the binding itself:** Android keeps the process alive and bound while access is on. Kin's process is already held by `LinkService`, so nothing new; if the user turns mirroring off, the app asks the user to switch access off in Settings (it can't revoke it by itself), and Kin ignores everything meanwhile.
- **Doze:** the listener still gets callbacks, but the WebSocket may be unusable until a maintenance window; messages are dropped, which is the specified behaviour (R1 already covers background killing).
- **A measurement is a TESTING.md item** (100 notifications, `dumpsys batterystats` before and after); it can't be meaningful on an emulator.

### 4.12 What the phone stores and logs

- **Stores:** DataStore keys only (`mirror_enabled`, allowed packages, hidden-allowed packages, "setup shown"). An icon cache under `cacheDir`. **No notification content, no `nid` map on disk, no history rows.**
- **Logs:** once per 60 s while active, one line with counts: `mirror: sent N, removed N, dropped {not_allowed: N, ongoing: N, rate: N, offline: N}`. When the allow list changes: `mirror: N apps allowed`. **Never** a title, text, label, reply text, key, `nid`, code or package name of a non-allowed app. `ExportRedactor` needs no new rule if this holds; a test greps a log produced by the e2e run for the test strings.
- **Debug screen:** the same counters, a "Mirror test" section (4.14).

### 4.13 Permissions and manifest

- `<service android:name=".notifmirror.MirrorListenerService" android:exported="true" android:label="@string/mirror_service_label" android:permission="android.permission.BIND_NOTIFICATION_LISTENER_SERVICE">` + intent filter. The label is what the user sees in Notification access.
- `<queries>`: add the `MAIN`/`LAUNCHER` entry (4.3).
- **No new runtime permission.** `POST_NOTIFICATIONS` is unchanged. `docs/android.md`'s permission table gets a row "Notification access (special access, not a manifest permission the user is prompted for)".

### 4.14 The debug build

- A **debug-only own-package override**: notifications on the channel `kin_debug_mirror` from Kin itself are **not** skipped by rule 1 (everything else about them is real). Release builds don't contain it.
- A **debug "Mirror test" screen** (in the Debug screen, debug build only) with buttons that post, from Kin: (a) a MessagingStyle notification with a RemoteInput reply and "Mark as read" (a `BroadcastReceiver` PendingIntent) and "Open" (an activity PendingIntent: must **not** be offered on the PC), (b) a notification with a code ("Your verification code is 482913"), (c) an ongoing one, (d) a group summary plus two children, (e) a `VISIBILITY_SECRET` one, (f) a low-importance one, (g) one that looks like a redacted placeholder.
- `DebugCommandReceiver` gets: `MIRROR_ALLOW --es pkg <p>` (also `com.android.shell`, which is the package of `adb shell cmd notification post`), `MIRROR_ENABLE`, `MIRROR_POST --es kind <a..g>`, `MIRROR_STATE` (counters and the last result of each action, **no content**). Each prints a `CMD …` log line like the existing commands.
- **Granting access in the emulator:** `adb shell cmd notification allow_listener app.kinrelay.android/app.kinrelay.android.notifmirror.MirrorListenerService` **[M; the shell can do this on emulator images; the integration agent confirms on API 29 and 37]**.

## 5. Windows

### 5.1 Pieces (new, desktop coder; names are suggestions)

| Piece | Job |
|---|---|
| `mirror/ledger.rs` | The in-memory ledger: `nid` → `{generation, shown_at, content_hash, has_reply, silent, code?}`. Never persisted |
| `mirror/toast_xml.rs` | Pure: `notif.post` → XML string (escape, 4,800-byte fit, button priority). Built with `XmlDocument` setters, never string concatenation |
| `mirror/surface.rs` | A trait over WinRT (`show`, `remove`, `remove_group`, `history_tags`) plus the Windows implementation and a fake for tests |
| `mirror/activator.rs` | The COM class: `INotificationActivationCallback` and its class factory |
| `mirror/registration.rs` | The HKCU keys (5.2) |
| `mirror/icons.rs` | Cache of `<hash>.png` (5.6) |
| `mirror/otp_clip.rs` | Copy and the 60-second clear (5.7) |
| `mirror/mod.rs` | Receives the messages from `ws.rs`, enforces 3.5, calls the pieces. Settings, status |

Don't touch the Phase 1 `app.rs` toasts, `register_toast_clicks` and `tauri-plugin-notification`. Phase 1 toasts keep their behaviour exactly.

### 5.2 Toasts for an unpackaged app: what is needed

**The question.** Can an unpackaged Tauri app with an AUMID receive toast activation (button, reply input) **when the toast is in the Action Center, or when the process that shows it is not the one that created the toast object**?

**The answer, with evidence.**

- **Not without a COM activator.** Microsoft: "Notifications in the action center can stay around for several days before they are seen by the user and processed. Because of this, you can't assume that your app will be running when the user responds … you will need to create a local COM server that can be invoked when users interact with the toast." **[D: Win32 "Respond to toast activations"]**. `INotificationActivationCallback::Activate` is "called when a user interacts with a toast in the action center" and "in order for your app to respond to toasts in the action center, you need to override this method … and create a shortcut on the start menu" **[D]**. That's exactly what Phase 1 measured (`windows.md`, R10): no activator, nothing after the toast leaves the screen.
- **With a COM activator, yes:** the callback receives the button's `arguments` and the values of the toast's `<input>` elements (`NOTIFICATION_USER_INPUT_DATA` key/value), and if the app isn't running, COM **starts it** through the class's `LocalServer32` registration **[D]**; "if you want to respond to the interactive toast without showing any additional UI, you will need to register a non-UI process as the COM server … replying to a message without bringing your app to the foreground" **[D]** (that exact scenario is inline reply).
- **How an unpackaged app registers (two pieces, both under HKCU, both written by the app itself at each start, like the native-host keys in windows.md):**
  1. **The COM class:** `HKCU\Software\Classes\CLSID\{<our CLSID>}\LocalServer32` = `"<path>\Kin.exe" --toast-activated` **[D: the registry line of the MS page]**, and `CoRegisterClassObject(&CLSID, factory, CLSCTX_LOCAL_SERVER, REGCLS_MULTIPLEUSE)` once the process is the primary instance **[D: "RegisterObjects at startup"]**.
  2. **The app identity with the activator.** MS documents the **Start Menu shortcut** carrying `System.AppUserModel.ID` and `System.AppUserModel.ToastActivatorCLSID` **[D]**. A shortcut-less registration also exists, `HKCU\Software\Classes\AppUserModelId\<AUMID>` with the string values `DisplayName`, `IconUri` and `CustomActivator` = `{CLSID}`. **[V on this PC]**: Cloudflare WARP (`Cloudflare.CloudflareOne.WARP`), Wondershare and MSI Center (`ToastNotificationManagerCompat`, i.e. the Windows Community Toolkit) all have exactly those values under `HKCU\Software\Classes\AppUserModelId`. And the Windows App SDK documents that for unpackaged apps "the calling process will be registered as the COM server and assets like the app display name and icon will be retrieved from the shell and registered as well" **[D]**. **Use the registry form**: it needs no shortcut (the installer's shortcut is in the all-users Start Menu, written by Tauri's template, and the app can't edit it without elevation).
- **Not yet verified for Kin end to end [U]:** that the system really calls our activator for a toast shown under a registry-only AUMID, with Kin both running and not running. That is **spike S1** (section 9), the first thing the desktop coder does, with acceptance tests that need no clicking.
- **Kin's AUMID for these toasts: `app.kinrelay.desktop.phone`** (dev instances: `app.kinrelay.desktop.dev-<key>.phone`, **only registered by a test that removes it again**). A second AUMID means the Phase 1 plugin toasts (`app.kinrelay.desktop`, in-process handlers) can't be affected by registering an activator, and the user gets its own entry in Windows Settings → Notifications ("Kin: phone notifications") to mute these alone.
- `DisplayName` = "Kin: phone notifications". `IconUri` = a PNG of the app icon in `%APPDATA%\Kin\` (written by the app). `CustomActivator` = the fixed CLSID (a random GUID, generated once and written into the source and the doc).

**The activation handler.** `Activate(appUserModelId, invokedArgs, userInput[], count)` runs on a COM thread: it parses `invokedArgs` strictly (5.3), takes `userInput["reply"]` if present, and sends one message to the async runtime. It **returns at once** (the doc: the UI may wait for it). When COM launched Kin only for this call (`--toast-activated` and no other instance), Kin starts normally in the tray and the action runs after the phone connects (up to 10 s; else a toast "Phone not connected, nothing was sent").

**The in-process path too.** Each toast also gets `Activated` and `Dismissed` handlers on the `ToastNotification` object (works while the toast is a popup and Kin is the one that showed it). It is **not known [U]** whether Windows then calls only the COM class or also the event for a popup click. Both routes feed **one** function `on_activation(args, input)` which ignores a repeat of the same `(tag, arguments)` within 3 s.

**A registered activator and the single-instance plugin.** Register the class only after Kin knows it is the primary instance (the second process of a `--toast-activated` launch hands over and exits; if it registered the class first, COM would call into a process that is about to die).

**If S1 fails.** Fallback decided now (Q2's recommendation): when the activator can't be registered, or doesn't work in S1, the toast is built **without** the input box, the buttons, and Copy (text only, the user clicks nothing), and Settings says "Replying from the PC needs a newer Windows setup" (and the risk is recorded). No sparse MSIX, no certificate in 2B.2.

**Windows versions.** The activator interface is documented for Windows 10 (desktop apps only) **[D]**; Windows 11 is the dev PC. The input box and the 5-button limit are from Windows 10 1607 **[M]**.

### 5.3 The toast and its identity

- **Tag** = `nid` (32 characters; Windows allows 64 **[M]**). **Group** = `kin-` + the phone's 8-character short ID. So one phone's toasts can be listed and removed together.
- **`launch`** and every button's **`arguments`** are one string with **no content**: `kin=1&d=<shortId>&n=<nid>&a=<action>[&i=<index>]`, with `a` one of `tap`, `reply`, `copy`, `btn` (and `i` the phone's action index). Parsed strictly (exact keys, `kin=1`, 8-char `d` that must belong to a paired phone that lists `notif`, 32-hex `n`, `i` 0 to 31, a max length of 200). Anything else is refused and logged as a count.
- The toast element is `<toast launch="…" activationType="background">`: a click does **nothing visible** (Kin doesn't come to the front; the button for it is the reply).
- Example for a message with a reply, a code and a button:

```xml
<toast launch="kin=1&amp;d=ABCD2345&amp;n=54892d1f759b11f6762ff1bd98cda7df&amp;a=tap" activationType="background">
  <visual><binding template="ToastGeneric">
    <image placement="appLogoOverride" hint-crop="circle" src="file:///C:/Users/…/AppData/Roaming/Kin/notif-icons/<hash>.png"/>
    <text>Alice</text>
    <text>Are you coming tonight?</text>
    <text placement="attribution">WhatsApp · Pixel 8</text>
  </binding></visual>
  <actions>
    <input id="reply" type="text" placeHolderContent="Reply"/>
    <action content="Send" hint-inputId="reply" activationType="background" arguments="kin=1&amp;d=ABCD2345&amp;n=…&amp;a=reply"/>
    <action content="Copy 123456" activationType="background" arguments="kin=1&amp;d=ABCD2345&amp;n=…&amp;a=copy"/>
    <action content="Mark as read" activationType="background" arguments="kin=1&amp;d=ABCD2345&amp;n=…&amp;a=btn&amp;i=2"/>
  </actions>
  <audio silent="true"/>
</toast>
```

- **Sound:** silent unless the setting "Play a sound for phone notifications" is on (default off; the phone already rang).
- **Properties** set on every toast: `Tag`, `Group`, `ExpirationTime`, `ExpiresOnReboot = true`, `NotificationMirroring = Disabled` (so Windows doesn't mirror it on to another device, which here would be a loop through Phone Link **[M; the property exists in the crate: V]**), `SuppressPopup = silent`.
- **Text layout.** Title line = `title`, else `conversation`, else `app`. Body: for `messages`, the last 2 lines (`sender: text`, no prefix in a 1:1 chat, "You: " for the user's own); else `bigText` or `text`; else `lines`. Attribution: `<app> · <phone name>`. `sensitive`: title = app, body "Private notification". `redacted`: body "Code hidden by Android". Each string comes from `Element.SetInnerText`, so a `<` in a message can't change the structure (a test with a hostile string).
- **No link is made clickable** (toast text isn't linkified), so a link in a message is plain text.

### 5.4 Updating, removing, dismissing

- **Update:** `Show()` with the same tag and group replaces the toast (and Windows pops it up again). That's wanted for a new message. A `silent` post, an identical one, or an update the user can't see use `SuppressPopup = true`, so the Action Center changes without a popup. The ledger keeps a **generation** per `nid`; each toast's handlers carry its generation and **ignore events of a superseded toast**, because replacing a toast raises `Dismissed` on the old object.
- **Remove (from the phone):** `ToastNotificationManager.History().RemoveGroupedTagWithId(tag, group, aumid)` **[V: exists in `windows` 0.62.2]**. The ledger marks `removing` first, so the `Dismissed` event that may follow isn't taken for the user.
- **`notif.sync`:** `History().GetHistoryWithId(aumid)` lists the toasts Windows still holds (also after a Kin restart); each of the group whose tag isn't in `nids` is removed.
- **Dismiss from the PC to the phone.** Only `Dismissed` with `ToastDismissalReason::UserCanceled`, and only if the ledger doesn't say `removing` or superseded, and the setting "Dismiss on the phone too" is on, sends `notif.dismiss`. `TimedOut` (the popup went to the Action Center) and `ApplicationHidden` send nothing **[D: the three reasons are in `tauri-winrt-notification`'s docs and the WinRT enum]**.
- **Known gap [U]:** clearing an entry **from the Action Center** may not raise `Dismissed` in our process, and objects of toasts shown before a Kin restart have no handlers. v0.2.0 does **not** poll the history to guess ("a wrong guess cancels a real notification"). TESTING.md has a check that tells the user's real result; if clear-from-Action-Center turns out not to be detected, this is a documented limit, and a later version can add a polling watcher behind a setting.
- Unpair: `RemoveGroupWithId` for that phone's group and its icons when no phone is left.

### 5.5 Expiry (what Windows keeps)

Windows holds an Action Center toast, **including its text**, in its own notification store until it expires or is removed (the Notifications database under the user's profile **[M, U: path and format not checked]**). Kin can't change that, so it shortens it: `ExpirationTime` = now + **8 h** (normal), **2 h** (`silent`), **10 min** (`sensitive`, `redacted`, or with a `code`), and `ExpiresOnReboot = true` for all. The values are constants in one place. The honest sentence for docs: "A mirrored notification's text lives in Windows' Action Center until it expires or you clear it."

### 5.6 Icons

`notif.icon` handling: check `png` decodes to ≤ 32,768 bytes, SHA-256 prefix equals `hash`, then run it through the **existing bounded WIC decoder in `images.rs`** (so an odd PNG can't be abused through the shell's image loader) requiring a square image of 24 to 128 pixels, then write `%APPDATA%\Kin\notif-icons\<hash>.png` (the name is hex only, so no path can be injected) **only if it doesn't exist**. At most 200 files or 8 MiB, oldest by modification time removed first (at start and hourly). The folder is removed when the last phone is unpaired. **Nothing else about notifications is written to disk** (the plan's rule), apart from the HKCU registration of 5.2.

### 5.7 One-time codes: Copy and the 60 s clear

- The `code` field is validated (3.3). The button's label is `Copy <code>`.
- **Copy** (no phone round trip): writes the code with the **same sensitive writer as the pairing link and sensitive texts** (`ExcludeClipboardContentFromMonitorProcessing`, `CanIncludeInClipboardHistory` = 0, `CanUploadToCloudClipboard` = 0), so Windows' clipboard history and cloud clipboard don't keep it **and Kin's own automatic clipboard sync doesn't send it back to the phone**. It records `(GetClipboardSequenceNumber() after the write, SHA-256 of the code)`.
- **After 60 s:** open the clipboard once; if the sequence number is **unchanged and** the text still equals the code, empty it; otherwise **do nothing and don't read** anything else. (The check and the clear are one step; the v0.1.1 "clear the pairing link if unchanged" code is the pattern.) A second Copy re-arms the one timer. On quitting Kin the same check runs once at once.
- A code in the toast also expires with the toast after 10 minutes (5.5).

### 5.8 Settings and status (PC)

`settings.json` gains: `mirrorNotifications` (default **false**), `mirrorSound` (false), `mirrorHideContent` (false), `mirrorDismissToPhone` (true). The Settings window gets a section "Phone notifications": the switch, the three others, and a line from the phone's last `notif.status` ("Pixel 8: mirroring 6 apps" / "Pixel 8: notification access is off" / "Update Kin on your phone"). `status.json` gets `mirror: {shown, removed, dropped: {...}}` counts. The switch sends the new `hello` (3.1).

### 5.9 What the PC stores and logs

- **Stores:** icon cache; four booleans; the HKCU keys of 5.2. **No database table, no content, no `nid`.**
- **Logs:** counts, and a **keyed tag** for correlation (`<notif 1a2b3c4d>`, the existing HMAC tag of `privacy.rs`, never the `nid` itself or any text), the app label **cut to 40** is allowed in a log line; titles, texts, replies, codes and phone-side keys never are. `export.rs` needs no new rule if this holds; a test greps the log of an e2e run for the test strings.
- **In memory only:** the ledger, rate buckets, the 60 s clipboard record.

### 5.10 Dev and test hygiene for the desktop coder

- All HKCU registration code refuses to run for the installed Kin's names unless `may_register_release` (windows.md) says so, and a dev/test run uses **throwaway AUMID and CLSID** (`…dev-<key>.phone` and a derived GUID) written and removed by the test in a `finally`, like rule 7's native-host keys.
- Real toasts on the user's PC in tests: **`SuppressPopup` and silent only**, a game/EasyAntiCheat check first (rule 6), and every test toast removed again. A developer setting `--dev-toast-quiet` forces this.
- No input injection: activation is tested by calling the activator through COM from a test process (`CoCreateInstance(CLSID, CLSCTX_LOCAL_SERVER)` then `Activate` with synthetic arguments and input), and dismissal through the same function the `Dismissed` handler calls (`--dev-toast dismiss <nid>`). What can't be tested this way is in 8.4.

## 6. Privacy and security

| # | Threat | Mitigation | Residual |
|---|---|---|---|
| T1 | Notification content ends up in logs, history, the database or the debug export | Nothing stored (4.12, 5.9). Counts and keyed tags only. Tests grep logs for the test strings | None known |
| T2 | Windows keeps toast text in its notification store | Short expiry, `ExpiresOnReboot`, sensitive form for risky apps, a PC setting to hide content (5.5, 3.8) | **Real and unavoidable**: text sits in Action Center storage until expiry. Documented |
| T3 | Windows mirrors the toast on to other devices (Phone Link) → a loop | `NotificationMirroring = Disabled` | [M] works as documented |
| T4 | A hostile or buggy phone injects markup or spoofs a "Windows Security" toast | The phone is paired and trusted, but content is untrusted text: cleaned (bidi, control), cut, set only through DOM setters, attribution always shows "<app> · <phone>", `app` ≤ 40 | A paired phone can still show any text. Same as `text` |
| T5 | Icon abuse (bad PNG, path trick, cache poisoning) | Size, hash and dimension checks, WIC bounded decode, hex-only names, no overwrite, cache caps (5.6) | Decoder bugs in Windows |
| T6 | Another program on the PC calls Kin's COM activator and makes it send a reply or press a button on the phone | Strict argument parse, a paired phone that lists `notif`, an action only for a `nid` in the ledger **or** in `GetHistoryWithId`, 5 actions per 10 s, `ack`-checked `label` on the phone | **A same-user process can do this.** The same boundary as the native-host pipe (D19, extension.md): malware on the PC is out of scope (architecture.md). Listed as R22 |
| T7 | A forged `notif.action` to the phone (paired PC compromised) | Phone checks index and label, allow list, per-action limits; no actions for sensitive or activity actions | A paired PC can reply "as the user" in allowed apps. Equal to the trust of pairing |
| T8 | Wrong button pressed because the app changed the notification | `label` echo → `stale` (3.7) | None |
| T9 | Flood (phone or PC) | Budgets on both ends (3.5), AC cap 30 | Burst until the limiter acts |
| T10 | Stale notification acted upon hours later | Not queued, not re-sent, `not_found` if gone, expiry (5.5) | A toast on screen is as old as its expiry |
| T11 | Sensitive apps (banking, authenticators) leak codes to a screen others see | Off by default, flagged, confirmation, forced `sensitive`, 10-minute expiry, "Hide content" setting | The user can turn them on knowingly |
| T12 | OTP code lingers on the clipboard | Sensitive formats, 60 s clear if unchanged, clear at quit (5.7) | Another app that reads the clipboard in those 60 s |
| T13 | Kin reads more than it should (non-allowed apps) | Rules 1 to 3 run before any extra is read; no `QUERY_ALL_PACKAGES`; large icons and pictures never read (4.5) | Notification access is broad by nature: the system grants it for all apps; Kin's discipline is the only limit |
| T14 | Other users or profiles | Rule 3 | None |
| T15 | The user can't tell what is shared | The onboarding states it; a persistent "Mirroring" row in Home when on; per-app list always visible | — |
| T16 | Screen sharing or recording on the PC shows toasts | "Hide content" setting; Windows' own presentation/Focus handling (the system holds popups in a full-screen or presentation state **[M]**) | Windows Recall or screenshots can still capture the toast: Windows-level, not Kin's |

**Unit-testable invariants (desktop):** a hostile `title` (`</text><action …>`, bidi, 100 KB) never changes the XML structure or the 4,800-byte cap; an unknown or malformed `launch` string is refused; a `notif.post` for a phone that isn't paired or didn't list `notif` is ignored.

## 7. New risks for `docs/risks.md` (the reviewer adds them at 2B.6; R18 is reserved for the photo permission)

| # | Risk | Likelihood | Impact | What we do about it | When we find out |
|---|---|---|---|---|---|
| R19 | Notification access is hard to get on a sideloaded APK (Android 13+ "Restricted setting", wider on 15), and many users give up at the settings screen | High | Medium | Step-by-step onboarding, install-source hint, re-check on return, steps card (4.2). Play-store installs aren't restricted | Real-hardware test |
| R20 | Android 15+ hides one-time codes from listeners, so the "Copy code" button often has nothing to copy | High | Low | Detect and say "Code hidden by Android"; no workaround. Research a Companion Device Manager association later (4.9) | Real Android 15+ phone (the emulator may lack the redaction) |
| R21 | A COM activator for an unpackaged app doesn't work reliably (registry-only AUMID, Kin not running, Windows 10) so replies and buttons from the Action Center don't work | Medium | Medium | Spike S1 first; fallback = text-only toasts (5.2); a separate AUMID isolates the Phase 1 toasts | S1, then real hardware |
| R22 | The COM class under HKCU lets any program of the same user trigger an action or a reply on the phone | Low | Low | Strict arguments, ledger/history check, rate limit, `label` check on the phone. Same boundary as the pipe (D19) | — |
| R23 | Windows keeps mirrored text in the Action Center store; screen sharing and Recall can capture a toast | High | Medium | Expiry, sensitive form, content-hiding setting, honest docs (5.5, T2, T16) | — |
| R24 | Messaging apps differ: a reply or button `PendingIntent` is an activity (silently blocked from the background), is cancelled after the first use, or the app needs the notification to be on screen | Medium | Medium | Offer only non-activity actions; `error action_failed`/`stale` to the PC; list in TESTING.md which apps were tried (WhatsApp, Telegram, Signal, Google Messages) | Real phone |
| R25 | OEMs unbind or kill the listener (the `NotificationListenerService` binding is dropped on some Xiaomi, Oppo, Samsung versions) | Medium | Medium | `onListenerDisconnected` → `requestRebind`, Home card "Notification access is off", `notif.status.access`, R1's setup list | Real phone |
| R26 | Google Play policy on notification access (declared use, user-data handling) | Medium | Medium | Opt-in, per-app, no storage, no analytics, privacy text; prepare the declaration with R4's Play work | Phase 2 Play submission |
| R27 | The `MAIN`/`LAUNCHER` entry in `<queries>` (to list the apps) draws a Play review question, or `getApplicationInfo` is refused for packages that aren't visible | Low | Low | Fall back to listing only packages that posted something (their package name, no label) | Phase 2 Play submission |

## 8. Test plan (lean)

Follows the user's lean-testing rule (memory, not a repo file): unit tests once, an emulator e2e only for what changed, no full regression.

### 8.1 Unit tests

**Android (JVM, no Robolectric; Android classes behind `NotifSnapshot`):** `MirrorFilter` (every rule and the order, from `skip[]`), `NotifEncoder` (limits, cleaning, `messages` order, `[Photo]`, 16 KiB shrink), `pickActions` (`pickActions[]`: reply chosen, activity actions dropped on API 31+, 3-button cap), `OtpDetector` (all of `otpDetect[]`: every keyword language, every negative), `RedactionMatcher`, `MirrorSender` with a fake clock (`rate[]`: coalescing, trailing post, identical post skipped, budget, per-package cap, remove cancels a pending post, icon before post, drop when offline, capability checked per message), `AppCatalog` classification, restricted-settings logic (install-source → hint), `nid` and icon hash against vectors, `notif.*` parsing against the `messages.json` cases, the `ExportRedactor` log grep.

**Desktop (`cargo test`):** message parsing and cleaning for every `notif.*` (vectors), `toast_xml` snapshots (full toast, sensitive, redacted, code, 5 buttons, hostile strings, 4,800-byte fit), `toastIds[]` and the strict argument parser, the ledger and generation rule (a superseded toast's `Dismissed` ignored; `removing` ignored), receiver rate limits (`rate[]`), `sync` diff, icon validation (bad hash, bad size, big, odd PNG), the clipboard clear with a fake clipboard and clock (unchanged → cleared, changed → untouched, second Copy re-arms), settings defaults and the `hello` re-send, `capabilities` follow the switch, logs grep.

**Vectors:** 3.11. `bun protocol/check-vectors.ts` stays green (the desktop coder adds the `scope-2b.2` group).

### 8.2 Emulator e2e (short; the integration agent writes `tools/e2e-2b-2.ps1`)

Uses `kin_api37` (and `kin_api29` for the Android 10 differences), the real desktop build with `--dev-bind 127.0.0.1`, a throwaway data dir and the developer setting `--dev-toast-quiet` (toasts go to the Action Center without a popup or sound). No clicks; the PC is checked through `GetHistoryWithId` and `status.json`, the phone through the debug `CMD` lines and `dumpsys notification`.

1. **Access and allow-list:** `cmd notification allow_listener …`; `MIRROR_ALLOW` for the debug channel and for `com.android.shell`; `MIRROR_ENABLE`. Expect `notif.status` seen on the PC.
2. **Post, update, remove:** `cmd notification post` from the shell, then again with new text (same tag), then `cmd notification cancel`. Expect one toast, replaced, then gone. (`cmd notification post` has flags for style, title and conversation messages; the coder reads `adb shell cmd notification help` on both images.)
3. **MessagingStyle:** a debug post with 3 messages → the toast shows the last two, group and "You:" handled.
4. **Reply and button:** the debug notification (4.14 a). The PC side runs `Kin.exe --dev-toast reply <nid> "hello"` (the same function the activator calls; **plus** one run through the real COM class from a test process, S1). Expect the phone's debug receiver to log the received reply text (a test-only echo of the string, e.g. a sha) and `ack`. The activity action is **not** offered.
5. **Dismiss:** `--dev-toast dismiss <nid>` → the phone's notification disappears from `dumpsys notification`, and no echo `notif.remove` returns.
6. **OTP:** the code post → `code` in the toast, `Copy` through `--dev-toast copy <nid>` → the clipboard holds it with the sensitive formats; **after 60 s it is empty**; and a variant where the test changes the clipboard first → left alone. **(Clipboard rule 4: save and restore the real clipboard around it.)**
7. **Skips:** the ongoing, summary, secret, low-importance, own, non-allowed posts → nothing reaches the PC; the counters say why.
8. **Offline and sync:** stop the dev PC, post and cancel two notifications, start it → no toast appears, and a toast left from before is removed by `notif.sync`.
9. **Rate limit:** 100 posts in 3 s → at most the budget reaches the PC; the rest are counted.
10. **Redaction (API 37):** post an OTP-looking notification from the shell and record what the listener sees. If the image redacts it, check `redacted: true` and the "Code hidden" toast; if not, record "no ASI on this image".
11. **Logs:** grep the phone and PC logs for the test strings (none).

Everything is cleaned up at the end (toasts removed, HKCU test keys removed, listener access revoked with `cmd notification disallow_listener …`).

### 8.3 Unit/e2e boundary decisions

- The e2e does **not** re-run the Phase 1 or 2B.1 suites. It runs only if `protocol/` or the shared link code changed, and the unit tests cover that.
- The COM activator is also tested **once** in the desktop's own test (S1's acceptance test), not again in the e2e.

### 8.4 What can't be tested on the emulator or without a click

- The restricted-settings flow with a really sideloaded APK (adb installs via a session).
- Android 15+ OTP redaction on a Google image with System Intelligence (maybe none on the emulator).
- Real messaging apps' reply and button `PendingIntent`s (WhatsApp, Telegram, Signal, Google Messages), contact `Person` URIs, MessagingStyle from real apps.
- OEM behaviour: listener unbinding (R25), battery, Doze, the real WebSocket after a deep sleep.
- On the PC: a real **click** on Send, a button or Copy, in the popup **and** in the Action Center; the **X** (`UserCanceled`) and clearing from the Action Center (5.4); toasts after a Kin restart; the look of the toast (icon, wrap, 5 buttons); Windows 10 behaviour; lock screen and Focus.
- The `NotificationMirroring` property doing what it says.

All of these go in `docs/TESTING.md` as the user's real-hardware checklist at v0.2.0.

## 9. Work split and first steps

**Desktop coder** owns: `protocol/` (including the vectors and `check-vectors.ts`), `docs/protocol.md` changelog 56 (section 3 of this document copied in), the Windows code (5), `docs/windows.md`, `examples/fake_phone.rs` (extended to send `notif.*` and print what it receives, so the PC can be tested with no phone), the dev commands (`--dev-toast reply|copy|dismiss <nid>`, `--dev-toast-quiet`).

1. **S1, spike, first, before anything is built on it** (≤ half a day): a throwaway dev instance registers a throwaway AUMID and CLSID under HKCU; shows a `SuppressPopup` toast with an `<input>` and a button; confirms it in `GetHistoryWithId`; then (a) with Kin running, a separate test process calls `CoCreateInstance(CLSID, CLSCTX_LOCAL_SERVER)` then `Activate(…)` with synthetic args and input and the handler gets them; (b) with Kin **not** running, the same call launches `Kin.exe --toast-activated` and the handler gets them once it is up; (c) the Phase 1 toasts (`app.kinrelay.desktop`) behave as before. Writes the outcome and the decision (works / fallback of 5.2) in `phase-2b-2-desktop.md` within the first hour of work. **Keys removed in a `finally`.**
2. `protocol/notif.json`, the `messages.json` cases and the protocol.md section **as early as possible**: the Android coder runs them as soon as they exist.
3. Message handling and the ledger, then `toast_xml` and the surface, activator, registration, icons, OTP clipboard, Settings and status.

**Android coder** owns: the manifest, the Android code (4), `Messages.kt`/`MessageReceiver.kt` for `notif.*`, the debug tools (4.14), `docs/android.md`, `android/README.md` (the new `CMD`s). If the Android coder finds a spec problem it writes it in its results file; it doesn't edit `protocol/` or `docs/protocol.md`.

1. Pure parts first (filter, encoder, pickActions, OtpDetector, sender, catalog) with the vectors of this document's tables until `protocol/notif.json` exists, then against it.
2. The listener service, the sender wired to the link, then the screens and onboarding.
3. The debug screen and commands, so the integration agent can start.

**Integration agent** (after both): `tools/e2e-2b-2.ps1` and `docs/results/phase-2b-2-integration.md` per section 8.2, with the emulator used by that one agent only.

**Results files:** `docs/results/phase-2b-2-{desktop,android,integration}.md`.

## 10. Open questions for the user

| # | Question | Recommendation |
|---|---|---|
| Q1 | An app the user turns on that Kin flags as sensitive (banking, authenticator, password manager): allow it **with a warning and a preview-less toast** (what this design does), or **refuse completely**? | Allow with the warning and the forced no-preview form. It's their phone; a hard block would be wrong when the flag is a guess (`pay`, `wallet`), and authenticator apps are the one place where a user may really want "something arrived" on the PC |
| Q2 | If the spike shows that replies and buttons from the Action Center can't work for the unpackaged Kin (R21), accept **text-only toasts** in 0.2.0 (the design's fallback), or hold the feature until Kin has package identity (a sparse MSIX plus a paid certificate)? | Accept text-only. The certificate is already a "later" decision (plan, 2B.5); mirroring that shows the text is still the core value |
| Q3 | Should the PC dismissing a toast (**X on the popup**) also dismiss it on the phone by default? The plan says yes; the cost of a wrong guess is a notification gone from the phone's shade, never a lost message | Keep default **on**, as the plan says, with the setting on both sides, and only the exact `UserCanceled` signal |
| Q4 | After 0.2.0, should an agent spend time on the **Companion Device Manager** route to be exempt from the Android 15 code redaction (a research spike, no promise)? | Not now. Ship "Code hidden by Android" and see how often real users hit it |
| Q5 | Should phone **calls and alarms** ever be mirrored? 0.2.0 skips them (calls need answering, which isn't possible) | Keep them out of 0.2.0; revisit after real use |
| Q6 | Is a `MAIN`/`LAUNCHER` `<queries>` entry (lets Kin list your apps for the picker) acceptable, knowing the Play review may ask about it (R27)? | Yes: it's the standard way and not `QUERY_ALL_PACKAGES`; the fallback is the packages that posted something |

## 11. Sources

- Android: [Android 15 behavior changes, OTP redaction](https://developer.android.com/about/versions/15/behavior-changes-all) (quoted in 4.9); [Restrictions on starting activities from the background](https://developer.android.com/guide/components/activities/background-starts) (4.7); AOSP [`CompanionDeviceManager.java`](https://android.googlesource.com/platform/frameworks/base/+/refs/heads/main/core/java/android/companion/CompanionDeviceManager.java) (the "Side-loaded apps must allow restricted settings" line, `REQUEST_COMPANION_PROFILE_COMPUTER`).
- Secondary, via search: Android Authority on [sensitive notifications](https://www.androidauthority.com/android-15-sensitive-notifications-3416414) and [the Phone Link fix](https://www.androidauthority.com/phone-link-sensitive-notifications-fix-3540158) (the pages themselves returned 403 to my fetch; only the search summaries were read), [9to5Google on sideloaded app restrictions](https://9to5google.com/2024/09/12/android-15-sideloaded-apps-restrictions/), [CleanBrowsing's steps](https://cleanbrowsing.org/support/mobile/disable-restricted-settings-android).
- Windows: [Respond to toast activations (Win32)](https://learn.microsoft.com/en-us/previous-versions/windows/desktop/win32_tile_badge_notif/respond-to-toast-activations), [`INotificationActivationCallback::Activate`](https://learn.microsoft.com/en-us/windows/win32/api/notificationactivationcallback/nf-notificationactivationcallback-inotificationactivationcallback-activate), [`AppNotificationManager.Register`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.windows.appnotifications.appnotificationmanager.register).
- In this repo and on this PC: `tauri-plugin-notification` 2.6.0, `notify-rust` 4.18.2, `tauri-winrt-notification` 0.8.1 and `windows` 0.62.2 sources under `~/.cargo/registry/src`; `HKCU\Software\Classes\AppUserModelId` of this PC (read-only listing).
