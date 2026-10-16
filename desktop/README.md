# Kin Relay for Windows (desktop)

The Windows app: Tauri v2, a Rust backend and a small TypeScript UI (plain DOM, no component library). **This is milestone 1.5 (packaging) on top of 1.4:** the NSIS installer (per-machine; the firewall rules, the Start Menu shortcut with the app user model ID, uninstall with an option to delete the data), the parts of the install that are the user's and so are made by the app (the "Send to" shortcut, "Start with Windows" on by default: `integration.rs`), and the scripts that check an install (`verify-install.ps1`, `verify-uninstall.ps1`). See docs/windows.md "Install" and docs/results/phase-1-5-desktop.md. Milestone 1.1 was identity, the HTTPS server with mutual TLS, pairing by QR code, mDNS, the WebSocket with `hello`, `ack`, `error`, `ping` and `unpair`, a tray, and the logs. 1.2 added text, links and the clipboard. 1.3 added files in both directions. **1.4 adds the queue, the history, the settings and the debug window:** every explicit send (a `text`, files) is stored as `queued` before it is tried and goes out in order when the phone is back (nothing is queued for the automatic clipboard or for a `sensitive` text); a history window lists the recent transfers both ways (click to open or copy, Retry, Cancel); a settings window (device name, clipboard sync, images, received folder, the hotkey with conflict detection, start with Windows, unpair); a debug window (the first-try success rate of sends, recent errors, latency, the connection history, the network profile of this PC, and Export logs); and the tray menu of docs/windows.md, with "Phone offline – last seen <time>". The contract is [docs/protocol.md](../docs/protocol.md) (see its [Queue](../docs/protocol.md#queue) section), the plan is [docs/phases/phase-1-mvp.md](../docs/phases/phase-1-mvp.md), and what was measured is in [docs/results/phase-1-1-desktop.md](../docs/results/phase-1-1-desktop.md), [phase-1-2-desktop.md](../docs/results/phase-1-2-desktop.md), [phase-1-3-desktop.md](../docs/results/phase-1-3-desktop.md) and [phase-1-4-desktop.md](../docs/results/phase-1-4-desktop.md).

## Layout

```
desktop/
  index.html, src/          the UI: the pairing window (src/pairing.ts), the drop window (src/drop.ts) and, since 1.4, the history (src/history.ts), settings (src/settings.ts) and debug (src/debug.ts) windows, told apart by `?w=` in the URL (src/main.ts); src/dom.ts has the small helpers they share
  src-tauri/
    src/
      main.rs, lib.rs      entry point, module list, `run()`
      cli.rs               command line and KIN_* environment: data dir, developer settings, commands
      filelog.rs           rotating log files (5 x 1 MB)
      encoding.rs          base64url, base32, device ID, short ID, tokens, UUIDv7
      identity.rs, dpapi.rs   P-256 key + certificate (rcgen), key kept DPAPI-encrypted
      tls.rs               TLS 1.3 config, the client certificate verifier (P-256 only, signature always checked, one SPKI for the signature and the device ID)
      server.rs            accept loop (attaches the device ID to every request), connection limits and timeouts, POST /v1/pair, GET /v1/ws, 404, the route table
      pairing.rs           tokens (single use, 5 min, 5-failure limit), the pairing URL, the QR code as SVG
      netaddr.rs           which addresses go into the QR code
      json.rs, messages.rs strict JSON rules, and the WebSocket receiver as a pure state machine (1.3: `offer`, `complete`, `cancel`, `sensitive`)
      ws.rs, connections.rs  one WebSocket per device: hello timer, ping, unpair, close codes, replacing
      storage.rs           SQLite: `devices`, the memory of message IDs, and (1.3) `transfers` and `transfer_files`
      mdns.rs              _kinrelay._tcp advertising
      core.rs              shared state and what the app can do, with no UI in it; emits events
      sync.rs              1.2/1.3: sending `text` and `clipboard` (sensitive flag, 128 KiB threshold, LF), the debounced clipboard loop, receiving `text`, toast targets, dev hooks
      clipboard.rs         the opt-out format checks, loop guard, debounce, text helpers; the Win32 clipboard (hidden listener window) and an in-memory one for tests
      links.rs             what counts as an openable URL (`isUrl`), opening links, files and folders from a toast (a file only if its type is on the allow-list)
      settings.rs          `settings.json` (clipboard sync, hotkey, received folder, images to clipboard)
      filenames.rs         1.3: the File names rules, create-new, the no-replace move (`MoveFileExW`)
      transfers.rs         1.3: transfer state, the phone to PC side (POST/PUT/DELETE, streaming, resume, checkpoints, finalize, cancel), recovery, clean-up, views
      outgoing.rs          1.3: PC to phone: `--send`, the drop window, offers, `GET` checks, cancel, the long text as `Text.txt`
      fileserver.rs        1.3: the HTTP handlers of the four file endpoints, and the streaming body of a `GET`
      arrival.rs           1.3: what happens when files arrive: the toast, and the image to the clipboard
      images.rs            1.3: decoding an image (WIC, row by row, EXIF rotation done by our own code) into the clipboard formats; telling an image by its content
      motw.rs              1.3 review: the Mark of the Web (`Zone.Identifier`) written on every received file
      paths.rs             1.3: the Downloads folder, free disk space
      queue.rs             1.4: the queue (protocol.md "Queue"): items stored as `queued` first, the flush in creation order after the `hello`, the 5 s ack timeout that re-queues, expiry with an explicit `now`, cancel, retry
      queuedb.rs           1.4: the database side: `transfer_texts`, `sends`, `connection_log`, the queue and history queries
      history.rs           1.4: the history window's list, what a click does (the allow-list rule, copy a text), Cancel
      prefs.rs             1.4: what the settings window changes (device name, received folder, ...) and its view
      autostart.rs         1.4: "Start with Windows" behind a trait: the current user's Run key (quoted path, HKCU only, release builds only) and the file stand-in that debug builds, developer runs and tests use instead of the registry
      integration.rs       1.5: what the installer leaves to the app because it must be done as the signed-in user: the app user model ID of the process (`app.kinrelay.desktop`), the "Send to" shortcut `Kin.lnk` (COM, made or repaired at every start), and "Start with Windows" on by default for a new profile; what the person wants is in `integration.json`. Never in a developer run or from the build folder
      privacy.rs           1.4 review: the keyed (HMAC-SHA256, per-installation DPAPI key) tags that log lines use for texts and file names
      metrics.rs           1.4: the first-try success rate, latency numbers, the debug window's view
      export.rs            1.4: "Export logs": a stored-method zip written by hand, and the redaction of paths (long and 8.3 short profile path, any C:\Users\<name>, the received folder wherever it is), IPv4 and IPv6 addresses, names (any letter case) and Wi-Fi network names
      netprofile.rs        1.4: the network profile (Private / Public) from the Network List Manager (COM)
      localtime.rs         1.4: local time for "last seen" (Windows converts, the wording is plain code)
      app.rs, tray.rs, ui.rs   the Tauri side: start-up, single instance, tray (the menu of windows.md), global hotkey (changed from the settings window), toasts and clicks on them, windows, UI commands
    windows/hooks.nsh      1.5: the installer hooks (the firewall rules, the uninstall steps): `bundle.windows.nsis.installerHooks` in tauri.conf.json
    tests/                 vectors.rs, transfer_vectors.rs, tls_vectors.rs, fake_phone.rs, transfers_e2e.rs, offers_e2e.rs, review_1_3.rs, server_limits.rs, queue_1_4.rs, review_1_4.rs, parallel_files.rs (v0.1.1: 4 files of one transfer at once, both directions) (+ common/)
    examples/fake_phone.rs a fake phone for manual checks and for the scripts
  clipboard-backup.ps1     save and restore every format of the real clipboard (used by the check and e2e scripts). v0.1.1: `Save-Clipboard` keeps an earlier backup that a crashed run left behind instead of overwriting it
  run-real-clipboard-test.ps1  v0.1.1: runs the one unit test that uses the real clipboard (`clear_if_text`) between Save-Clipboard and Restore-Clipboard
  check-1-3.ps1            the 1.3 checks against the real Kin.exe
  check-rotation.ps1       a 12 MP photo with EXIF rotation (orientations 6, 3 and 8) uploaded to the real Kin.exe: how long the `PUT` and the clipboard take, and the corners of the picture
  check-dragdrop.ps1       a real drag and drop onto the drop window
  make-sendto-shortcut.ps1 creates (or removes) the "Send to" shortcut by hand (an installed Kin makes it itself)
  verify-install.ps1       1.5: run after the installer (as the signed-in user, no admin needed): the firewall rules, the shortcuts and their app user model ID, the Run value, the files; `-Toast` shows a test notification
  verify-uninstall.ps1     1.5: run after the uninstaller: all of that is gone, your data kept (`-ExpectDataDeleted` if you ticked the box)
  check-1-4.ps1            the 1.4 checks against the real Kin.exe: the queue (arrival after the phone's hello, order, a restart, cancel, expiry), the tray's "last seen", the settings (a hotkey conflict, the stand-in for start with Windows), the debug numbers, Export logs
  capture-windows.ps1      screenshots of the history, settings, debug, drop and pairing windows from the real Kin.exe (and the memory before and after they were opened and closed)
  measure-idle.ps1         idle memory of the tray-only app; `-OpenWindows` opens and closes every window first
```

