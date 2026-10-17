# Browser extension and the native messaging host

The contract between the Kin browser extension (`extension/`) and Kin's native messaging host (`kin-native-host.exe`, built in the Rust workspace by the desktop side). Everything the host has to do is here. Where this document and the extension's code disagree, that's a bug in one of them.

Decision D19 in [decisions.md](decisions.md) records why it's built this way. The phone side never sees any of it: Kin turns an extension request into the ordinary `text`, `offer` and (from 2B.1) `link` messages of [protocol.md](protocol.md).

## Architecture

```
 Browser (Chrome / Edge / Brave / Vivaldi / Opera / Firefox)
 +------------------------+   stdin/stdout, native messaging framing
 | Kin extension (MV3)    |------------------------------+
 +------------------------+                              v
                                           +---------------------------+
                                           | kin-native-host.exe       |  one process per request
                                           | (started by the browser)  |  (or per port, see below)
                                           +-------------+-------------+
                                                         | named pipe, current user only
                                                         v
                                           +---------------------------+
                                           | Kin (kin.exe, in the tray)|
                                           +---------------------------+
```

- The browser starts the host, once per `runtime.sendNativeMessage` call (the extension uses only that). The host reads one request, answers with exactly one response and exits. It must also work as a long-lived `connectNative` host (loop until stdin closes), so the extension can switch later without a host change.
- The host is a thin relay: it checks sizes, forwards the request to Kin over the pipe and writes Kin's answer back. All the logic (devices, queue, downloads, errors) is in Kin. The only things the host answers on its own are the errors marked **host** in [Error codes](#error-codes).
- **Nothing in Kin listens on the network because of the extension.** The extension never talks to the phone and never learns an address or a key.

## Names

| What | Release | Dev and tests |
|---|---|---|
| Native messaging host name | `app.kinrelay.desktop` | `app.kinrelay.dev` |
| Host executable | `kin-native-host.exe`, next to `Kin.exe` (`C:\Program Files\Kin\`) | whatever the test registers |
| Pipe | `\\.\pipe\app.kinrelay.desktop.<user SID>` | `\\.\pipe\app.kinrelay.dev.<user SID>` |
| Chromium extension ID (dev, from the manifest `key`) | `cbioefpddjpaionhljhffgbedncfchmn` | the same |
| Firefox extension ID | `kin@kinrelay.app` | the same |

The host name follows the browser rules: lowercase letters, digits, `_` and `.`, no leading, trailing or doubled dots. The extension picks the host name at build time: `bun run build` makes the release build (`app.kinrelay.desktop`), `bun run build:dev` and the test build use `app.kinrelay.dev`. A dev Kin (any Kin that is not the installed one: a debug build, a developer setting, a `--data-dir` of its own) must listen on the dev pipe, never on the release one, so a test can't talk to the user's installed Kin. A dev Kin appends `.<suffix>` to its pipe name when the environment variable `KIN_DEV_PIPE_SUFFIX` is set (letters, digits, `-` and `_`), so two test Kins can listen at once; the dev host build takes the whole pipe name from `KIN_HOST_DEV_PIPE`.

The store IDs of the extension (Chrome Web Store, Edge Add-ons) are different from the dev ID. They are added to `allowed_origins` below when the extension is published (a later step; see [Open questions](#open-questions)).

## Host manifests

One manifest file per family, **written by Kin itself at every start** into its own data folder (`%APPDATA%\Kin\native-messaging\`; the installer only lays down `kin-native-host.exe` next to `Kin.exe`, see [Registry keys](#registry-keys)). Both name the same `kin-native-host.exe`, by an absolute `path` (the manifest is in the data folder and the host is in the install folder; a moved install repairs itself at the next start).

**Chromium** (`app.kinrelay.desktop.chromium.json`):

```json
{
  "name": "app.kinrelay.desktop",
  "description": "Kin Relay: send things from the browser to your phone",
  "path": "..\\kin-native-host.exe",
  "type": "stdio",
  "allowed_origins": [
    "chrome-extension://cbioefpddjpaionhljhffgbedncfchmn/"
  ]
}
```

- `path` may be relative to the manifest's folder on Windows. Kin writes an absolute one (the manifest and the host are in different folders).
- `allowed_origins` has no wildcards (Chromium refuses them). The trailing `/` is required. **Edge reads only the first manifest it finds** (see the registry order below), so the list must hold every extension ID that may use the host, from every store.
- The dev manifest is the same file with `"name": "app.kinrelay.dev"` and a `path` the test chooses.

**Firefox** (`app.kinrelay.desktop.firefox.json`):

```json
{
  "name": "app.kinrelay.desktop",
  "description": "Kin Relay: send things from the browser to your phone",
  "path": "..\\kin-native-host.exe",
  "type": "stdio",
  "allowed_extensions": [
    "kin@kinrelay.app"
  ]
}
```

Firefox uses `allowed_extensions` (IDs), not origins. The first command-line argument of the host is the full path of the manifest and the second is the extension ID; Chromium's first argument is the caller's origin (`chrome-extension://<id>/`) and, on Windows, a `--parent-window=<handle>` (0 from a service worker). The host ignores `--parent-window` and passes the origin or ID to Kin as `origin` (see [Pipe](#pipe-host--kin)).

## Registry keys

All keys are written under **HKCU** (no administrator needed, per user), with the manifest's full path as the **(Default)** value of a key named after the host. **Kin writes them itself, as the signed-in user, at every start** (the installer is per machine and runs elevated, so its `HKCU` can be another account's: the same reason the "Send to" shortcut and the Run value are the app's). Only the installed Kin does (a release build that runs the way a user runs it, not from a build folder, with the host beside it): a debug build, a developer setting, a `--data-dir` of its own or a test never writes the release keys, and Kin never writes keys for the dev host name. The uninstaller removes the keys in both contexts. (A per-machine `HKLM` copy is possible and every browser reads it, but then the key would have to be written for every user, so we don't.)

