# Phase 2B.1: native messaging host (`kin-native-host.exe`)

Agent: Sonnet 5.5, coder. Date: 2026-10-16. Owns `desktop/native-host/` and this file. Nothing else was changed (not `desktop/src-tauri`, `android/`, `protocol/`, `tools/`, `extension/`, `docs/extension.md`), nothing was committed. The contract is [docs/extension.md](../extension.md); where this file and the contract disagree, see "Gaps in the contract" below.

## What is built

`desktop/native-host/`, a crate of its own (own `Cargo.toml` with an empty `[workspace]`, own `Cargo.lock`, own `target/`; **not** in `desktop/src-tauri`'s build yet).

| Path | What |
|---|---|
| `src/main.rs` | The host. `#![windows_subsystem = "windows"]` (checked by a test: PE subsystem 2): no console window can flash. Reads framed requests on stdin, writes framed answers on stdout (nothing else ever goes to stdout), loops until stdin closes. Exit codes (browsers ignore them): 0 stdin closed, 2 stdin broken, 3 request over 64 MiB |
| `src/framing.rs` | u32-LE length + JSON. `read_frame` (partial reads, clean EOF at a boundary, truncation, the length checked **before** the body is read, 64 MiB request / 1 MiB response / 65 MiB pipe limits) and `write_frame` |
| `src/envelope.rs` | Looks only at `v`, `id`, `type` (serde skips the rest without keeping it, so a 40 MB `data` costs no memory here). `BAD_REQUEST`, `UNSUPPORTED_VERSION` (`supported:[1]`), `UNSUPPORTED_TYPE`, `TOO_LARGE`, `KIN_NOT_RUNNING`, `TIMEOUT`, `INTERNAL`. Adds `origin` and `hostVersion` to the request **at the end of the extension's own bytes** (no re-serialization of a 64 MiB body); fills `hostVersion` into a `status`/`start` answer when Kin left it out |
| `src/pipe.rs` | The pipe client. Opens `\\.\pipe\<host>.<SID>` with `SECURITY_SQOS_PRESENT \| SECURITY_IDENTIFICATION` (the server can see who we are but can't act as us), waits for a busy pipe (`WaitNamedPipeW`), **verifies the server before sending anything** (`GetNamedPipeServerProcessId` → same user SID, and its image path is the expected `Kin.exe`), runs the exchange in a thread with a deadline and cancels the blocked call (`CancelIoEx`) on expiry |
| `src/host.rs` | The loop and the per-request logic (below) |
| `src/winutil.rs` | The only `unsafe` code of the host proper (SID, process owner and image, `CancelIoEx`, `CreateProcessW`, a per-user named mutex). Every block says what it relies on |
| `src/config.rs` | `Config::release()` (fixed: release pipe, `Kin.exe` next to the host). With the `dev` feature: the dev pipe and `KIN_HOST_DEV_*` overrides (`KIN_HOST_DEV_PIPE`, `KIN_HOST_DEV_KIN_EXE`, `KIN_HOST_DEV_ANY_SERVER`). The override can never name the release pipe (tested). **The release binary contains none of those strings** (checked: no `KIN_HOST_DEV`, `app.kinrelay.dev`, `kin-host-test`, `mock`) |
| `src/mock/` (feature `mock`) | `server.rs`: a pipe **server** that follows the server-side requirements, and is the reference for Kin's own (below). `kin.rs`: the Kin side of the contract (all request types, the offline-queue answer, the 30 per 10 s rate limit, image download by URL over plain http, image-by-content check, scenario file, log). A port of `extension/tests/smoke/mock-host.ts` |
| `src/bin/mock-kin.rs` (feature `mock`) | A stand-in Kin on the pipe. Refuses any pipe name that isn't a dev or test one. It is also what the host starts in the "start Kin" tests (the "dummy exe") |
| `tests/` | `relay.rs` (26), `binary.rs` (9), `pipe_security.rs` (4); `src/**` unit tests (21) |
| `smoke/` | `make-real-host-smoke.py` generates `run-real-host.ts` from `extension/tests/smoke/run.ts` (read only, by exact text replacements that must each match once); `run.ps1` builds and runs it |

Release profile: `opt-level = "z"`, LTO, `panic = "abort"`, stripped. **`kin-native-host.exe` is 243 KiB** (std + serde_json + windows-sys). Dependencies: `serde`, `serde_json`, `windows-sys`; `sha2` and `base64` only under the `mock` feature.

### What the host does, per request type

| Request | No pipe | Pipe there |
|---|---|---|
| `status` | `{ok:true, running:false, hostVersion, devices:[]}`. **Never starts Kin.** A pipe that stays busy for more than 1 s is the same answer | Forwarded; `hostVersion` filled in if Kin left it out |
| `open` | `KIN_NOT_RUNNING` (never starts Kin) | Forwarded |
| `start` | Starts `Kin.exe` (once, see the lock), retries the pipe every 100 ms for up to **10 s**, then forwards. Kin didn't come: `KIN_NOT_RUNNING` | Forwarded, answered like `status` |
| `send` | Same as `start`: starts Kin, waits, **then the send goes through** | Forwarded |
| anything else | `UNSUPPORTED_TYPE` from the host, no pipe contact | |

Host-side limits (the contract gives only the extension's: 8 / 15 / 90 s): status and open 7 s, start 14 s, send 85 s, all a little under the extension's own. After that the host answers `TIMEOUT` and goes on with the next request (long-lived `connectNative` use works, tested).

- **The server check fails** (impostor, another user's process, access denied): `KIN_NOT_RUNNING`, **nothing is sent**, also for `status`, and Kin isn't started (the pipe exists). The reason goes to stderr (browsers discard it).
- **Starting Kin** (`winutil::start_detached`): `CreateProcessW` of `Kin.exe` next to the host, `DETACHED_PROCESS | CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP | CREATE_BREAKAWAY_FROM_JOB` (retried without the breakaway flag if the browser's job forbids it, so closing the browser doesn't kill Kin), **`bInheritHandles = FALSE`**. That last one is the reason it isn't `std::process::Command`: the browser's stdin/stdout pipes are inheritable, and a Kin that inherited them would keep them open so the browser never sees the host exit. Started with no arguments and the current directory of `Kin.exe`'s folder (the same as the sign-in start).
- **One start at a time:** the start is under a per-user named mutex (`Local\<host name>.host-start`); a host that gets the lock second looks at the pipe first. Tested: 4 hosts started together start the Kin once.
- **Kin hangs up or crashes mid-request, or its answer isn't a JSON object, or is over 1 MiB:** `INTERNAL`, and the reason on stderr.
- **Framing errors:** invalid JSON, a non-object, no/odd `id`, no `type` → `BAD_REQUEST` (no `ref`) and the session goes on. A length prefix over 64 MiB → `TOO_LARGE` (no `ref`: the body isn't read, so the id isn't known) and the host exits. A frame cut short by stdin closing → exit, nothing answered.
- **`origin`:** Chromium's first argument if it starts with `chrome-extension://`; for Firefox (first argument is the manifest's `.json` path) the second argument (the extension ID); otherwise `""`. Only printable ASCII, at most 200 characters.

## Tests

All run in this repository, no admin, nothing outside `%TEMP%` and the test pipes (`\\.\pipe\kin-host-test-<pid>-<n>`; none of the tests can reach the user's Kin, and the real host binary is only run in its dev build with the pipe overridden).

| Suite | Result |
|---|---|
| `cargo test` (60 tests: 21 unit, 26 relay, 9 binary, 4 pipe security) | **60 pass, 0 fail** (about 3 s) |
| `bun smoke/run-real-host.ts` (the extension's 23 smoke steps, headless Chromium 1248, throwaway profile, the manifest's `path` is the **real host exe**, the Kin is `mock-kin.exe` on the pipe) | **23/23**; the registry key is gone afterwards; no `mock-kin`, host or Chromium process left |
| Release binary, hand-fed requests that never touch a pipe (unsupported type, bad version, bad frame) | correct answers, exit 0, stderr empty |

What the Rust tests cover:

- **Framing:** one byte at a time, 2/3/5/1000 bytes at a time, zero-length frame, EOF inside the header / body, a length prefix of exactly 64 MiB (works) and 64 MiB + 1 (`TOO_LARGE`, body unread), 4 GiB - 1, round trip with non-ASCII. Real stdio: a request dribbled in 3 bytes at a time with pauses.
- **Envelope:** bad JSON, a JSON array (serde would accept it as a struct), `null`, trailing garbage, missing / non-string / empty / 65-character / duplicate `id`, 64 non-ASCII characters (accepted: characters, not bytes), `v` of 0, 2, `"1"`, 1.5 and missing, wrong `type`.
- **Forwarding:** the request reaches Kin byte for byte with only `,"origin":…,"hostVersion":…` added at the end; a forged `origin` / `hostVersion` from the extension is overridden (last key wins); a hostile origin is escaped; a 5 MiB image request; an answer of exactly 1 MiB is forwarded, 1 MiB + 1 byte, non-JSON and a JSON array are `INTERNAL`; 5 requests in one session answered in order; `hostVersion` filled in only when Kin left it out.
- **Kin not running:** `status` is `running:false` and starts nothing (checked after 500 ms); `open` is `KIN_NOT_RUNNING` and starts nothing; `send`/`start` with no `Kin.exe` or a missing one answer at once; `start` starts the mock Kin (with a 600 ms start delay), waits, answers like `status`, and a second `start` starts nothing; `send` starts Kin and the send arrives (with `origin`); a Kin that exits at once gives `KIN_NOT_RUNNING` after the wait; 4 simultaneous hosts start Kin once.
- **Timeouts:** a Kin that sleeps past the host's limit gives `TIMEOUT` at the host's limit (not Kin's pace) and the next request in the same session works; `status` has its own shorter limit; Kin hanging up is `INTERNAL`.
- **Busy pipe** (a server limited to one instance, held by another request): `status` waits its second and says `running:false`; `open` says `TIMEOUT`.
- **Who the server is:** a server in another program than expected is refused for `status`, `send`, `start` and `open`, and received **nothing**; and an impostor on the pipe doesn't make the host start Kin.
- **Security properties of the pipe**, against a real server: with the host's open flags a server that impersonates the client gets `SecurityIdentification` (and `SecurityImpersonation` for a plain open: the control that proves the test discriminates); the reference server's DACL is exactly `D:P(A;;FA;;;<user SID>)` (read back from the pipe: protected, one entry, no Everyone, Users, Network, Administrators or SYSTEM); a second `FILE_FLAG_FIRST_PIPE_INSTANCE` of the same name is refused.
- **Binary:** one request / one answer / exit when stdin closes; Chromium and Firefox argument shapes; a long-lived session answering each request before the next arrives; bad JSON then a good request; the oversize prefix answered and the host exiting with stdin still open; stdin cut mid-frame (exit 2, empty stdout); stdout holds only whole frames; the PE subsystem is GUI.

### Not tested here

- **A real Edge, Brave, Vivaldi, Opera or Firefox.** Only Playwright's Chromium (through the Chromium key). The host's Firefox argument handling is unit- and process-tested, but a Firefox start with a real `allowed_extensions` manifest is not.
- **The installer and the registry keys** (below): written from the contract, never run.
- **A pipe server that belongs to another user** (needs a second account): the user check is `EqualSid` on the server process's token; the "wrong image" path is tested and takes the same branch.
- **An elevated Kin** (a standard-user host can't open an elevated process's token; it would answer `KIN_NOT_RUNNING`). Kin shouldn't run elevated; noted for the desktop agent.
- **`PIPE_REJECT_REMOTE_CLIENTS`** can't be read back from a client; it's in the reference server's creation call (and a test checks the source contains it).
- The host's start of the **real `Kin.exe`** (no test may start the user's Kin; every start test used `mock-kin.exe`).

## How the desktop agent integrates it

### (a) Workspace and installer

The crate is deliberately standalone so the other agent's `desktop/src-tauri` build isn't disturbed. Two ways to bring it in; **recommended: keep it standalone and build it from `desktop/build.ps1`.**

1. **Build** (release, no features: this is what makes it the release host):

   ```powershell
   cargo build --release --manifest-path desktop\native-host\Cargo.toml
   # -> desktop\native-host\target\release\kin-native-host.exe  (243 KiB)
   ```

   Never pass `--features dev`. Check: the exe must not contain the strings `KIN_HOST_DEV` or `app.kinrelay.dev`.

   *If a Cargo workspace is preferred* (the contract says "in the Rust workspace"): add `desktop/Cargo.toml` with `[workspace] members = ["src-tauri", "native-host"]`, `resolver = "3"`, and **remove the `[workspace]` table and the `[profile.release]`** from `native-host/Cargo.toml` (profiles live in the workspace root, so the root must carry both crates' needs: Tauri's `opt-level = "s"` would apply to the host as well, or use a per-package override `[profile.release.package.kin-native-host] opt-level = "z"`). The cost: the target directory moves to `desktop/target`, which every script that mentions `src-tauri\target\release` (build.ps1, the smoke and capture scripts) has to follow. That is why it isn't recommended mid-phase.

2. **Bundle it next to `Kin.exe`.** The host must be in the install folder (`C:\Program Files\Kin\`) as `kin-native-host.exe`: the host looks for `Kin.exe` next to itself, and Kin's installer manifests say `..\kin-native-host.exe` from `native-messaging\`. With Tauri's NSIS bundler, use a sidecar:
   - copy the built exe to `desktop/src-tauri/binaries/kin-native-host-x86_64-pc-windows-msvc.exe` (the triple suffix is required by Tauri; it's stripped on install),
   - `tauri.conf.json` → `bundle.externalBin: ["binaries/kin-native-host"]`,
   - `bundle.resources`: `{ "native-messaging/*": "native-messaging/" }` for the two manifests (below).
   Untested: check that the installed exe is `kin-native-host.exe` and that the manifests land in `$INSTDIR\native-messaging\`. If the resource map puts them elsewhere, install them from `hooks.nsh` with `File` instead and keep the relative `path`.

3. **Manifests** (written by whoever builds the installer; the contract has the exact JSON). Chromium: `allowed_origins: ["chrome-extension://cbioefpddjpaionhljhffgbedncfchmn/"]` (add the store IDs later, **in this one file**, because Edge reads only the first manifest it finds). Firefox: `allowed_extensions: ["kin@kinrelay.app"]`. Both `"path": "..\\kin-native-host.exe"`, `"type": "stdio"`, name `app.kinrelay.desktop`. Do **not** register or ship the dev name `app.kinrelay.dev`.

### (b) The pipe server inside Kin

Everything the host assumes, with the reference code in `desktop/native-host/src/mock/server.rs` (`create_instance`; the file uses `windows-sys`, translate to the `windows` crate Kin uses) and the Kin-side behavior in `src/mock/kin.rs`.

**Name and who owns it**
- Release Kin: `\\.\pipe\app.kinrelay.desktop.<user SID string>` (`ConvertSidToStringSidW` of the process token's user). A **dev Kin** (`--dev-bind`, a throwaway data dir, a test run) must use `\\.\pipe\app.kinrelay.dev.<SID>` (the dev host's default; `KIN_HOST_DEV_PIPE` overrides it for a test). Never the release name.
- **The host only talks to the installed `Kin.exe`:** it checks that the server process's image is `Kin.exe` in the host's own folder. A dev Kin run from `src-tauri\target\debug\` fails that check by design; the test points the **dev host build** at it: `KIN_HOST_DEV_KIN_EXE=<path of the dev Kin>` (also the program the host starts), or `KIN_HOST_DEV_ANY_SERVER=1` to skip the image check (the user check stays).
- Kin must not run elevated (the host, in the user's normal token, can't open an elevated process's token to check the user, and answers `KIN_NOT_RUNNING`).

**Creating the pipe** (`CreateNamedPipeW`)
- `dwOpenMode = PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE` for the **first** instance only. If creating it fails (access denied = the name is taken by someone else), log it, keep running **without** the extension pipe, and never fall back to a non-first instance.
- `dwPipeMode = PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS`.
- `nMaxInstances = PIPE_UNLIMITED_INSTANCES` (255), buffers 64 KiB.
- **Security descriptor: `D:P(A;;GA;;;<current user SID>)`**: a protected DACL with a single allow entry for the current user. No Everyone, Users, Network, Administrators or SYSTEM entry. (`ConvertStringSecurityDescriptorToSecurityDescriptorW`, then `LocalFree`.) The test reads the resulting DACL back and expects exactly `D:P(A;;FA;;;<SID>)` (`GA` is shown as `FA` for a pipe). `bInheritHandle = FALSE`.
- **Always keep one instance listening.** After `ConnectNamedPipe` returns for a client, create the next instance **before** serving the client (the reference does), and serve each client on its own thread or task. A moment with no listening instance makes the host wait (`WaitNamedPipe`) and, for a `status`, after 1 s answer `running:false`.

**Per connection (one request per connection)**
1. Read a frame: u32 LE length, then that many bytes, **at most 64 MiB + 1 MiB** (the host adds fields to a request that may already be 64 MiB). Drop a client that hasn't sent a whole request in about 30 s (the host's own limit is 85 s for a send; don't let a stuck client hold an instance).
2. Parse as a generic JSON tree first, then validate (protocol.md rule). **Duplicate keys: the last one wins** (serde_json's `Value` does that already; keep it). The host appends `"origin"` and `"hostVersion"` **at the end** of the extension's own object, so a forged `origin` inside the extension's object is overridden by the host's. `origin` is `chrome-extension://<id>/`, a Firefox extension ID, or `""`; treat it as information (the log), not as a permission.
3. Answer with one frame: `{v:1, ref:<id>, type:<the request's type>, ok, …}`, **at most 1 MiB** (the host answers `INTERNAL` for more) and a JSON object (anything else is `INTERNAL` too). Fill `hostVersion` in `status`/`start` answers or leave it out (the host adds its own). Echo `ref` exactly.
4. **`FlushFileBuffers`, then `DisconnectNamedPipe`, then close.** Without the flush the answer can be thrown away before the client reads it. (The reference does all three.)
5. Answer `status` quickly (the host gives up at 7 s, the extension at 8 s) and **never block the accept loop** on a slow `send` (the image download takes up to 35 s).
6. The 30 `send` per 10 s limit, the validation and every limit of the contract again (`BAD_URL`, 8 KiB URL, 4 MiB text, 32 MiB image bytes, image by content). `mock/kin.rs` shows the answers the extension's smoke test expects.

**A second launch of `Kin.exe` with no arguments must be harmless** (no window, nothing): the host starts `Kin.exe` when the pipe isn't there. If Kin is already running but has no extension pipe (first-instance creation failed, an old build, or Kin is still starting up and hasn't made it yet), every `send` or `start` starts a second `Kin.exe`, and the single-instance plugin has to treat an empty command line as a no-op. (It looks like `cli::parse` of no arguments gives no commands, so it should be: check once with a debug build.)

### (c) Installer registry keys

Per the contract, under **HKCU**, (Default) = the full path of the manifest. **Gap: the installer is per machine and runs elevated, so its HKCU may be another account's** (the same problem `hooks.nsh` explains for Send to and Run). Recommended: **the app writes these keys itself at every start, as the signed-in user** (like `integration.rs` does for the shortcut and the Run value, with the same "never from a developer run" rule), and the installer only lays down the files; the uninstaller removes the keys in both contexts (current user and `RunAsUser`, as it already does for Send to). The keys:

| Key (under `HKCU\Software\`) | Value (Default) |
|---|---|
| `Google\Chrome\NativeMessagingHosts\app.kinrelay.desktop` | `<install dir>\native-messaging\app.kinrelay.desktop.chromium.json` |
| `Chromium\NativeMessagingHosts\app.kinrelay.desktop` | same |
| `Microsoft\Edge\NativeMessagingHosts\app.kinrelay.desktop` | same |
| `BraveSoftware\Brave-Browser\NativeMessagingHosts\app.kinrelay.desktop` | same |
| `Vivaldi\NativeMessagingHosts\app.kinrelay.desktop` | same |
| `Mozilla\NativeMessagingHosts\app.kinrelay.desktop` | `<install dir>\native-messaging\app.kinrelay.desktop.firefox.json` |

(Opera reads the Chrome key, unverified. The contract calls it six keys; it is five Chromium keys plus the Firefox one.) Remove exactly those keys on uninstall, nothing else under the parents. Use `RegSetValueExW` of the default value, or in NSIS `WriteRegStr HKCU "Software\Google\Chrome\NativeMessagingHosts\app.kinrelay.desktop" "" "$INSTDIR\native-messaging\app.kinrelay.desktop.chromium.json"` and `DeleteRegKey HKCU "…\app.kinrelay.desktop"`. If the app writes them, the path comes from `current_exe()`'s folder, so a moved install repairs itself.

## Gaps in the contract (nothing in `docs/extension.md` was edited)

1. **Elevated installer and HKCU** (above). The contract says the installer writes the keys "for the user who installs".
2. **Host-side time limits** weren't specified; chosen 7 / 14 / 85 s (a little under the extension's 8 / 15 / 90 s), so the host's `TIMEOUT` reaches the extension instead of the extension's own.
3. **Kin closing the pipe without an answer** (a crash mid-request): the contract has no code for it. The host answers **`INTERNAL`**. (The extension smoke test expected `HOST_EXITED` here because its mock host exits; the generated real-host test expects `INTERNAL`.)
4. **Oversize request** (prefix over 64 MiB): the host answers `TOO_LARGE` **without a `ref`** (it doesn't read the body, so it doesn't know the id), and the extension ignores answers with a wrong `ref`, so it would end in its own `TIMEOUT`. It can't happen with this extension (images stop at 32 MiB, text at 4 MiB), and Chrome's own limit is 64 MiB, so it's noted rather than fixed.
5. **Finding Kin.** The task listed App Paths / the registry as a place to look; the host looks **only next to itself** (the contract's choice). A registry value that a same-user program can write would let it point the host (and the "expected server image" check) anywhere. Also: this only means something while the install folder is not user-writable (Program Files). A per-user install in `%LOCALAPPDATA%` would make the image check meaningless (a same-user attacker could replace `Kin.exe`); it would still stop other users and non-Kin programs squatting the pipe.
6. **Same-user squatting before Kin starts.** `FILE_FLAG_FIRST_PIPE_INSTANCE` stops a squatter only if Kin creates the pipe first. A same-user program that creates the pipe before Kin could receive requests, but the host refuses it (the server's image isn't `Kin.exe` next to the host), and Kin logs that its first instance failed. That is the contract's "security notes" case, now covered by a test.
7. **`status` with an untrusted server** is `KIN_NOT_RUNNING` (`ok:false`), per the contract, not `running:false`. The extension will show "Kin isn't running. Start Kin" and Start Kin will then also fail the same way: an impostor is a dead end by design, and the host's stderr has the reason.
8. **A start of `Kin.exe` when it's already running without a pipe** (see (b)): harmless only if a no-argument second launch does nothing.
9. **Duplicate keys** in a request: the host rejects duplicate `id`/`type`/`v` (serde refuses them: `BAD_REQUEST`) but passes other duplicate keys on; Kin must take the last (it does with `serde_json::Value`). Worth a line in the contract's "Pipe" section.

## Cleanup list (not blocking)

- `docs/extension.md` says the host is built in "the Rust workspace"; it's a standalone crate for now (see (a)).
- `cargo clippy` leaves 4 cosmetic suggestions (collapsible `if`).
- `Cargo.lock` of the crate is committed-worthy (it's a binary); `desktop/native-host/target/` is ignored by the repo's `target/` rule.
- The generated `smoke/run-real-host.ts` is checked in so it's reviewable; regenerate it with `smoke/make-real-host-smoke.py` after the extension's smoke test changes (the generator fails if one of its text replacements no longer matches).
- Version is `0.1.0` (`hostVersion` in answers); bump with the app at 0.2.0.
- Antivirus heuristics: a small unsigned exe that starts another program detached can be flagged; the host is signed with the installer's certificate once there is one.
