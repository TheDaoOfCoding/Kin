# Windows app

## Basics

- **Tauri v2.** The Rust backend does all the real work. The TypeScript UI is only for windows: pairing, sending, history, settings and debug.
- Windows 10 and 11, x64. ARM64 comes later.

## Rust crates (planned)

| Job | Crate |
|---|---|
| Async runtime | `tokio` |
| HTTPS and WebSocket server | `axum` (with its WebSocket support), `rustls`, `tokio-rustls` |
| Certificates | `rcgen` |
| Encrypting the private key | DPAPI through the `windows` crate |
| mDNS advertising | `mdns-sd` |
| Database | `rusqlite` (bundled SQLite) |
| Clipboard read and write | The Win32 clipboard API through the `windows` crate: `AddClipboardFormatListener` on a hidden message-only window for the change events, `CF_UNICODETEXT` for the text, and `EnumClipboardFormats` for the opt-out formats. (`arboard` was in this plan. It would only have covered the text, and the listener and the format checks need Win32 anyway, so it was left out: one dependency fewer.) |
| QR rendering | `qrcode` (rendered as SVG for the UI) |
| Decoding a received image | The Windows Imaging Component (WIC) through the `windows` crate. No image crate is added (see [Images](#images-and-the-clipboard)) |
| The file dialog and the free space | `IFileOpenDialog` and `GetDiskFreeSpaceExW`, also through the `windows` crate |
| Tray, hotkey, notifications, autostart, single instance | Tauri's built-in tray, plus the `global-shortcut`, `notification`, `autostart` and `single-instance` plugins |
| JSON | `serde`, `serde_json` |

Handlers need to know which device made each request, but axum doesn't pass the client certificate to them by default. That needs a custom TLS accept loop that attaches the peer certificate to every request. It's a known pattern, and it gets built in 1.1. Two rules from [protocol.md](protocol.md#transport) apply to it. The client certificate is mandatory in the handshake, and the custom verifier accepts any chain but must still verify the TLS 1.3 signature (`verify_tls13_signature`), so a replayed public certificate can't pass. And axum's default `Json` extractor answers `415`/`422` in plain text, so the app uses its own extractor that returns `400 bad_request` JSON (see [protocol/README.md](../protocol/README.md#notes-for-the-implementers)).

## Process model

- One process. It starts when you sign in and lives in the tray.
- Windows are created when opened and **destroyed** when closed, not just hidden. An open WebView2 costs tens of MB, and destroying it keeps idle memory low.
- Single instance: a second launch (for example from "Send to") passes its arguments to the running instance and exits.

## Tray

- The icon shows whether the phone is connected.
- **Left-click** opens the drop window. Windows doesn't support dropping files onto a tray icon, so this window is the drop target.
- **Menu:** status line, Send files…, Send clipboard to phone, Pair new phone, Open received folder, History, Settings, Quit.

## Sending to the phone

- **Drop window:** the tray's left click opens it (`Kin.exe --drop` does the same). Drag files from Explorer onto it, or click it (or the tray's "Send files…") to choose files in the Windows file dialog. It also shows what is going on: one row per transfer with a progress bar, the speed, and a **Cancel** button, and the last few finished ones. The window is destroyed when it is closed. The tray's tooltip follows the numbers too ("Kin: Pixel 8 connected, sending 45%").
- **Explorer "Send to":** a shortcut in `%APPDATA%\Microsoft\Windows\SendTo` runs the app with `--send <paths>`; Explorer adds the selected files to its command line. The running instance gets them (the single-instance rule: a relative path is resolved against the folder the second launch was started in) and the second process exits. On Windows 11 it's under "Show more options". The modern menu entry comes in Phase 2. **The installer creates the shortcut in 1.5.** Until then `desktop\make-sendto-shortcut.ps1` makes it by hand (give it `-Folder` to try it in a test folder first, `-Remove` to take it out). Nothing creates it on its own.
- **What gets sent.** The files are *offered* over the WebSocket (`offer`) and the phone downloads them from the PC, which reads them from where they are (nothing is copied, and a file that changes after the offer makes the phone's request fail with `source_changed`). An offer lasts 24 hours. A phone that isn't connected gets nothing and the user is told (the queue that sends later is 1.4). More than 800 files go in several offers.
- **Folders are not sent in 1.3.** A dropped folder is skipped with a message ("A folder can't be sent yet"), and the files that were dropped with it are sent. Flattening a folder would lose its structure and clash names, and sending the structure needs a protocol addition.
- **Progress and cancel:** in the drop window, and `status.json`. Cancel sends a `cancel` message (reason `user`) and the partial files on the receiving side are dropped.
- **Hotkey** (default `Ctrl+Alt+Shift+K`, configurable): sends the current clipboard text as a `text` message. The tray's "Send clipboard to phone" and `kin.exe --send-clipboard` do the same. See [Hotkey](#hotkey). Text goes out with **LF line ends** and, if it is **over 128 KiB**, as a `Text.txt` file through the same offer path (a temporary file of ours in `outgoing\`, removed when the phone has it or the offer expires).
- **Automatic clipboard sync** (on by default, can be turned off): each clipboard change sends a `clipboard` message.
  - Debounced by 300 ms.
  - Text only in the MVP.
  - **Skips password-manager content.** That means content carrying any of the formats apps use to opt out: `ExcludeClipboardContentFromMonitorProcessing`, or `CanIncludeInClipboardHistory` / `CanUploadToCloudClipboard` set to 0.
  - Over 128 KiB: not sent, and `status.json` says `too_large`.
  - **Loop prevention:** remember a hash of the text last set from the phone, and ignore the change event it causes. The hash is of the **wire form** (LF line ends), so the CRLF text on the clipboard and the LF text the phone sent are the same text.

### Hotkey

**Default: `Ctrl+Alt+Shift+K`** ("K" for Kin). It is picked to stay out of the way of what people already use:

- **Windows keeps every `Win+<key>` for itself** (`Win+V` is the clipboard history, `Win+Shift+S` the snipping tool, `Win+K` casting, and so on), so none of those is used.
- **Two-modifier shortcuts are what apps use** (`Ctrl+Shift+K` opens the web console in Firefox, for one, and editors bind many `Ctrl+Alt+<letter>` commands). A global hotkey wins over the app in front, so it would break those. With three modifiers it sits where far fewer commands are bound.
- **`Ctrl+Alt+<letter>` is `AltGr+<letter>` on many keyboard layouts**, which is how they type characters (`€`, `@`, Polish `ł` and `ś`, and so on). A global `Ctrl+Alt+<letter>` takes that character away from the people who type it. Adding `Shift` keeps it clear of the common ones.
- **Checked on the development PC:** Windows accepted the registration, so no other program holds it there. The check is repeated on every start (a program that holds the key makes the registration fail, which shows in the log, in `status.json` and in a toast). Every program on every PC can't be checked, which is why it is configurable.

**Changing it.** Put the key in `%APPDATA%\Kin\settings.json` (a settings window comes in 1.4):

```json
{ "autoClipboardSync": true, "hotkey": "Ctrl+Alt+Shift+F9" }
```

The syntax is the accelerator syntax of the Tauri global-shortcut plugin: modifiers `Ctrl`, `Alt`, `Shift` and `Super`, a letter, a digit or a name such as `F9`, joined with `+`. An empty string turns the hotkey off. The change takes effect the next time the app starts. If the text isn't a hotkey, or another program already has it, the app starts without a hotkey and says why once (a toast, the log, and `hotkey.error` in `status.json`).

**What the hotkey does.** It reads the clipboard at the moment of the press and sends the text as a `text` message, which the phone treats as an explicit send (a notification, and later an entry in the history). Two rules differ from the automatic sync, because someone pressed a key on purpose: it also sends content that a password manager marked "don't record" (the person asked) **with `sensitive` set**, so the phone shows no preview and keeps it out of its history (a sensitive text over 128 KiB is not sent at all: a file can't carry the mark), and a failure (the phone isn't connected, no `ack` within 5 s) gets a toast. The toast on success is "Sent to <phone>". A clipboard with no text gets "The clipboard has no text to send."

## Receiving from the phone

- **Files** go to the Downloads folder of the user, in `Kin\` (Windows is asked where Downloads is, so a moved folder works), or to `receivedFolder` of `settings.json`. Each one is streamed to `name.ext.part` (never held in memory, 1 MiB at a time) and, when the last byte is in, `fsync`ed, checked to be exactly as long as announced, marked with the Mark of the Web (see [What a received file may do](#what-a-received-file-may-do)) and moved to `name.ext` **without ever replacing a file** (`MoveFileExW` without `MOVEFILE_REPLACE_EXISTING`; a name that is taken gets ` (1)`, ` (2)`, …, up to ` (99)`, also when it was taken during the transfer; if all 100 names are taken the file fails with `internal`). Names are cleaned by the rules of protocol.md ("File names"). **Finishing a file does not depend on the request:** the `fsync`, the mark and the move run in a task of their own, with the database told the name before the move and the result after it, so a phone that gave up waiting and asks again finds the file complete (never a second copy), and the `PUT` is answered as soon as the file has its final name. The toast and the image for the clipboard come after that, in another task: a slow decode or a busy clipboard never holds the phone's `200`. Folders deeper than 259 characters work: the final move and the disk-space check use the `\\?\` form of a long path.
- **Resume.** A `.part` file is kept for 24 hours after its last progress, and the transfer continues from the bytes that are there (the smaller of what the database says and what is on disk). A clean-up at start and every hour drops transfers with no progress for that long (their `.part` files go), offers that expired, and the `.part` files that the database still assigns to a transfer that is over. **It deletes only paths a record names:** a `.part` file in the received folder that no record names (a browser download, a file the user saved) is the user's and is never touched, and neither is a finished received file, even one that is named `*.part`. A `.part` file belongs to the transfer whose record names it: the names of unfinished transfers count as taken even when their file is missing, so another transfer never ends up with the same `.part` file and a resume never adopts a file the database doesn't assign to it. **A transfer ID is never used twice:** the records of finished, canceled and failed transfers stay in the database; a finished ID answers (the offsets, or `200` to a repeated empty `PUT`) for 24 hours and is then `404`, and a canceled or failed ID is `404` at once.
- **When the last file of a transfer has arrived:** a toast "<n> file(s) from <phone>". **Clicking it** opens the file (one file) or the folder (several), but only a file of a type on the **allow-list** below: any other type is never opened by a click, its folder opens with the file selected instead.
- **Images** (what is an image is decided by the content, not by the name or the type the phone gave) are also put on the clipboard (on by default; `imagesToClipboard` in `settings.json`), so Ctrl+V works right away. See [Images and the clipboard](#images-and-the-clipboard).
- **Text and links** go to the clipboard, with a toast. Clicking a link's toast opens it in the browser. A text the sender marked **sensitive** goes to the clipboard with the formats that keep it out of the clipboard history and the cloud clipboard (`ExcludeClipboardContentFromMonitorProcessing`, `CanIncludeInClipboardHistory` = 0 and `CanUploadToCloudClipboard` = 0), the toast says only "Text from <phone>" with no preview and nothing to open, and nothing about it (not even its length) goes to the log or `status.json`. Text lands with CRLF line ends.

### Toast buttons (R10)

**Finding (checked on Windows 11 with the real app, 2026-10-15): the buttons work for an app that isn't packaged.** A toast for one received file shows **Open** and **Show in folder** buttons (only **Show in folder** if the file's type isn't on the allow-list), a toast for several files **Open folder**, and a click on a button reaches the app (verified: a click on "Show in folder" logged `showed the received file in its folder` and opened Explorer with the file selected). The rules that come with it:

- A click or a button press is reported **only while the toast is on screen**. Once it has moved to the Action Center, pressing it does nothing (the notification plugin keeps the handler in the running process; there is no registered activator to start the app again). That is why the *file is always also in the folder and the log says where*.
- An unpackaged app has no app ID of its own, so Windows shows the toast under **"Windows PowerShell"** with PowerShell's icon. That is cosmetic and goes away once the installer of 1.5 registers an app ID with a Start Menu shortcut. The buttons don't depend on it.
- A click that doesn't say which toast it was opens nothing (1.2 used to open the newest link, which could open a link the user hadn't clicked).

So toasts are **not** click-only: the buttons are on. If a Windows update ever stops reporting button presses, the click on the toast itself does the first button's job, and that part is the same code.

### What a received file may do

Any app on the phone can send a file to the PC (R13), so a received file is as untrusted as a download. Two rules keep a click from running it:

**1. The Mark of the Web.** Every received file gets the NTFS alternate data stream `Zone.Identifier` with `[ZoneTransfer]`, `ZoneId=3` (the Internet zone) and `HostUrl=about:internet`, written on the `.part` file just before the final move (so the file has it from the moment it has its name). **SmartScreen and Office Protected View now apply to received files**, as they do to a browser download: a program asks for confirmation (and the "unknown publisher" warning), a Word or Excel file opens in Protected View with macros and active content off until the user allows them, and Explorer shows "This file came from another computer" in its properties. A volume with no alternate data streams (FAT, exFAT, some network shares) can't carry the mark: the file is received all the same and the log says so, with no name in it.

**2. An allow-list for "Open".** A click on the toast (or its **Open** button) opens a single received file with its default program only if its **last** extension, compared ASCII case-insensitively, is one of:

| Kind | Extensions |
|---|---|
| Pictures | `jpg` `jpeg` `png` `gif` `webp` `heic` `heif` `bmp` `tiff` |
| Audio and video | `mp3` `m4a` `aac` `flac` `wav` `ogg` `opus` `mp4` `mov` `mkv` `webm` `avi` `3gp` |
| Documents | `pdf` `txt` `csv` `md` `docx` `xlsx` `pptx` `odt` `ods` `odp` |

`csv` and `md` open as text (`csv` with whatever program the user has for it). The macro-enabled Office types (`docm`, `xlsm`, `pptm`, …) are not on the list. **Everything else gets "Show in folder"** (Explorer opens with the file selected) and the toast has no Open button: programs, scripts and anything another program registers for itself (on a developer's PC `.sh` opens in Git Bash and `.pyz` runs under Python), shortcuts and installers, archives, and files with no extension. Only the last extension counts, so `report.pdf.exe` is not openable and `report.exe.pdf` is a PDF; trailing dots and spaces are ignored (Windows ignores them too), and a name that holds a stream (`:`) is never openable. (The first 1.3 version had a deny-list; it missed `.sh`, `.pyz`, `.pyc`, `.wsb`, `.theme`, `.ahk`, `.appinstaller`, `.website`, `.vsto`, `.jnlp` and more, and could not be kept complete. The mark is the second line of defence: the Open button on an allow-listed file still hands it to SmartScreen and Protected View.)

### Images and the clipboard

An image from the phone is put on the clipboard so that Ctrl+V works in the apps people paste into. **Exactly one image in a transfer** goes there (several images leave the clipboard alone: one clipboard can't hold them as pictures, and picking one at random would be a surprise). The formats:

| Format | Why |
|---|---|
| `CF_DIBV5` | A 32-bit BGRA bitmap with an alpha channel. Windows builds `CF_DIB` and `CF_BITMAP` from it on demand, so Paint, Word, Outlook, Photoshop and any app that asks for a plain bitmap gets one, and an app that wants alpha gets alpha |
| `PNG` | The original PNG bytes, if the file is a PNG: Chrome, Edge, Firefox, Teams, Slack and Office prefer it |
| `CF_HDROP` | The path of the file in the received folder: Explorer pastes the file, and chat apps attach the original file (full quality, with its name) instead of a bitmap |

The picture is decoded with the Windows Imaging Component (PNG, JPEG, BMP, TIFF, the first frame of a GIF, and WebP, HEIC or AVIF where the user has the codec): no image library is added to the app. **A file is an image if the decoder opens it as one**, whatever its name or MIME type says, and the `PNG` format is written when the decoder says the container is PNG. **A phone photo has its rotation in the EXIF data and not in the pixels**, and an app that reads a bitmap never looks at EXIF, so the rotation (all eight orientations) is applied while decoding. The picture is read **row by row, in order**, and every pixel is written straight to its place in the bottom-up bitmap (no WIC flip-rotator, which asks a JPEG decoder for a scanline per pixel and took over 400 s for a 12 MP photo with orientation 6; now about 45 ms, as fast as an unrotated one). **Memory is bounded:** the pixels are decoded straight into the one block of memory that becomes the clipboard's `CF_DIBV5`, 4 bytes a pixel, and an image that needs more than 64 MiB of pixels (about 16 megapixels: a 12 MP phone photo fits, a 48 MP one doesn't) is not put on the clipboard, with the reason in the log. The same limit applies to the `PNG` format (a file over 64 MiB). Verified with the real clipboard: an 8 × 4 PNG with transparency pastes as 8 × 4 with its colours (and the PNG format holds the original bytes), and a JPEG stored sideways with EXIF orientation 6 pastes upright.

## Install

- **Tauri NSIS installer, per-machine.** It needs admin rights to add the firewall rule:

  ```
  netsh advfirewall firewall add rule name="Kin Relay" dir=in action=allow program="<install path>\kin.exe" enable=yes profile=private,public
  ```

  The rule allows both the Private and Public profiles (D12). Windows 11 marks new networks Public by default, and every request is authenticated (see R8 in [risks.md](risks.md)). The uninstaller removes the rule.
- Also adds the "Send to" shortcut and start-with-Windows.
- **Code signing:** unsigned in the MVP, so SmartScreen shows a warning. Sign before the public launch.

## Data locations

`%APPDATA%\Kin\` holds:

- Settings (`settings.json`: `autoClipboardSync`, `hotkey`, `receivedFolder`, `imagesToClipboard`)
- The private key (DPAPI-encrypted)
- The SQLite database (`devices`, the memory of message IDs, and the `transfers` and `transfer_files` tables of [architecture.md](architecture.md#storage))
- `outgoing\`: temporary files of ours (the `Text.txt` of a long text), each in its own folder, removed when the offer ends
- Logs (rotating, 5 files of 1 MB)

Received files are not in there: they go to the received folder (`Downloads\Kin` by default).

## UI

- Vite with TypeScript, using plain DOM or Svelte. No component library.
- Windows: Pairing (QR), Drop/send, History, Settings, Debug.