## Build, test, run

Rust is installed per user. In a shell that predates the install, `. .\_env.ps1` puts `cargo` on the PATH.

```powershell
cd desktop
. .\_env.ps1
bun install
bun run build                       # the UI into dist/. cargo needs dist/ to exist (tauri embeds it)
cd src-tauri
cargo test                          # 375 tests (and 5 ignored): unit tests, the protocol vectors, and a fake phone over real TLS on loopback (the real clipboard and Downloads folder are never touched)
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cd ..
powershell -ExecutionPolicy Bypass -File build.ps1    # release build + NSIS installer (bun tauri build --bundles nsis)
```

**`cargo test --release` (and any `cargo build --release`) overwrites `target\release\Kin.exe` with a dev-server build** (a plain cargo build does not embed the pages: the window would load `http://localhost:1420`). After one, run `build.ps1` again before a script that starts the real app. `check-1-4.ps1` and `tools\e2e-1-*.ps1` refuse an exe that is newer than the installer in `bundle\nsis` (which `tauri build` writes just after the exe); `-Build` rebuilds it.

Outputs: `src-tauri\target\release\Kin.exe` and `src-tauri\target\release\bundle\nsis\Kin_0.1.1_x64-setup.exe` (3.6 MiB, per-machine, unsigned). **Running the installer needs administrator rights, and no script here runs it**: the generated NSIS script is `src-tauri\target\release\nsis\x64\installer.nsi` (the hooks are included from `src-tauri\windows\hooks.nsh`), and an install is checked by hand with `verify-install.ps1` (see docs/TESTING.md). If the bundler's own NSIS download times out, stage NSIS by hand as described in [phase-0-tauri.md](../docs/results/phase-0-tauri.md#snag-the-nsis-download-timed-out).

**The ignored tests.** `cargo test real_clipboard -- --ignored --test-threads=1` touches the **real clipboard** (it saves the text, uses the clipboard, puts the text back, and does nothing if there is more than plain text on it). The 2 GB runs are ignored because they move 2 GB through the disk (in a temporary folder that is deleted): `cargo test --release --test transfers_e2e large_upload -- --ignored --nocapture` and `cargo test --release --test offers_e2e large_download -- --ignored --nocapture` print `THROUGHPUT` lines.

**The scripts against the real app** (each starts the release `Kin.exe` with the developer settings below, a throwaway profile and a throwaway received folder, and stops and deletes everything it started):

```powershell
cd desktop
cd src-tauri; cargo build --example fake_phone; cd ..                       # the fake phone (the scripts use the release build if there is one)
powershell -ExecutionPolicy Bypass -File check-1-3.ps1                       # 39 checks: uploads (cut and resumed), the Mark of the Web, `--send`, the sensitive flag, a long text, images on the real clipboard, a full disk
powershell -ExecutionPolicy Bypass -File check-1-4.ps1                       # 57 checks: the queue, a restart, cancel, expiry, the tray line, the settings, the debug numbers, Export logs (uses the real clipboard: saved and restored)
powershell -STA -ExecutionPolicy Bypass -File capture-windows.ps1            # PNGs of the windows in %LOCALAPPDATA%\Temp\kin-capture-windows (moves windows to the front for a minute); -UpdateScreens writes the tracked docs\results\phase-1-4-desktop-screens instead
powershell -ExecutionPolicy Bypass -File measure-idle.ps1 -OpenWindows       # idle memory after every window was opened and closed
powershell -ExecutionPolicy Bypass -File check-rotation.ps1                  # 12 MP JPEGs with EXIF orientation 6, 3 and 8: times and upright check on the real clipboard (-Exe <path> to try another build)
powershell -STA -ExecutionPolicy Bypass -File check-dragdrop.ps1             # a real drag and drop onto the drop window (moves the mouse for a second)
```