| Browser | Key (under `HKCU\Software\`) | Notes |
|---|---|---|
| Chrome | `Google\Chrome\NativeMessagingHosts\<host>` | Verified in the Chrome docs. Chrome looks at HKCU, then HKLM, 32-bit view first |
| Chromium | `Chromium\NativeMessagingHosts\<host>` | Used by Chromium builds (it's the key the test browser reads) |
| Edge | `Microsoft\Edge\NativeMessagingHosts\<host>` | Verified in the Edge docs. Edge's search order is its own key, then the Chromium key, then the Chrome key (HKCU first, then HKLM), and **it stops at the first manifest it finds** |
| Brave | `BraveSoftware\Brave-Browser\NativeMessagingHosts\<host>` | The Chrome key is also read (KeePassXC relies on that). Writing the own key as well costs nothing |
| Vivaldi | `Vivaldi\NativeMessagingHosts\<host>` | The Chrome key is also read (reported by KeePassXC users). Same advice |
| Opera | the Chrome key | Opera is reported to share the Chrome key. **Not verified on a real Opera**: the user checks it on real hardware at v0.2.0 |
| Firefox | `Mozilla\NativeMessagingHosts\<host>` | Verified in the Mozilla docs. Firefox reads HKCU, then HKLM |

So the installer writes **five keys** pointing at the Chromium manifest (Chrome, Chromium, Edge, Brave, Vivaldi; Opera has no key of its own, it uses the Chrome one) and **one** key at the Firefox manifest, six in all. The uninstaller removes exactly those keys (and nothing else under those parents). The keys are harmless when a browser isn't installed.

Tests register only `HKCU\Software\Chromium\NativeMessagingHosts\app.kinrelay.dev` (the key the Playwright Chromium reads) and remove it in a `finally`.

## Framing

The native messaging framing, the same in both directions:

- A message is a UTF-8 JSON object, preceded by its length in bytes as an unsigned 32-bit integer in the platform's byte order (little-endian on Windows).
- **Set stdin and stdout to binary mode** (`_O_BINARY`). Otherwise Windows turns `\n` into `\r\n` and the framing breaks. (Rust's `std::io` is binary already; this is only a warning for a C runtime.)
- **Never write anything else to stdout**, not even a log line. Log to stderr, or better to Kin's own log (no message bodies, no URLs: counts and codes only).
- **Limits (verified in the Chrome and Edge docs):** the host may send at most **1 MB (1,048,576 bytes)** to the extension; larger and the browser kills the connection. The extension may send up to **64 MiB (67,108,864 bytes)** to the host in Chrome (Edge and Firefox allow 4 GB; we stay with the smaller number). The host checks the length prefix before it reads the body, and answers `TOO_LARGE` for anything over 64 MiB without reading it (it still has to drain or close; closing stdin and exiting is fine).
- The browser may close stdin at any time (the user closed the browser, the extension was reloaded). The host exits quietly.

## Messages

Every message is a JSON object. Unknown fields are ignored on both sides (the same rule as [protocol.md](protocol.md#json-conventions)), so v1 can gain optional fields.

### Request (extension → host)

```json
{ "v": 1, "id": "7d0a3f0e-…", "type": "status" }
```

| Field | Type | Notes |
|---|---|---|
| `v` | integer | Contract version. `1` |
| `id` | string | 1 to 64 characters. The extension uses a random UUID. Echoed as `ref` in the response |
| `type` | string | `status`, `start`, `open`, `send` |

### Response (host → extension)

```json
{ "v": 1, "ref": "7d0a3f0e-…", "type": "status", "ok": true, "...": "type-specific fields" }
```

| Field | Type | Notes |
|---|---|---|
| `v` | integer | `1` |
| `ref` | string | The request's `id`. The extension ignores a response whose `ref` doesn't match |
| `type` | string | The request's `type`, or `error` when the host or Kin couldn't even tell what the request was |
| `ok` | boolean | `true` when the request did what it was for. `false` is an error: `code` and `message` are set |
| `code` | string | Optional when `ok` is `true` (only for a notice, see `DEVICE_OFFLINE_QUEUED`), required when `ok` is `false`. See [Error codes](#error-codes) |
| `message` | string | Human text, English, **ready to show the user** as it is (at most 300 characters). Never used for logic. The extension has its own fallback text for each code and shows Kin's `message` when it has one |

If the request isn't valid JSON, has no usable `id`, or has no `type`, the response has `type: "error"`, a missing `ref` and `code: "BAD_REQUEST"`.

If `v` isn't supported, the answer is `{"v":1, "ref":…, "type":"error", "ok":false, "code":"UNSUPPORTED_VERSION", "supported":[1], "message":"…"}`. The extension shows "Update Kin" or "Update the extension", depending on whether the host's `supported` list is lower or higher than its own version. A host that is newer than the extension answers every version it still knows.

### `status`

Asks what Kin is doing. **It never starts Kin** (opening the popup must not launch a program).

Request: `{ "v": 1, "id": "…", "type": "status" }`

Response when Kin is running:

```json
{
  "v": 1, "ref": "…", "type": "status", "ok": true,
  "running": true,
  "kinVersion": "0.2.0",
  "hostVersion": "0.2.0",
  "devices": [
    { "id": "<43-char device id>", "name": "Pixel 8", "online": true, "queuedCount": 0 },
    { "id": "…", "name": "Galaxy A54", "online": false, "queuedCount": 2 }
  ],
  "defaultDeviceId": "<id of the device a send without deviceId goes to>"
}
```

| Field | Type | Notes |
|---|---|---|
| `running` | boolean | Whether the host reached Kin through the pipe |
| `kinVersion` | string | Optional. Kin's version |
| `hostVersion` | string | The host's own version (set by the host, even when Kin isn't running) |
| `devices` | array | The paired devices, at most 16, in the order Kin shows them. Empty when none is paired |
| `devices[].id` | string | The device ID of [protocol.md](protocol.md#encodings) |
| `devices[].name` | string | 1 to 64 characters |
| `devices[].online` | boolean | `true` while the device's WebSocket is open |
| `devices[].queuedCount` | integer | Items waiting in the queue for that device (texts, links, offers) |
| `defaultDeviceId` | string | Optional. The device a `send` without `deviceId` goes to: the only paired device, or with several the one that connected last |
| `paused` | boolean | Optional (2B.4). `true` while the user has paused Kin. The extension shows "Paused" and still sends (a manual send isn't blocked by a pause) |

**When Kin isn't running** the host answers (this is **not** an error):

```json
{ "v": 1, "ref": "…", "type": "status", "ok": true, "running": false, "hostVersion": "0.2.0", "devices": [] }
```

The host decides this itself: it tries to open the pipe, and on "file not found" or "pipe busy for more than 1 second" it answers `running: false` without starting anything.

### `start`

The "Start Kin" button. Same request shape with `"type": "start"`. The host starts Kin if the pipe isn't there (`Kin.exe` next to the host, with no window: the same start as the tray at sign-in), waits up to **10 seconds** for the pipe, and answers like `status`. If Kin still isn't there it answers `ok: false`, `code: "KIN_NOT_RUNNING"`. A running Kin: answered like `status`, nothing is started twice (Kin is single-instance anyway).

### `open`

Optional. Asks a running Kin to show a window. `{ "v":1, "id":"…", "type":"open", "target":"main" }` with `target` `main` (Kin has no main window: it opens the "Send to phone" window) or `pairing` (the pairing window of "Pair new phone", which starts a pairing). The response is `{ ok: true }` or an error (`KIN_NOT_RUNNING`, `UNSUPPORTED_TYPE`). The extension shows this button only on a `NOT_PAIRED` error. A Kin that doesn't implement it answers `UNSUPPORTED_TYPE` and the extension hides the button.

### `send`

```json
{
  "v": 1, "id": "…", "type": "send",
  "deviceId": "<optional device id>",
  "kind": "link",
  "...": "kind-specific fields"
}
```

`deviceId` is optional. Without it Kin uses `defaultDeviceId`. An unknown `deviceId` is `NOT_PAIRED`.

Kinds:

| `kind` | Fields | Kin sends |
|---|---|---|
| `text` | `text` (string, 1 byte or more, at most **4 MiB** of UTF-8) | A `text` message, with the rules of [protocol.md](protocol.md#text-messages) (over 128 KiB it goes as `Text.txt`). `isUrl` is Kin's own decision, not the extension's |
| `link` | `url` (string), `title` (optional string), `timestampSec` (optional integer) | A `link` message `{url, title?, sourceApp?}` (2B.1). The extension doesn't say which browser it is (Brave and Vivaldi can't be told apart reliably), so Kin sets `sourceApp` to `"Browser"` or leaves it out |
| `page` | `url`, `title` (optional) | The same as `link`. A separate kind only so the history and the toast can say "page" |
| `image` | `url` + `pageUrl` (optional) + `name` (optional), **or** `data` + `mime` + `name`, see below | A file offer with one image file (the ordinary file transfer, which lands in the phone's gallery) |

Rules for `link` and `page` (the extension checks them first, **and Kin checks them again**, because any program can write to the pipe):

- The URL must be `http` or `https`. Anything else (`javascript:`, `file:`, `data:`, `chrome:`, `about:`, `blob:`, `intent:`) is `BAD_URL`.
- No whitespace or control characters, and at most 8 KiB (8,192 bytes) of UTF-8: the "openable URL" rule of [protocol.md](protocol.md#links-isurl-and-openable-urls). The extension sends `new URL(x).href` (so a non-ASCII host is punycode and spaces are `%20`), which satisfies it. The extension also refuses, with a message that says why (not a generic `BAD_URL` from Kin), the authority forms Kin refuses: user information (`user@host`), a `\` or `%` before the first `/`, one of `[ ] < > ^ |` in a host that is not an IPv6 literal, and a bad port. Kin rejects what the rule rejects, and it does **not** rewrite a URL that passes: YouTube `t=`, `list=`, fragments all stay as they are.
- `title` is at most **300 characters** (code points). The extension collapses whitespace runs and cuts at 300; Kin cuts again and cleans it like a file name (bidi characters replaced, protocol.md "File names"). An empty title is left out.
- `timestampSec` is informational: the number of seconds the extension already put in the URL's `t=` parameter. Kin may show "at 1:23" in the toast. **Kin never edits the URL.**

Rules for `text`: the selected text, with the line ends the browser gives (Kin converts CRLF to LF like any text). It's never `sensitive` (a browser can't know).

#### Images: URL first, bytes as a fallback

Two shapes of `image`:

1. **By URL** (the first try for an `http` or `https` image):

   ```json
   { "kind": "image", "url": "https://example.com/a.jpg", "pageUrl": "https://example.com/post", "name": "a.jpg" }
   ```

   Kin downloads it itself: `GET`, no cookies, no credentials, redirects followed up to 5 (only `http` and `https`, and never from https to http), 30 seconds in total, **at most 64 MiB** (stops reading at the cap, not after), `User-Agent` of a normal browser, `Referer: <pageUrl>` when given (a lot of image hosts refuse a request without one). It accepts the result only if the bytes are an image, **decided by the magic bytes** of PNG, JPEG, GIF, WebP, BMP, TIFF, AVIF and HEIC (Kin leaves out "the decoder opens it": those are the formats a browser shows, and a byte check needs no COM; SVG is not accepted, it can carry script), whatever `Content-Type` says. The file name is `name` if given, else the last path segment of the URL, else `image`, with an extension from the detected type, cleaned and made unique like every file name. Then it's queued like any offer.

   It may point at an address on the user's own network (the user picked that image on a page they were looking at), but the answer never goes back to the extension or the page: the bytes only go to the phone.

2. **By bytes** (when the extension had to fetch it, see the round trip below, or for a `data:` URL):

   ```json
   { "kind": "image", "data": "<base64, standard alphabet, with padding>", "mime": "image/png", "name": "photo.png", "retryOf": "<id of the failed request>" }
   ```

   `data` is the file content. **The extension sends at most 32 MiB of raw bytes this way**, because base64 makes it a third bigger (32 MiB becomes 42.7 MiB, which with the JSON fits under the 64 MiB limit; 48 MiB of raw bytes would not). Bigger is `TOO_LARGE`. Kin checks the same "is an image by content" rule and ignores `mime` for security. `retryOf` is optional and only for correlation in the log.

Exactly one of `url` and `data` must be present, otherwise `BAD_REQUEST`.

**The round trip for an image that needs the browser** (sign-in required, a cookie, a one-time URL):

```
 extension                          host/Kin                       website
    | send image {url, pageUrl} ------->|                              |
    |                                   |-- GET url (no cookies) ----->|
    |                                   |<-- 403 / 401 / not an image -|
    |<-- ok:false, IMAGE_NEEDS_BYTES ---|
    | fetch(url, credentials: include)  |                              |
    |   in the PAGE (scripting + activeTab) -------------------------->|
    |<------------------------------------------------ the bytes ------|
    | send image {data, mime, name, retryOf} -->|                      |
    |<-- ok:true (queued / sent) -------|
```

- Kin answers `IMAGE_NEEDS_BYTES` when the download failed in a way the browser might get past: an HTTP status 401, 403, 404, 429 or 5xx, a timeout, a connection error, or a result that isn't an image (an HTML sign-in page). The `message` says what happened ("The site wouldn't give Kin the picture").
- It answers `TOO_LARGE` (not `IMAGE_NEEDS_BYTES`) when the download reached the 64 MiB cap or `Content-Length` says it would, and `NOT_AN_IMAGE` is **not** used for the first attempt (the fallback gets the chance), only when bytes sent by the extension aren't an image.
- The extension then fetches the URL **inside the page** with `credentials: "include"` (the page's cookies; the page's origin and the CORS rules apply, so this works for same-site images and CORS-enabled hosts and fails for the rest). If that works and the result is at most 32 MiB it sends the bytes; if not it shows the failure to the user (`IMAGE_UNREACHABLE`, or `TOO_LARGE`). **At most one retry per send.**
- `blob:` URLs only make sense inside the page, and `data:` URLs can be decoded anywhere, so for those the extension skips step 1 and goes straight to bytes.
- The page is read only because the user clicked "Send to Kin" on that image, which gives the extension `activeTab` for that tab (the permission list has no host permissions at all).

**Response to `send`:**

```json
{
  "v": 1, "ref": "…", "type": "send", "ok": true,
  "queued": false,
  "deviceId": "…", "deviceName": "Pixel 8",
  "transferId": "<uuid, optional>"
}
```

- `ok: true, queued: false`: the item was sent to the connected device (and acked, for a text or link; for a file offer, "acked" is the phone's `ack` of the `offer`).
- `ok: true, queued: true`: the device is offline and the item was **saved in Kin's queue** (the ordinary queue of [windows.md](windows.md#queue)). `code` is `DEVICE_OFFLINE_QUEUED` and `message` says it ("Pixel 8 is offline. It will be sent when the phone is back."). This is a success with a notice, so a client that only looks at `ok` is right too.
- `ok: false`: not sent and not queued. `code` and `message` say why.

If sending takes a while (the image download is the only slow part: up to about 35 seconds), the host simply doesn't answer until it has the result. **The extension waits up to 90 seconds** for a `send` and 8 seconds for a `status` (15 for `start`). On expiry it reports `TIMEOUT` and ignores a late answer.

### Error codes

| Code | Produced by | Meaning and what the extension says |
|---|---|---|
| `KIN_NOT_RUNNING` | host | Kin isn't running and the host couldn't start it, or it didn't open the pipe within 10 s. "Kin isn't running. Start Kin and try again." (With a **Start Kin** button) |
| `HOST_NOT_INSTALLED` | extension | The browser has no host with that name (`Specified native messaging host not found`, `…is not registered`), or the host's start failed. "Kin isn't installed, or its browser support is missing. Install or repair Kin." |
| `HOST_FORBIDDEN` | extension | The browser said "Access to the specified native messaging host is forbidden": the extension's ID isn't in `allowed_origins`. "This copy of the extension isn't allowed to talk to Kin." |
| `HOST_EXITED` | extension | "Native host has exited" or "Error when communicating with the native messaging host" (a crash or a framing bug). "Kin's browser helper stopped unexpectedly." |
| `TIMEOUT` | extension, or host (Kin didn't answer the pipe) | "Kin didn't answer in time." |
| `NOT_PAIRED` | Kin | No device is paired, or `deviceId` isn't one of them. "Pair your phone with Kin first." (With **Open Kin** when `open` works) |
| `DEVICE_OFFLINE_QUEUED` | Kin | **Success notice** (`ok: true`, `queued: true`), see above |
| `TOO_LARGE` | host or Kin | Over a limit: 64 MiB request, 4 MiB text, 32 MiB image bytes, an image download over 64 MiB. "That's too big to send." |
| `BAD_URL` | extension, Kin | The URL isn't `http` or `https`, has whitespace or control characters, or is over 8 KiB. "Kin can only send web links (http or https)." |
| `BAD_REQUEST` | host or Kin | Malformed JSON, a missing or wrong-type field, an unknown `kind`. A bug, not something the user fixes |
| `UNSUPPORTED_TYPE` | host or Kin | Unknown request `type` (or `open` not implemented) |
| `UNSUPPORTED_VERSION` | host | `v` isn't supported; `supported: [1]` |
| `IMAGE_NEEDS_BYTES` | Kin | The download by URL failed, the browser may fetch it. **Never shown to the user**: the extension turns it into the retry, see above |
| `IMAGE_UNREACHABLE` | extension | Both ways failed. "Kin couldn't get that image. If the site needs you to be signed in, save it and send the file instead." |
| `NOT_AN_IMAGE` | Kin | The bytes the extension sent aren't a picture. "That doesn't look like an image." |
| `INTERNAL` | host or Kin | Anything else that went wrong inside. "Something went wrong inside Kin." (The cause goes to Kin's log) |

An unknown code is treated as a failure and its `message` is shown as it is. A code never changes meaning in v1; new codes can be added.

## Pipe (host ↔ Kin)

This part is Kin's own business (the extension never sees it), but it's written down here so the host and Kin are built to the same plan.

- **Name:** `\\.\pipe\<host name>.<user SID>` (the table above). One instance of Kin owns it.
- **Access:** created with a security descriptor that allows **only the current user** (the user's SID, and nothing for `Everyone`, `Users` or `Network`), and with `PIPE_REJECT_REMOTE_CLIENTS`. Byte-mode pipe (`PIPE_TYPE_BYTE`), `FILE_FLAG_FIRST_PIPE_INSTANCE` on the first instance (so a program can't squat the name before Kin starts: if creating the first instance fails, Kin logs it and runs without the extension pipe), several instances so a second host isn't refused.
- **The host checks the server too:** after connecting it checks that the pipe server's process belongs to the current user and is `Kin.exe` in the install folder next to the host (`GetNamedPipeServerProcessId`, then the image path). Otherwise it answers `KIN_NOT_RUNNING` and sends nothing. (A same-user program could still squat the name earlier than Kin; see the security notes.)
- **Framing:** the same as the native messaging framing (a 32-bit little-endian length, then UTF-8 JSON), in both directions, **up to 64 MiB + 1 MiB** per message. The host forwards the extension's request object **unchanged**, with two fields added: `"origin": "<chrome-extension://id/ or the Firefox extension ID>"` and `"hostVersion"`. It forwards Kin's answer object unchanged, except that it fills `hostVersion` in a `status` or `start` response if Kin didn't.
- **One request per connection** is fine (open, write, read, close). The host opens a connection per request.
- **Kin's side** parses the request with the JSON rules of [protocol.md](protocol.md#json-conventions) (generic tree first, then validate), enforces every limit of this document again, and treats the request as coming from the user (same trust level as a click in the tray), but **only for the actions here**: `send` of the four kinds, `status`, `open`, `start`. Nothing in this API changes settings, unpairs a device or reads history.
- **Rate limit:** a runaway program must not flood the phone. Kin accepts at most **30 `send` requests per 10 seconds** over the pipe and answers `INTERNAL` ("Too many requests, slow down") for the rest.

## Security notes

- **Who can use the pipe:** any process running as the same Windows user. That is the same trust boundary as everything else Kin has: such a process can already read the user's files, use their clipboard, press the tray's menu through UI automation, run `Kin.exe --send-clipboard` or `Kin.exe --send <paths>` (the single-instance command line already accepts it) and read Kin's own data directory (DPAPI protects the keys only from other users and other machines). So the pipe adds no new power over the existing command line. It's acceptable by design (D19), and it is why the API is narrow (no unpair, no settings, no history) and rate-limited.
- **Not reachable from other users or the network:** the DACL names the current user only and `PIPE_REJECT_REMOTE_CLIENTS` refuses SMB access.
- **Not reachable from web pages:** a page can't talk to a native messaging host at all. Only the extensions listed in `allowed_origins` (or `allowed_extensions`) can start it, and the browser enforces that. The extension has no content script and no `externally_connectable`, so no web page can drive it either: the only way to a `send` is a click on the user's own menu, button or keyboard shortcut.
- **Link safety:** both ends accept only `http` and `https` links, so a page can't make Kin send `javascript:` or `file:` links to the phone. The title is untrusted text and is cleaned like a file name. The real domain is shown on the phone.
- **Image download:** Kin sends no cookies and no credentials, never returns the bytes to the browser, only follows `http` and `https`, and has size and time caps. It does fetch addresses on the local network if the user picked an image from one; that is a conscious "send me this" and shows nothing back to any page.
- **The extension reads a page only on a click.** `activeTab` gives access to the one tab the user clicked on, until it navigates. It's used for exactly two things: the time of a YouTube video, and the bytes of an image the user chose to send. Nothing else is read, and there's no permanent site access and no `<all_urls>`.
- **What the extension stores:** the selected device, the cached device list (session storage, gone when the browser closes) and the **last 10 sends**, each as a kind, a short label (the page title or the host name for a link; "Text, 42 characters" for text, never the text itself), the time and the outcome. The popup has **Clear**. Nothing leaves the browser except the messages to the host.
- **No telemetry**, no network access of the extension at all (it has no host permissions, so its own `fetch` could only reach pages through the injected function).

## Host build notes (for the desktop agent)

- A separate small binary in the Rust workspace, `kin-native-host.exe`, with no dependency on Tauri (a small install and a quick start: the browser starts it for every request). Only `std`, `windows-sys` (pipes, security descriptors, the registry stays in the installer) and `serde_json`.
- Parse only what it must: the length prefix, `v`, `id`, `type`. Everything else is forwarded byte for byte after the size checks, so a new field needs no host update.
- Build it with `/SUBSYSTEM:WINDOWS`-like behavior or a `windows_subsystem = "windows"` attribute: **no console window may flash** when a browser starts it. stdin and stdout still work.
- Exit codes don't matter to the browser. Exit when stdin is at EOF, after the answer, or on any framing error.
- Starting Kin from the host (`start`, and `send` when Kin isn't running): `CreateProcessW` of `Kin.exe` next to the host with `DETACHED_PROCESS | CREATE_NO_WINDOW`, then retry the pipe for up to 10 seconds (every 100 ms). **A `send` request waits for that start and then goes through**, so the first click after boot isn't lost. `status` never starts it.
- **Installer:** copies the host next to `Kin.exe` and removes the keys and the manifests on uninstall; **Kin** writes the manifests and the keys of [Registry keys](#registry-keys) at every start. The Chromium manifest's `allowed_origins` is a list that can grow (store IDs).
- **Testing without a browser:** feed the host a framed request on stdin and read the framed answer. A test can use the mock host in `extension/tests/smoke/mock-host.ts` as the reference of the expected answers (it implements every request and error of this document, with a scenario file to pick the behavior).
- The cases the extension's smoke test covers are listed in [results/phase-2b-1-extension.md](results/phase-2b-1-extension.md); a host that passes the same cases is compatible.

## Extension behavior (for reference)

- **Menu:** "Send to Kin" on selected text, links, images and the page. One paired device: one item. Several: a submenu with one entry per device. An offline device is shown as "Pixel 8 (offline, will queue)". The device list is refreshed when the browser starts, when the popup opens and after each send (a menu can't change while it is open, and the extension has no background timer).
  - On an image, the image wins over a link around it. On a link, the link wins over the selection. A selection wins over the page.
- **Toolbar popup:** state of Kin and the devices, a device picker when there are several, **Send this tab**, the last 10 sends, and errors in plain words.
- **Shortcut:** `Alt+Shift+K` sends the current tab to the selected (or only) device. The user can change it in the browser's own shortcut page.
- **Feedback for menu and shortcut sends** (no notifications permission): the toolbar button shows a badge for a few seconds (`✓` sent, `Q` queued, `!` failed) and its tooltip carries the message. The popup shows the details.
- **YouTube:** on `youtube.com`, `www.youtube.com`, `m.youtube.com`, `music.youtube.com` and `youtu.be`, for a watch page (`/watch`, `/live/…`, `youtu.be/<id>`), the extension reads the playing video's `currentTime` in the page at click time, and writes `t=<whole seconds>s` into the URL, replacing an existing `t`. It skips this for the time 0, a live stream, an ad that's playing (the player has the `ad-showing` class) and a link to a different video than the one on screen. Everything else in the URL stays.
- **Permissions:** `contextMenus`, `nativeMessaging`, `activeTab`, `scripting`, `storage`. No host permissions, no `<all_urls>`, no content scripts, no remote code.

## Open questions

1. **Store IDs.** Once the extension is in the Chrome Web Store and Edge Add-ons, their IDs go into the Chromium manifest's `allowed_origins` (next to the dev ID, which can stay for side-loading). Needs the user's developer accounts. **Edge reads the first manifest it finds**, so a single manifest holding every ID is the safe design (already so above).
2. **Opera** is assumed to read the Chrome key. Check on real hardware.
3. **Images over 32 MiB that need the browser's cookies** can't be sent by bytes in one message. A chunked upload (`send` with `part`/`parts`) would fix that; it isn't worth it for v0.2.0 (the usual image is a few MB).
4. **Cross-origin images that need cookies** (a CDN that checks a cookie and has no CORS header) fail the in-page `fetch`. A content script that draws the `<img>` on a canvas can't read it either (tainted). The honest fallback is the error message. A later improvement could ask the user for the optional host permission of that one site.
5. **Live status** (the menu always showing the current online state) needs the host to stay connected (`connectNative` with pushes) or an alarm; neither is in the permission list. Out of scope for 2B.1.
