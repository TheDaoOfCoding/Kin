# Protocol v1

The PC runs an HTTPS server. The phone is always the client. Example messages and test vectors go in `protocol/` at the repo root, and both apps' tests use them.

## Transport

- TLS 1.3 only, over TCP. HTTP/1.1 (ALPN `http/1.1`).
- Default port 47821. If it's taken, the PC uses a random port and advertises it.
- **Server certificate:** the PC's self-signed certificate. The phone ignores hostname and expiry and checks only that SHA-256(SPKI) matches the pinned PC device ID.
- **Client certificate:** required. The PC accepts any client certificate during the TLS handshake. Authorization happens per request:
  - `POST /v1/pair` needs a valid pairing token.
  - Every other endpoint needs a client certificate whose device ID is in `devices`. Otherwise it returns `401 unauthorized`.

## Pairing

### QR code contents

```
kinrelay://pair?v=1&a=192.168.1.20:47821,10.0.0.5:47821&id=<pc-device-id>&tk=<token>&n=<pc-name>
```

| Field | Meaning |
|---|---|
| `v` | Protocol version |
| `a` | Up to 3 private IPv4 address:port pairs. Leave out loopback, link-local and virtual adapters (Hyper-V, WSL, VPN) |
| `id` | PC device ID: SHA-256(SPKI), base64url |
| `tk` | Pairing token: 16 random bytes, base64url. Single use, valid for 5 minutes |
| `n` | PC name, URL-encoded |

In Phase 2 the QR switches to an `https://<domain>/pair#...` link so a normal camera can open the app. The phone app accepts both forms from day one.

### Steps

1. The phone races the addresses in `a` and connects with TLS. It checks that the PC certificate's SHA-256(SPKI) equals `id`, and aborts if it doesn't.
2. The phone sends `POST /v1/pair`:

   ```json
   { "token": "<tk>", "name": "Pixel 8", "platform": "android", "model": "Google Pixel 8", "appVersion": "0.1.0", "protocol": 1 }
   ```

3. The PC compares the token in constant time and checks that it hasn't expired or been used. It then saves the phone's device ID, taken from the TLS client certificate, into `devices`.
4. The PC responds `200`:

   ```json
   { "name": "Desk PC", "deviceId": "<pc-device-id>", "protocol": 1 }
   ```

5. The phone saves the PC as trusted and opens the WebSocket.

Errors: `401 bad_token`, `401 token_expired`, `401 token_used`, `426 unsupported_version`. After 5 failed attempts in a minute, the PC invalidates the token. Pairing an already-paired phone again just updates its record.

## Control channel: WebSocket

`GET /v1/ws` upgrades to a WebSocket. Each device has at most one, and a new connection replaces the old one.

All messages are JSON text frames and have a `type`. Messages that need an ack also have an `id`: a unique string from the sender, such as a UUIDv7. The receiver ignores IDs it has already processed but still acks them.

| Type | Direction | Fields | Receiver does |
|---|---|---|---|
| `hello` | Both, first message | `id`, `protocol`, `name`, `platform`, `appVersion` | Checks the protocol version. On a mismatch, sends `error` (`unsupported_version`) and closes |
| `ack` | Both | `ref` (the acknowledged ID) | Marks the item delivered |
| `error` | Both | `ref`, `code`, `message` | Marks the item failed and shows the error |
| `text` | Both | `id`, `text`, `isUrl` | Copies to the clipboard, notifies, stores in history. Links get an "Open" action |
| `clipboard` | PC → phone | `id`, `text` | Sets the clipboard silently. Not stored in history |
| `offer` | PC → phone | `id` (transfer ID), `files[]` of `{fileId, name, size, mime}`, `totalSize` | Acks, then downloads each file (see below) |
| `complete` | Phone → PC | `id` (transfer ID) | Marks the transfer done |
| `cancel` | Both | `id` (transfer ID), `reason` | Stops the transfer and deletes partial files |
| `unpair` | Both | `id` | Removes the sender from `devices` and closes the connection |

- Use `text` for explicit sends (share sheet, text menu, hotkey) and `clipboard` for automatic sync.
- A message can be at most 1 MiB. Longer text is sent as a `.txt` file.
- Unknown message types get an `error` with code `unsupported_type`.
- Liveness uses WebSocket ping/pong frames, not JSON messages.

## Files: phone → PC

1. `POST /v1/transfers`

   ```json
   { "id": "<transfer-id>", "files": [ { "fileId": "f1", "name": "IMG_0042.jpg", "size": 3145728, "mime": "image/jpeg" } ] }
   ```

   The response `200` gives, for each file, how many bytes the PC already has (0 for a new transfer, more when resuming):

   ```json
   { "id": "<transfer-id>", "files": [ { "fileId": "f1", "offset": 0 } ] }
   ```

   Errors: `507 disk_full`, `413 too_large`.

2. `PUT /v1/transfers/{id}/files/{fileId}?offset=N`. The body is the raw bytes from offset N to the end, and `Content-Length` = size − N.
   - `200 { "received": <size> }` means the file is complete.
   - `409 offset_mismatch { "offset": <actual> }` means the client should retry from `actual`.

3. Files are sent one at a time, in order. The transfer is done when every PUT has returned 200.

4. `DELETE /v1/transfers/{id}` cancels the transfer.

## Files: PC → phone

1. The PC sends an `offer` over the WebSocket. The phone acks it and accepts automatically, because the PC is paired.
2. `GET /v1/transfers/{id}/files/{fileId}`, with `Range: bytes=N-` when resuming. The PC responds `200` or `206 Partial Content`.
   - `404 not_found`: the offer expired or was canceled.
   - `409 source_changed`: the file's size or modified time changed since the offer.
3. When every file is saved, the phone sends `complete`.
4. Offers expire after 24 hours, and then the PC stops serving them.

## File names

The receiver cleans every file name before saving:

- Remove any path, keeping only the final part after `/` or `\`.
- Replace `< > : " / \ | ? *` and control characters with `_`.
- Strip trailing dots and spaces.
- Prefix Windows reserved names (`CON`, `PRN`, `AUX`, `NUL`, `COM1`–`COM9`, `LPT1`–`LPT9`) with `_`.
- Limit to 200 characters, keeping the extension.
- If a file with the same name exists, add ` (1)`, ` (2)` and so on.
- If the name ends up empty, use `file`.

## Error codes

| Code | HTTP status | Meaning |
|---|---|---|
| `unauthorized` | 401 | Device isn't paired |
| `bad_token` / `token_expired` / `token_used` | 401 | Pairing failed |
| `unsupported_version` | 426 | Protocol versions don't match |
| `not_found` | 404 | Transfer or file not found |
| `offset_mismatch` | 409 | Upload offset is wrong; retry from the offset returned |
| `source_changed` | 409 | The file changed after it was offered |
| `too_large` | 413 | Over a size limit |
| `disk_full` | 507 | Not enough free space |
| `unsupported_type` | (WebSocket) | Unknown message type |
| `internal` | 500 | Bug; check the logs |

## Versioning

- The version appears in the path (`/v1/`) and in `hello.protocol`.
- New optional fields can be added within v1. Receivers ignore fields they don't know.
- A breaking change means v2. In that case the apps show "Update the app on your phone/PC".