`check-1-3.ps1`, `check-1-4.ps1`, `capture-windows.ps1` and `check-rotation.ps1` use the **real clipboard**: `clipboard-backup.ps1` saves every format of it first and puts them back at the end (also after a failure), and `-SkipClipboard` leaves those checks out.

**They run beside the user's installed Kin** (v0.1.1): every script gives its Kin a data directory of its own, so it is a separate instance (below), sends every command with `--data-dir`, and refuses only when a Kin is running on *its own* data directory. `check-dev-beside-installed.ps1` is the proof of that (31 checks: two test Kins beside the installed one, commands routed by data directory, marked clipboard text, the installed Kin unchanged). The shared code is `tools\kin-dev.ps1`; see [tools/README.md](../tools/README.md#kin-devps1).

`bun tauri dev` works too (Vite on `localhost:1420`), but the developer settings below are what you want for anything that listens.

## Developer and integration settings

Everything here is also available in a release build, so the integration agent can drive the real `Kin.exe` without clicking. **Every setting that has to do with the network is loopback only: with `--dev-bind` or `--dev-qr-address` the app never binds to anything but `127.0.0.1` (or `::1`), and it doesn't start mDNS** (which would open multicast sockets on every interface). No Windows Firewall prompt can appear. **Always give a test run `--data-dir` and `--dev-received-dir`**: without the second, received files go to the real `Downloads\Kin`.

### A test Kin beside the installed one

**An instance is its data directory** (`instance.rs`). The installed Kin's own directory, `%APPDATA%\Kin`, is *the* instance. Any other `--data-dir` (or `KIN_DATA_DIR`) is another instance with a key of its own, a short hash of the directory's normalised path (absolute, `..` resolved, long names, no `\\?\`, lower case: every spelling of one folder has one key, whether or not it exists yet). Everything that Windows would share between two copies of the program is made from the key, so the two never meet:

| Shared thing | What an instance with a key does |
|---|---|
| Single-instance lock (mutex, window class and name of `tauri-plugin-single-instance`) | Built from the bundle identifier, which the app replaces at start by `app.kinrelay.desktop.dev-<key>`. A second launch with the same `--data-dir` reaches this instance; a launch with another data directory is another instance (it starts), and a launch with none is the installed Kin's |
| App user model ID of the process and the origin of toasts (the notification plugin takes the identifier) | The same `app.kinrelay.desktop.dev-<key>`, never the installed Kin's. A copy that runs from `target\release` shows its toasts as "Windows PowerShell" as before (the plugin's rule for a build folder), one that runs from anywhere else under its own name, which Windows doesn't show: use `--dev-no-toasts` or `--dev-no-windows` for a quiet run |
| Port | Never the default 47821: the one from `--dev-port`, else any free port (a bind to `127.0.0.1:47821` can succeed on Windows beside the installed Kin's bind to every interface and would take loopback traffic) |
| Bind address | `127.0.0.1` unless `--dev-bind` gives another loopback address (a keyed copy listening on every interface would raise a Windows Firewall prompt for its program file) |
| mDNS | Never advertised, whatever the other flags say |
| Hotkey | The default `Ctrl+Alt+Shift+K` is the installed Kin's: an instance with a key does not register it (`status.json` says `registered: false` and no error); another key can be set with `--dev-set hotkey` |
| "Start with Windows", the Send to shortcut | Already the stand-in file and nothing (a `--data-dir` of its own is never "a normal run") |
| Web view data (cache, storage) | `<data dir>\webview`, not `%LOCALAPPDATA%\<identifier>`; it goes with the throwaway directory (the helper processes hold it for a moment after Kin quits: delete the directory with a retry, `Remove-DevDataDir` in `tools\kin-dev.ps1`) |
| The real clipboard | Every text and image the instance writes carries `ExcludeClipboardContentFromMonitorProcessing` (the format an installed Kin, 0.1.0 too, skips: it sends none of it to a phone, and Windows keeps it out of its history and the cloud clipboard) and `KinRelayDevTestContent`, which only an instance with a key reads: it then treats the content as ordinary text (its own sync, hotkey and echo guard behave as for any copy). A real password manager's mark, without the test format, still holds it back |

**Two refusals protect the installed Kin** (both exit with 1 and write the reason to `kin-startup-error.txt` in the temp folder):

- **`--dev-*` settings need a `--data-dir` of their own.** Without it they would run against the profile of the installed Kin (its identity, pairing and history).
- **A launch with no data directory hands nothing to a running Kin that is another program file.** `Kin.exe --quit` from `target\release` with the installed Kin running would otherwise quit the user's Kin. The program that owns the single-instance window is found the way the second launch finds it (`instance::running_program`) and compared with the launching file: the "Send to" shortcut and the Start Menu shortcut run the same file as the running one, so they are never refused.

To start one by hand: `Kin.exe --data-dir "%LOCALAPPDATA%\Temp\kin-mine" --dev-bind 127.0.0.1 --dev-port 49333 --dev-received-dir "%LOCALAPPDATA%\Temp\kin-mine\received" --dev-print-pair-url --dev-no-windows`, and give every later command the same `--data-dir`: `Kin.exe --data-dir "%LOCALAPPDATA%\Temp\kin-mine" --send a.jpg`, `... --quit`.

### Settings (first launch)

A flag wins over its environment variable.

| Flag | Environment variable | Meaning |
|---|---|---|
| `--data-dir <path>` | `KIN_DATA_DIR` | Use this folder instead of `%APPDATA%\Kin`: a throwaway profile with its own key, database and logs, **and a separate instance that runs beside the installed Kin** (see above). Every `--dev-*` setting needs it. Quote it if it has a space. If the app can't start (for example it can't create this folder), it writes the reason to `kin-startup-error.txt` in the temp folder and exits with 1 |
| `--dev-bind <ip>` | `KIN_DEV_BIND` | Bind the HTTPS server to this address. Only a loopback address is accepted, anything else is an error at start |
| `--dev-qr-address <ipv4>` | `KIN_DEV_QR_ADDRESS` | Put `<ipv4>:<port>` in the QR code and the pairing URL instead of the PC's own addresses (`10.0.2.2` is the PC's loopback as seen from the Android emulator). Implies `--dev-bind 127.0.0.1` when no `--dev-bind` is given |
| `--dev-port <n>` | `KIN_DEV_PORT` | Port to try first (default 47821 for the installed Kin's own data directory, **random for any other**). If it is taken, a random port is used. `0` means random. **Read the real port from the URL or from `status.json`** |
| `--dev-print-pair-url` | `KIN_DEV_PRINT_PAIR_URL=1` | Whenever a pairing starts (tray menu, `--pair` or `--dev-start-pairing`), write the URL to `pair-url.txt` in the data directory and to stdout |
| `--dev-no-windows` | `KIN_DEV_NO_WINDOWS=1` | Don't create windows and don't show toasts. A toast is written to the log as `toast: ...` instead (for "Text from <phone>" the log has the title and the length of the preview, never the text; for files, the title and the length of the names) |
| `--dev-received-dir <path>` | `KIN_DEV_RECEIVED_DIR` | **1.3.** Received files go here instead of `Downloads\Kin` (and instead of `receivedFolder` in `settings.json`), so a test never writes to the real Downloads folder. `status.json` has the folder in `transfers.receivedFolder` |
| `--dev-no-toasts` | `KIN_DEV_NO_TOASTS=1` | **1.4.** Windows are created as usual, but toasts are only written to the log. For screenshots of the windows and for `measure-idle.ps1 -OpenWindows` |
| `--dev-clock-offset <seconds>` | `KIN_DEV_CLOCK_OFFSET` | **1.4.** The app's wall clock runs this many seconds ahead (everything: expiry, the 24 hour windows, `last seen`). `--dev-clock-offset 604801` makes a queued text 7 days old at the next start, so expiry can be tested in a running app: the sweep at start fails it with `expired`. Zero in a normal run. **Every test of expiry in `cargo test` passes an explicit `now` to `Core::expire_queue` instead (a fake clock)** |
| `--dev-free-bytes <n>` | `KIN_DEV_FREE_BYTES` | **1.3.** Pretend the disk has this many free bytes. The check for `507 disk_full` (only the bytes the PC doesn't have yet have to fit, with no reserve) uses it, so a full disk is easy to test: `--dev-free-bytes 1000000` refuses a 5 MB upload and takes a 900 KB one |

**The developer clipboard commands (`--dev-set-clipboard`, `--dev-get-clipboard`) work only when the running instance was started with a developer setting** (any of the flags above). In a normal run they are refused and the log says so: any other program could otherwise launch `Kin.exe --dev-get-clipboard` and have the clipboard written to a file.

### Commands (any launch)

The single-instance plugin forwards a second launch's arguments to the running instance, which acts on them, and the second process exits. On a first launch the same commands run once the app is up.

| Command | What it does |
|---|---|
| `--ping` | Ping the connected phone. Same as the Ping button of the debug window (the tray menu no longer has it). The result is the toast, the log, and `lastPing` in `status.json` |
| `--dev-start-pairing` | Start a pairing without a window. With `--dev-print-pair-url` set at the first launch, the URL is in `pair-url.txt` |
| `--pair` | Open the pairing window (and start a pairing), like "Pair new phone" |
| `--send <path>...` | **1.3. Explorer's "Send to", headless.** Offer these files to the connected phone, like dropping them on the drop window. Every argument up to the next `--flag` is a path; a relative path is resolved against the folder the launch was started in. Folders are skipped with a message. **Without a connected phone the offer is queued** (1.4) and goes out when the phone is back (a toast says so). Progress is in `status.json` (`transfers.active`) |
| `--drop` | **1.3.** Open the drop window, like the tray's left click |
| `--history`, `--settings`, `--debug-window` | **1.4.** Open the history, settings or debug window (the tray's History and Settings, and Shift + click on the tray icon) |
| `--dev-dump-view <history\|queue\|settings\|debug>` | **1.4 developer hook** (refused unless the app was started with a developer setting). Writes what a window would show to `view-<name>.json` in the data directory, so a script can read it without a window. `queue` is the same as `queue` in `status.json` |
| `--dev-history <cancel\|retry\|open> <id>` | **1.4 developer hook.** What the history window's Cancel, Retry and click do to an item (its ID is in `view-history.json`). The outcome is in `dev-history-result.json` (`{ action, id, ok, message }`) |
| `--dev-set <setting> <value>` | **1.4 developer hook.** What the settings window does, without a window: `deviceName`, `autoClipboardSync`, `imagesToClipboard`, `receivedFolder`, `hotkey` (a conflict with another program is reported in the result) and `startWithWindows`. The outcome is in `dev-set-result.json` (`{ key, ok, message }`) |
| `--dev-export-logs <path>` | **1.4 developer hook.** "Export logs" without the dialog |
| `--cancel-transfers` | **1.3 (integration).** Cancel every transfer that is going, in both directions: what the Cancel button of the drop window does for each row (a `cancel` message with reason `user` goes to the phone, partial files go). Added so a script can cancel from the PC side without clicking |
| `--send-clipboard` | Send the text on the clipboard to the phone as a `text` message: **the hotkey, headless.** The tray item "Send clipboard to phone" and the hotkey run exactly the same code. The result is `lastSent.text` in `status.json` (with `trigger` `command`, `hotkey` or `tray`) and a log line. A clipboard that carried a password-manager format goes with `sensitive`; a text over 128 KiB goes as a `Text.txt` file offer |
| `--dev-set-clipboard <text>` | Developer hook: put this text on the PC clipboard, as if the user had copied it. It goes through the normal pipeline (change event, 300 ms debounce, automatic sync), so this is how to trigger a PC to phone `clipboard` message. Add `--dev-clipboard-mark exclude`, `no-history`, `no-cloud` or `viewer-ignore` to add the opt-out format a password manager would add (the text is then not sent automatically, and the hotkey sends it as sensitive). Refused unless the app was started with a developer setting |
| `--dev-get-clipboard` | Developer hook: write the PC clipboard text to `dev-clipboard.txt` in the data directory (UTF-8, exactly the text; the file is deleted if there is no text). Refused unless the app was started with a developer setting |
| `--quit` | Close the WebSockets (1000, "app quit"), stop listening, remove the tray icon and exit |

Unknown arguments are logged and ignored. **The log names a command and never prints its text.**

**Which clipboard hook to use.** For ordinary text, **PowerShell's `Set-Clipboard` and `Get-Clipboard` are the better choice**: they work on the real clipboard like any app does, so the whole path is tested, including the Windows change event, and they need no running Kin to answer. The two things PowerShell can't do are why the hooks exist: `--dev-set-clipboard ... --dev-clipboard-mark ...` sets the password-manager formats, and `--dev-get-clipboard` reads the clipboard *as Kin sees it* (UTF-16 to UTF-8, the same function that sends it). Note that a `text` from the phone is stored with `\r\n` line ends (the wire has LF), so `Get-Clipboard -Raw` shows `\r\n` where the phone sent `\n`. **Whatever you do to the real clipboard, save it first and put it back:** `clipboard-backup.ps1` does that for every format (an image, HTML and all), and `check-1-3.ps1` shows how.

```powershell
# PC -> phone, automatic: copy something. The phone gets a `clipboard` message about 300 ms later.
Set-Clipboard -Value 'hello from the PC'
# PC -> phone, explicit (the hotkey, headless):
& $kin --send-clipboard
# A password manager's copy is not sent automatically, and the hotkey sends it as sensitive:
& $kin --dev-set-clipboard 'hunter2' --dev-clipboard-mark exclude
# PC -> phone, files (Explorer's "Send to", headless). The phone must be connected.
& $kin --send C:\photos\a.jpg C:\photos\b.jpg
# Phone -> PC: after the phone sent a `text`
Get-Clipboard -Raw                              # or: & $kin --dev-get-clipboard ; Get-Content "$data\dev-clipboard.txt" -Raw
# What happened, with the times and the transfers:
(Get-Content "$data\status.json" -Raw | ConvertFrom-Json) | Select-Object lastSent, lastReceived, clipboardSync, hotkey, transfers | ConvertTo-Json -Depth 6
```

### Files in the data directory

| File | What |
|---|---|
| `status.json` | Rewritten on every state change, and **once a second while a transfer is going**. `state` is `not_paired`, `phone_connected`, `phone_offline` or `stopped` (written on exit). Also `port`, `bind`, `deviceId`, `shortId`, `pcName`, `pairing` (`null`, or `{ active, expiresAtMs }`: it is `null` as soon as the code expires or five wrong guesses shut it down), `devices[]` (`deviceId`, `name`, `platform`, `connected`, `pairedAt`, `lastSeen`), `lastPing` (`{ at, result, rttMs? }`) and `pid`, the 1.2 objects under [status.json in 1.2](#statusjson-in-12), and **`transfers`** under [status.json in 1.3](#statusjson-in-13). (A script that has the file open at the very moment it is replaced makes Windows say "access denied" to the app, which then tries again a few times) |
| `settings.json` | The settings: `{ "autoClipboardSync": true, "hotkey": "Ctrl+Alt+Shift+K", "receivedFolder": null, "imagesToClipboard": true, "deviceName": null }`. **1.4: the Settings window writes it** (every change applies at once). Missing keys are the defaults. `deviceName` null is the computer's name |
| `autostart-mock.json` | **1.4.** `{ "enabled": true }`: the stand-in for the Run key. **Only a release build** that runs with no `--dev-*` flag and the default data directory writes the real Run key (current user only, the quoted path of `Kin.exe`); a debug build, a developer run (any `--dev-*` flag, or a `--data-dir` of its own) switches "Start with Windows" here and **never touches the registry**; `status.json` and the settings view say which one is in use (`startWithWindowsIsReal`) |
| `integration.json` | **1.5.** `{ "startWithWindows": true, "sendTo": true }`: what the person wants of the "Start with Windows" value and the "Send to" shortcut, which an installed app puts right at every start (`integration.rs`). `startWithWindows` is written by the Settings switch (missing: not decided, a new profile gets `true`); `sendTo: false` removes the shortcut and keeps it away. A developer run writes it too but never makes a shortcut or a Run value |
| `view-<name>.json`, `dev-set-result.json`, `dev-history-result.json` | **1.4.** Written by the developer hooks above |
| `dev-clipboard.txt` | The clipboard text, after `--dev-get-clipboard` |
| `pair-url.txt` | The current pairing URL (only with `--dev-print-pair-url`). It holds the token, so it is never written to the log |
| `logs\kin.log` | Rotating log: `kin.log`, `kin.1.log` to `kin.4.log`, 1 MB each |
| `identity.key`, `identity.crt` | The DPAPI-encrypted PKCS#8 key and the certificate (public) |
| `kin.db` | SQLite: `devices`, `seen_messages`, `transfers` (1.4: status `queued`, kind `text` or `file`, `sent_at`, `retry_of`), `transfer_files`, `transfer_texts` (the body of an explicit, non-sensitive text), `sends` (one row per send for the success rate: kind, times, attempts, outcome; no text, no name) and `connection_log` (the last 200 connects and disconnects) |
| `outgoing\<id>\Text.txt` | A long text on its way to the phone as a file. Removed when the phone has it, the offer is canceled, or a day has passed |

### status.json in 1.2

```json
{
  "settings": { "autoClipboardSync": true, "hotkey": "Ctrl+Alt+Shift+K", "receivedFolder": null, "imagesToClipboard": true },
  "hotkey": { "accelerator": "Ctrl+Alt+Shift+K", "registered": true, "error": null },
  "lastSent": {
    "text":      { "id": "0199...", "at": 1760443200123, "result": "acked", "chars": 17, "trigger": "hotkey", "tapToAckMs": 41.3, "rttMs": 38.9, "sensitive": false, "asFile": false },
    "clipboard": { "id": "0199...", "at": 1760443199000, "result": "acked", "chars": 17, "trigger": "clipboard", "tapToAckMs": 352.8, "rttMs": 40.2, "sensitive": false, "asFile": false }
  },
  "lastReceived": { "text": { "id": "...", "at": 1760443201000, "chars": 20, "isUrl": false, "openable": false, "clipboardSet": true, "sensitive": false } },
  "clipboardSync": { "enabled": true, "sent": 3, "skipped": 1, "lastSkip": { "at": 1760443198000, "reason": "password_manager:ExcludeClipboardContentFromMonitorProcessing" } }
}
```

- **`result`** is `acked`, `no_reply` (no `ack` in 5 s: the connection was closed and the phone is offline), `phone_offline`, `no_phone_paired`, `error:<code>` (the phone answered with an `error`) or `too_large` (over 128 KiB: an automatic `clipboard`, or a `sensitive` text, which is not sent at all; a longer ordinary `text` goes as a file and `asFile` is `true`).
- **`tapToAckMs`** is the arrival time of targets.md: from the tap to the phone's `ack`. For the hotkey the tap is the key press (or the command arriving). For the automatic sync it is the **first** clipboard change of a burst, so the 300 ms debounce is included (target: under 1 s). **`rttMs`** is only from sending the message to the `ack`. Both are monotonic-clock measurements made on the PC, and the same two numbers are in the log (`acked, tap to ack 352.8 ms (round trip 40.2 ms)`).
- **`lastSkip.reason`** is `password_manager:<format>`, `echo_of_phone_text` (loop prevention: the text the phone just sent), `not_text`, `empty`, `too_large` or `read_failed`.
- **No clipboard text is in `status.json` or in the log.** Both have an id, a length, a result and times. A log line also carries the first 8 hex digits of the SHA-256 of the text, so one text can be followed through the log without reading it. **A sensitive text has none of that**: `chars` is `null`, and its log line says only "sensitive text".

### status.json in 1.3

```json
{
  "transfers": {
    "active": [
      { "id": "0199...", "direction": "in", "peer": "Pixel 8", "status": "active", "files": 3, "filesDone": 1,
        "bytesDone": 734003200, "totalBytes": 2147483648, "speedBps": 612345678.0, "error": null,
        "startedMs": 1760443200123, "finishedMs": null, "names": ["IMG_0001.jpg", "IMG_0002.jpg", "IMG_0003.jpg"], "cancelable": true }
    ],
    "recent": [
      { "id": "0198...", "direction": "out", "status": "done", "files": 1, "bytesDone": 3145728, "totalBytes": 3145728,
        "error": null, "startedMs": 1760443100000, "finishedMs": 1760443101500, "names": ["clip.mp4"] }
    ],
    "receivedFolder": "C:\\Users\\me\\Downloads\\Kin"
  }
}
```

- **`direction`** is `in` (phone to PC) or `out` (PC to phone). **`status`** is `active`, `done`, `failed` or `canceled`; `error` says why for the last three (`user`, `expired`, `disk_full`, `no_reply`, …). `speedBps` is the speed over the last few seconds (0 when nothing moved for 2 s). `active` is the live list; `recent` is the last 10 that ended, from the database.
- **Where to look when something doesn't arrive:** an upload is `active` with `bytesDone` growing; a received file is finished when its row is `done` and the file is in `receivedFolder` (the `.part` file is gone). An `out` transfer is `active` from the phone's `ack` of the `offer` until its `complete` (`done`), a cancel (`canceled`) or 24 hours (`failed`, `expired`).
- The log has one line per step (`upload <id> from device <short>: n file(s), bytes, into <folder>`, `file k is complete`, `is done`, `offer <id> to device ...`, `the user canceled transfer ...`). **The contents of files and the text of the clipboard are never in the log**, and file names are not either, with one exception found by the integration run: the toast "Sending <name> to <phone>" of a one-file offer is logged like every toast, so that name is in the log (`toast: Sending dup2.txt to ...`), and so is the name in "<name> can't be read, so it was not sent". The toast for received files has only a title ("1 file from Pixel") and the length of the names. A resumed transfer logs where it carries on: `upload <id>: file 1 continues from byte N of M` (phone to PC) and `offer <id>: file 1 is served from byte N of M` (a ranged `GET`).

### status.json in 1.4

```json
{
  "pcName": "Desk PC",
  "queue": { "count": 2, "items": [ { "id": "7a8b9c0d", "kind": "text", "createdMs": 1760443200000, "expiresMs": 1761048000000, "sentMs": null, "bytes": 27 } ] },
  "tray": { "status": "Phone offline – last seen 14:32 · 2 queued", "tooltip": "Kin: Pixel 8 offline, last seen 14:32 · 2 queued" },
  "lastSent": { "text": { "result": "phone_offline", "queued": true } }
}
```

- **`queue`** is what waits for the phone: how many, and for each item its (short) ID, its kind (`text` or `file`), when it was made, when it expires (a text 7 days after it was made; an offer 24 hours after it was sent, or 7 days after it was made if it never was) and when it was first sent. **No text and no file name.** (`--dev-dump-view queue` gives the same.)
- **`tray.status`** is the status line of the tray menu as it is right now (what a script can't read from the menu itself).
- **`lastSent.text.queued`** is `true` when the send did not get its `ack` at once and is waiting in the queue; `result` says why (`phone_offline`, or `no_reply` for a connection that gave no `ack` in 5 s).

### Running against the Android emulator

```powershell
$kin = 'C:\Dev\ecosystem-app\desktop\src-tauri\target\release\Kin.exe'
$data = "$env:TEMP\kin-emulator"            # a throwaway profile
# Start-Process doesn't quote the items of an argument array, so quote a path that could have a space in it.
Start-Process $kin -ArgumentList '--data-dir', ('"{0}"' -f $data), '--dev-bind', '127.0.0.1', '--dev-qr-address', '10.0.2.2', '--dev-received-dir', ('"{0}"' -f "$data\received"), '--dev-print-pair-url', '--dev-no-windows'
# wait until $data\status.json exists, then:
& $kin --dev-start-pairing                      # a second launch: forwarded to the running instance
$url = (Get-Content $data\pair-url.txt).Trim()  # kinrelay://pair?v=1&a=10.0.2.2:<port>&id=...&tk=...&n=<PC name>
adb shell am start -a android.intent.action.VIEW -d "'$url'"   # quote it: & is special in the shell
Get-Content $data\status.json                   # state: phone_connected once the phone has paired and sent hello
& $kin --ping                                   # then read lastPing in status.json, or the toast line in the log
& $kin --send C:\path\to\photo.jpg              # PC -> phone: the phone downloads it (the Android app's debug commands list what it received)
Get-Content $data\logs\kin.log -Tail 20
& $kin --quit
```

- A dev instance and a normally installed Kin share the single-instance slot (same app identifier). Quit one before starting the other.
- `10.0.2.2` is a valid address in the pairing URL (it is in `10.0.0.0/8`).
- The pairing token lasts 5 minutes. `--dev-start-pairing` again makes a new one (and the old one stops working).
- `Kin.exe --send` takes no files from a phone's point of view: the phone has to be connected (`state: phone_connected` in `status.json`).

### The fake phone

`cargo run --example fake_phone -- "<pairing URL>" --host 127.0.0.1 --seconds 60 ...` (use `--release` for speed: it hashes the files). Besides the 1.2 flags (`--send-text`, `--no-ack`, `--ping-every`):

| Flag | What |
|---|---|
| `--upload <file>` (repeat), `--upload-generated <bytes>` (repeat), `--upload-after <s>` | Upload these files (a generated file is a fixed pattern that is never written to disk) as one transfer, `--upload-after` seconds (default 3) after the `hello`. Prints `uploaded <name> bytes=<n> sha256=<hex> seconds=<s> mbps=<MB/s>` per file (the hash is of the whole file) and `upload done: <id>` |
| `--upload-cut <bytes>` | Drop the connection of the first file bigger than this after that many bytes, then resume (a new connection, a new `POST`, a `PUT` from the offset the PC reports) |
| `--offline-after <s>`, `--offline-for <s>` | **1.4.** Closes the WebSocket that many seconds after the `hello`, stays away for `--offline-for` seconds (default 5) and connects again with a new `hello`. Prints `going offline at=<unix ms>`, `reconnected at=<unix ms>`, and **one `arrival type=<type> id=<id> at=<unix ms> ms_since_hello=<n>` line for every message the PC sends**: that is how `check-1-4.ps1` sees what the PC queued arrive, in order |
| `--identity-file <path>`, `--skip-pairing` | **1.4.** The phone's key is kept in that file, so a later run is the same phone; `--skip-pairing` connects without pairing (it is paired already). Together they test a restart of the PC |
| `--save-dir <folder>`, `--download-cut <bytes>`, `--no-download`, `--refuse-offer <code>` | For an `offer` from the PC: where to save (default `fake-phone-downloads` in the temp folder), drop the first download after that many bytes and resume with `Range`, ack but don't download, or answer with an `error` of that code (such as `disk_full`) instead of an ack. Prints `downloaded <name> bytes=<n> sha256=<hex> ...` and sends `complete` |

## How it behaves

- **Phone to PC.** `POST /v1/transfers` creates the `.part` files at once (names cleaned by protocol.md, a taken name gets ` (1)`) after the checks of the spec in the spec's order: `401`, `413` (JSON over 1 MiB), `400`, `404` (another device's transfer, or an offer), `400` (different `fileId`s), `413` (a file over 1 TiB, or over 10,000 files), `507` (only the bytes the PC doesn't have yet, no reserve), `500`. A `PUT` is checked in the spec's order too (`offset` in ASCII digits only, `Content-Length` required, the offset must equal the bytes stored, `Content-Length` = size − offset, free space) and its body is streamed to the file in 1 MiB pieces with one write in flight while the next piece is read (never more than two pieces in memory). Every 32 MiB the data is flushed (`sync_data`) and the database brought up to date; when the file is complete it is `fsync`ed, checked to be exactly as long as announced, marked with the Mark of the Web (`Zone.Identifier`, `ZoneId=3`) and moved to its final name (`MoveFileExW` without replacing, with the `\\?\` form for a path over 259 characters; a name taken in the meantime gets the next suffix, up to ` (99)`). **That finish runs in a task of its own** (the database is told the name before the move and the result after it), so a phone that gave up waiting finds the file complete, never a second copy; the `PUT` is answered as soon as the file is final, and the toast and the clipboard image are made afterwards in another task. A connection that breaks keeps what arrived. A new request for a file that an older request is still writing stops the older one first (which first takes what has already arrived), so the offset is exact. `DELETE` and a `cancel` from the phone drop the partial files (complete ones stay); they are the same operation, done once (a second one finds it done), and serialised with a `PUT` and with the finish of the last file. **A transfer ID is never used twice** (see windows.md); a cancel reason from the phone is cleaned (no control characters, 32 characters at most) before it is logged or stored.
- **PC to phone.** `--send`, the drop window and the tray's "Send files…" make an `offer` (files are read where they are; a file that changes makes the `GET` answer `409 source_changed`; a file that is gone too). `GET` streams from disk in 1 MiB pieces, with `Range: bytes=N-` (`206`), `416` for a range that can't be met (or anything but one open-ended range), `404` for an unknown, canceled, finished or expired offer. An offer lasts 24 hours; when it expires the phone is sent a `cancel` (reason `expired`).
- **Cancel** from the drop window's button (either direction: a `cancel` message with reason `user` goes to the phone, partial files go) and from the phone (`DELETE` or a `cancel` message).
- **The WebSocket never waits for a file.** Everything about files runs in the HTTP handlers and in tasks of its own. A ping during a 300 MiB upload comes back in single-digit milliseconds.
- **Names and folders.** Received names follow protocol.md ("File names": UTF-8 byte limits, bidi controls, reserved names, the cut rules). A folder dropped for sending is skipped with a message (decision for 1.3, see windows.md).
- **After a restart** a transfer that was going is still there with what was stored (the smaller of the database and the disk), `.part` files are kept 24 hours after the last progress, and offers survive too. The clean-up (at start and hourly) drops transfers with no progress for a day and expired offers, and deletes only `.part` files that a database record names (a `.part` file nobody recorded is the user's), never a finished file, even one named `*.part`.
- **Automatic clipboard sync (PC to phone).** On by default, switched in the Settings window (1.4; in 1.3 in the tray) or with `autoClipboardSync` in `settings.json`. A hidden message-only window listens with `AddClipboardFormatListener`. Every change restarts a 300 ms timer. When it runs out the text on the clipboard goes to the phone as a `clipboard` message (LF line ends) and the PC waits for the `ack` (5 s). Text only. Not sent: content marked by a password manager (`ExcludeClipboardContentFromMonitorProcessing`, or `CanIncludeInClipboardHistory` or `CanUploadToCloudClipboard` with a value of 0, and the older `Clipboard Viewer Ignore`), the text the phone just sent (loop prevention: a hash of the wire form is remembered for 10 s and the change event it causes is ignored), anything that isn't text, and a text over 128 KiB (recorded as `too_large`). Never stored in a history (D10), never logged as text. While the phone is offline an automatic send is **dropped** (1.4: never queued, and never in the history: only the latest clipboard matters, and a stale one would overwrite newer content); an explicit one is queued and shows a toast.
- **The hotkey.** `Ctrl+Alt+Shift+K` by default ([why](../docs/windows.md#hotkey)), `hotkey` in `settings.json` to change it (read at start, empty turns it off). It sends the current clipboard text as a `text` message, with `isUrl` set when the trimmed text is an openable http(s) link (protocol.md's rule) and **`sensitive` set when the clipboard carried a password-manager format**. Over 128 KiB it goes as `Text.txt` (not at all if it is sensitive). The tray's "Send clipboard to phone" and `--send-clipboard` do the same. A toast says "Sent to <phone>", or why not.
- **Receiving `text` from the phone.** Acked first (the `ack` doesn't wait for the clipboard or the toast), then put on the clipboard with `\r\n` line ends (the wire has LF) without being sent back, and a toast "Text from <phone>" shows the first 100 characters on one line. A `sensitive` text goes to the clipboard with `ExcludeClipboardContentFromMonitorProcessing`, `CanIncludeInClipboardHistory` = 0 and `CanUploadToCloudClipboard` = 0, the toast has **no preview and no link**, and nothing about it is logged. If `isUrl` is set **and** the text really is an openable http(s) URL, clicking the toast opens it in the default browser. The rule is re-checked when the toast is made and again before `ShellExecute`, so `file:`, `javascript:`, `ms-msdt:` and the like never open, whatever the phone says. A click is only reported while the toast is on screen, and a click that doesn't say which toast it was opens nothing. A repeated message `id` gets the same `ack` and no second toast.
- **Toasts for received files** say "<n> file(s) from <phone>" with buttons **Open** and **Show in folder** (one file) or **Open folder** (several). A file is opened by a button or a click only if its type is on the allow-list (pictures, audio, video, PDF, text, macro-free Office files); any other file shows its folder with it selected, and its toast has only "Show in folder". Every received file carries the Mark of the Web. See [windows.md](../docs/windows.md#toast-buttons-r10).
- **Images** from the phone also go to the clipboard (a transfer with exactly one image): `CF_DIBV5`, `PNG` and `CF_HDROP`, EXIF rotation applied by our own code (a 12 MP photo with orientation 6 takes about 45 ms), nothing over 64 MiB of pixels; what is an image is decided by the decoder, not by the name or the MIME type. See [windows.md](../docs/windows.md#images-and-the-clipboard).
- **Connection limits.** TLS handshake 10 s, request headers 10 s, a JSON body 15 s (a slow client gets `400`), 30 s without a byte on a connection that isn't a WebSocket. 64 connections: 16 for handshaking, 24 for strangers (a client with a certificate of its own that isn't paired yet) and 24 that only a paired device can use, and at most 8 handshaking or unpaired connections per IP address. A flood of strangers can use up its own slots and never the paired phone's.
- **Identity:** a P-256 key and a self-signed X.509 v3 certificate (CN=Kin, no extensions) made on the first run. The device ID is `base64url(SHA-256(SPKI))`, and the short ID is the first 8 base32 characters.
- **Server:** TLS 1.3 only, ALPN `http/1.1`, port 47821 or a random one. A client certificate is mandatory in the handshake. The verifier accepts any chain but only an ECDSA P-256 leaf, and always verifies the CertificateVerify signature. Every request carries the device ID computed from the certificate.
- **Pairing:** "Pair new phone" opens a window with the QR code, the PC name, the short ID and a 5-minute countdown, and a **Copy pairing link** button (the same link as text, for a phone whose camera can't scan; it is taken off the clipboard again once the phone has paired, if it is still there). A token is single use and compared in constant time, and five wrong guesses in 60 seconds kill it. Closing the window ends the pairing. On success the window becomes the "New device paired" notice, with a button to remove the device (and a toast).
- **Addresses in the QR code:** private (RFC 1918) addresses of adapters that are up, without loopback, link-local, VPN or virtual adapters, except that the address on the default route is never left out. At most 3, the default route first, then physical adapters.
- **WebSocket:** one per device, a new one replaces the old one (4001). The PC sends `hello` at once. No `hello` in 10 seconds is a 1002 close. A PC `ping` that isn't acked in 5 seconds is "No reply" and the connection is closed. 10 minutes of silence marks the phone offline (the phone's WebSocket ping every 4 minutes keeps it alive). Message IDs are remembered in SQLite, so a repeated `ping` gets the same `ack` and no second toast.
- **Queue (1.4).** An explicit send is stored as `queued` before it is tried, and goes out after the phone's `hello` in creation order, one item at a time, each under its own ID. An item that gets no `ack` in 5 s stays queued (the PC sends no pings to find out: they would wake the phone's radio). Only an `error` from the phone fails it. Never queued: the automatic clipboard and a `sensitive` text. Details in [docs/windows.md](../docs/windows.md#queue); measured: [phase-1-4-desktop.md](../docs/results/phase-1-4-desktop.md).
- **Tray:** the icon (a green dot when connected, amber when offline, none when not paired) and the tooltip show the state, **"Phone offline – last seen 14:32"** when the phone is away (and how many sends are queued), and **the progress of a transfer ("sending 45%")**. A left click opens the drop window, **Shift + left click the debug window**. The menu is the one of docs/windows.md: the status line, Send files…, Send clipboard to phone, Pair new phone, Open received folder, History, Settings and Quit. (`--ping` is on the command line and in the debug window; the clipboard sync switch is in Settings.) Windows are destroyed when closed.
- **Logs:** every connect and disconnect with its reason or close code, every pairing attempt and its result, every ping, every transfer step, and start and stop. Never a key, a token or the pairing URL. **Never a file name (1.4):** a line that has to say which file has `<file 1a2b3c4d>`, a short **keyed** tag of the name (HMAC-SHA256 under a per-installation key that is protected with DPAPI in `log-key.dat` and never exported; the same for the tag of a text), and the name itself is written only at debug level, which is off. A `.part` path is never logged. The toasts still show the names (the log has the toast with the hash). This replaces the exception that the integration run found in 1.3 ("Sending dup2.txt to …").

## Not in this milestone

Folders as such (they are skipped), several images on the clipboard, code signing (the installer is unsigned: SmartScreen warns), a modern Windows 11 "Send to" menu entry (Phase 2), and a switch for the "Send to" shortcut in the Settings window (`"sendTo": false` in `integration.json` does it by hand). The installer work of 1.5 is described above and in docs/windows.md "Install".
