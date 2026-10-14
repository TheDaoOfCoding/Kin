# Phase 0 result: Tauri v2 tray spike for D2

Measured 2026-10-14 on Windows 11 Pro 10.0.26300, Ryzen 9 7900X, 31 GB RAM (the same PC as [phase-0-dotnet.md](phase-0-dotnet.md)). Code is in `spikes/desktop-tauri/`. This doc mirrors the .NET one so the two can be read side by side. D2 was decided for Tauri on 2026-10-14, using this doc and the .NET one (see [decisions.md](../decisions.md)).

## What was built

A tray-only Tauri v2 app. Product `Kin` (binary `Kin.exe`), identifier `app.kinrelay.desktop`, window title "Kin Relay". Scaffolded with `bun create tauri-app desktop-tauri --manager bun --template vanilla-ts --identifier app.kinrelay.desktop --yes --force`, then trimmed (the greet demo and the opener plugin are gone).

- Starts with no window and a tray icon with tooltip "Kin". `app.windows` is `[]` in `tauri.conf.json`, so no WebView2 is created at startup.
- Menu: **Open** creates a 320x160 window that shows "Hello from Kin Relay". **Quit** removes the tray icon (`remove_tray_by_id`) and calls `app.exit(0)`. Double-clicking the icon also opens the window (the .NET spike does the same). The menu is on right-click only.
- **Closing the window keeps the app running.** By default the window is *destroyed* on close, which is what the .NET spike does and what [windows.md](../windows.md) asks for. A `RunEvent::ExitRequested` handler refuses the exit that Tauri requests when the last window closes (it lets `app.exit(0)` through, because that one carries an exit code). The task wording was "closing hides it", so `KIN_HIDE_ON_CLOSE=1` switches to hiding the window instead. Both were measured (see Metrics).
- Single instance uses the official `tauri-plugin-single-instance` 2.5.3. A second launch exits by itself and the first instance shows its window.
- Tray is Tauri's own (`tray-icon` feature, `TrayIconBuilder`, the `muda` menu API). No third-party tray crate.
- Icon: the "K" from the .NET spike. `gen-icons.ps1` copies the 256 px PNG frame out of `spikes/desktop-dotnet/Assets/kin.ico` (read only, the original is untouched) and runs `bun tauri icon` on it.
- UI is `index.html` plus a 3-line `src/main.ts`, built by Vite. It is only there to prove the TypeScript to WebView2 path.

Checked on the real tray with UI Automation and keyboard input (Win+B, Enter, Apps key, Down, Enter): the "Kin" button with tooltip "Kin" is in the overflow flyout, the menu shows Open and Quit, Open shows the window and UI Automation finds the text "Hello from Kin Relay" in it, closing the window leaves the process running with the window gone, Quit exits with code 0, removes the icon and leaves no WebView2 processes. All of it repeated in 4 runs.

## Toolchain, build and run

- **Rust 1.99.0** (`rustc` b940084d7 2026-09-28, `cargo` 1.99.0) via **rustup**, `stable-x86_64-pc-windows-msvc`, minimal profile. Installed per user with the official `rustup-init.exe` from static.rust-lang.org (SHA-256 matched the published `.sha256`; the exe is not Authenticode-signed, which is normal for rustup-init). Lives in `%USERPROFILE%\.cargo\bin` and `%USERPROFILE%\.rustup`. rustup added `%USERPROFILE%\.cargo\bin` to the **user** PATH itself. The machine PATH was not touched, no `setx`, no admin. Verified from a fresh shell. The tool shell that did the install did not see the new PATH, so `_env.ps1` re-reads Machine+User PATH from the registry.
- **MSVC:** Visual Studio Build Tools 2022 (C++ workload), found by rustc on its own. `link.exe` is not on PATH and does not need to be.
- **Tauri:** `tauri` crate 2.12.3, `@tauri-apps/cli` 2.12.2, `@tauri-apps/api` 2.12.3, `tauri-plugin-single-instance` 2.5.3, Vite 8.3.4, TypeScript 6.0.3, Bun 1.4 as the package manager.
- **WebView2 runtime:** present, Evergreen **155.0.4283.45** (HKLM `WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}`; `C:\Program Files (x86)\Microsoft\EdgeWebView\Application` also holds 154.0.4258.62). A Tauri app needs no other runtime.
- **Build:** `powershell -ExecutionPolicy Bypass -File spikes\desktop-tauri\build.ps1` (runs `bun tauri build --bundles nsis`). Outputs `src-tauri\target\release\Kin.exe` and `...\bundle\nsis\Kin_0.1.0_x64-setup.exe`.
- **Dev:** `. .\_env.ps1; bun tauri dev` in `spikes\desktop-tauri`. Vite listens on `localhost:1420` (checked: `::1` only, no firewall prompt). Debug build took 47 s after the release deps were already compiled. The dev window loaded the page from Vite. The debug binary is `target\debug\kin.exe`. `mainBinaryName: "Kin"` only renames at bundle time. I did not test hot reload.
- **Run:** `src-tauri\target\release\Kin.exe`, then look for the "K" in the tray overflow ("Show hidden icons"). Quit from its menu.
- **Measure:** `measure.ps1 -Mode Session|Startup|SingleInstance|Quit` (needs an unlocked interactive desktop, it presses keys).

### Snag: the NSIS download timed out

`bun tauri build --bundles nsis` compiled fine but failed at "Downloading https://github.com/tauri-apps/binary-releases/releases/download/nsis-3.11/nsis-3.11.zip" with `failed to bundle project: timeout: global`, on 4 retries in a row. `curl` fetched the same URL in under a second, so it is the bundler's HTTP client here, not the network. What fixed it: downloading the two files by hand from the official GitHub releases and unpacking them where the bundler looks, `%LOCALAPPDATA%\tauri\NSIS`:

- `nsis-3.11.zip` (SHA-1 `ef7ff767e5cbd9edd22add3a32c9b8f4500bb10d`, which matches the hash embedded in the CLI), extracted so that `NSIS\makensis.exe` exists.
- `nsis_tauri_utils.dll` v0.5.3 (SHA-1 `75197fee3c6a814fe035788d1c34ead39349b860`, also matches the CLI) into `NSIS\Plugins\x86-unicode\additional\`.

Without the DLL the bundler says "NSIS directory is missing some files. Recreating it." and deletes the directory. This is a one-time per-machine step, but a CI or a second PC could hit it. Not tried: whether a longer timeout or another network avoids it.

## Metrics

### Build times (Ryzen 9 7900X)

| What | Time |
|---|---|
| Cold: first `build.ps1`, empty `target\`, empty cargo cache | Rust compile 2 m 32 s. The whole script took 3 m 4 s (bun, tsc, Vite, cargo), and then failed at the NSIS download (see above) |
| Rebuild after touching `lib.rs`, whole `build.ps1` including NSIS | 60.7 s (58 s is cargo, about 3 s is the bundle) |
| Rebuild with no changes | about the same: `tauri build` recompiles the app crate every time (about 58 s) because of `lto = true` and `codegen-units = 1` |
| `tauri dev` first debug build | 47 s |

The cold figure does not include the NSIS and rustup downloads.

### Idle memory

Release `Kin.exe`, launched by the script, sampled 60 s after launch. Private working set comes from the `Win32_PerfFormattedData_PerfProc_Process` counters (the same value Task Manager calls "active private working set"). Working set and private bytes come from `Get-Process`. Kin's WebView2 processes are found as descendants of `Kin.exe` plus any `msedgewebview2.exe` whose command line contains the app identifier. This PC runs other WebView2 apps, which were left out. Three runs of the default behaviour (window destroyed on close), all within 0.1 MB of each other for these numbers:

| State | Processes | Working set | Private working set | Private bytes (commit) |
|---|---|---|---|---|
| **A. Tray only, window never opened (60 s)** | 1 | 12.1 to 12.2 MB | **1.5 to 1.6 MB** | 2.0 to 2.1 MB |
| **B. Window open (5 s after Open)** | 7 | 325.6 to 326.7 MB | **72.8 to 73.3 MB** | 149.5 to 150.7 MB |
| **C. 10 s after closing the window** | 1 | 29.1 to 29.3 MB | **4.2 to 4.4 MB** | 5.9 to 6.1 MB |

- In A, `Kin.exe` has 2 threads and 149 handles. No WebView2 process exists.
- Summed working set double counts pages shared between processes (the WebView2 processes share DLLs), so private working set is the honest total.
- C stays above A because `Kin.exe` keeps the WebView2 loader and some heap after the window is gone (36 threads). All 6 WebView2 processes exit as soon as the window is destroyed. 0 leftovers.
- Per-process breakdown, state B, run 3 (the other runs match within about 1 MB):

| Process | Working set | Private working set | Private bytes | Threads | Handles |
|---|---|---|---|---|---|
| `Kin.exe` | 29.1 MB | 4.4 MB | 6.1 MB | 37 | 365 |
| WebView2 browser | 120.9 MB | 28.2 MB | 36.4 MB | 56 | 1423 |
| WebView2 gpu-process | 63.3 MB | 18.9 MB | 66.5 MB | 91 | 740 |
| WebView2 renderer | 51.0 MB | 11.3 MB | 20.0 MB | 14 | 247 |
| WebView2 utility (network) | 33.7 MB | 6.0 MB | 11.1 MB | 23 | 324 |
| WebView2 utility | 19.1 MB | 2.7 MB | 7.3 MB | 9 | 177 |
| WebView2 crashpad-handler | 9.6 MB | 1.6 MB | 2.5 MB | 11 | 151 |
| **Total** | **326.7 MB** | **73.1 MB** | **149.9 MB** | | |

- **If the window is hidden instead of destroyed** (`KIN_HIDE_ON_CLOSE=1`, one run): after 10 s the 7 processes are all still there, private working set 73.2 MB, working set 326.4 MB, private bytes 150.0 MB. So hiding costs about 73 MB for as long as the app runs, and destroying frees it. This is why [windows.md](../windows.md) says to destroy.
- The measurement comparable to the .NET "window open" figure is B: .NET 128 MB working set and 73 MB private bytes (framework-dependent, one reading), Tauri 326 MB working set and 150 MB private bytes in 7 processes. The .NET private working set with the window open was not measured, so only the Tauri side has that column.
- CPU was not measured (the .NET spike did not either).

### Startup

Two numbers per launch, from `Start-Process` returning:

| Run | `WaitForInputIdle` | Tray icon window exists |
|---|---|---|
| 1st launch of the freshly built exe | 126 ms | not captured (the script did not have this yet) |
| 2nd launch | 107 ms | not captured |
| Warm launches, 12 runs (the tray timer only existed for the last 8) | 15 to 45 ms (median 16 ms) | **30 to 52 ms** (median 32 ms, 8 runs) |
| Warm, at the start of each full session run, 4 runs | 35 to 45 ms | 41 to 52 ms |

- **The `WaitForInputIdle` figure is not comparable to the .NET one.** The .NET spike reported 125 to 216 ms with it. For Tauri it returns long before the event loop exists (15 ms warm), so I added a second timer: the moment the process owns the hidden `tray_icon_app` window, which the tray-icon crate creates right before it registers the icon with the shell. That is the better "tray is up" number, but it is still not "the icon is visible". I did not time that with UI Automation because polling the flyout is too coarse.
- Both numbers include process creation and the PowerShell `Start-Process` overhead, so they are upper bounds on the app's own start.
- Cold boot start was not measured. The first two launches (107 to 126 ms) show first-run cost is about 4 times the warm figure, as with .NET.

### Sizes

| Artifact | Size |
|---|---|
| `Kin.exe` (release, `lto`, `opt-level = "s"`, `strip`, `panic = "abort"`) | 3,184,128 bytes = **3.04 MiB**. Standalone: the WebView2 loader is linked in, no DLL next to it |
| NSIS installer `Kin_0.1.0_x64-setup.exe` | 1,137,689 bytes = **1.08 MiB** (not run) |

For scale: `tls-check` below, which is just rustls with aws-lc-rs, rcgen, an X.509 parser and the `windows` crate, is a 3.3 MB exe with default release settings (no LTO, no strip). That is a data point for what the TLS stack adds. A real Kin with tokio, axum, SQLite and the rest will be larger than this hello world. By how much is not measured.

## Network category (D12)

Not repeated. See [phase-0-dotnet.md](phase-0-dotnet.md): Ethernet "Network 4" is **Public**. No socket was opened by Kin.exe, so there was no firewall prompt.

## D2: Windows-side needs with Tauri

"Unverified" means I know it from documentation or memory and did not build or run it here. "Verified" means I built and ran it on this PC. Crate dates are from the crates.io API on 2026-10-14. Two verification programs exist: the tray spike, and `spikes/desktop-tauri/tls-check/` (an in-memory TLS 1.3 handshake plus DPAPI, no sockets, `cargo run --release` in that folder).

| Need | Rust/Tauri approach | Snag |
|---|---|---|
| HTTPS server + mutual TLS + WebSocket | `axum` 0.8.9 (Apr 2026, WebSocket built in) on `tokio` 1.53 with `tokio-rustls` 0.26.6 and `rustls` 0.23.46 (released today). Custom TLS accept loop that attaches the peer certificate to each request, as [windows.md](../windows.md) plans. `axum-server` 0.8.0 (Dec 2025) is a possible shortcut | **rustls side Verified** (see TLS row). **axum, WebSocket, the accept loop: Unverified, not built.** The accept loop is extra code that .NET's Kestrel gives for free (`HttpContext.Connection.ClientCertificate`). Whether `axum-server` exposes the peer cert to handlers: Unverified |
| TLS 1.3 only, Windows 10 and 11 | `rustls` is pure Rust, with its own crypto (`aws-lc-rs` by default, `ring` optional). It does not call Schannel, so the OS's TLS support is irrelevant | **Verified on Windows 11** with `tls-check`: `ServerConfig::builder_with_provider(..).with_protocol_versions(&[&version::TLS13])`, a custom `ClientCertVerifier` that accepts any client cert, then a handshake between a client and server in memory. Result: negotiated `TLSv1_3`, the server read the client's cert, and a TLS 1.2-only client was refused (`Tls12NotOfferedOrEnabled`). Application data went through. **Windows 10 was not tested.** Since nothing in the path uses Schannel, it should behave the same, but that is reasoning, not a test. This meets protocol.md ("TLS 1.3 only") on both OSes without relaxing it. The rustls docs warn that another crate in the tree can re-enable TLS 1.2, so keep the explicit version list |
| Build requirements for rustls on Windows | `aws-lc-rs` (default) compiles C with MSVC | **Verified:** `aws-lc-sys` built here with only the VS Build Tools, with no CMake or NASM on PATH. `tls-check` took 35 s cold. Not checked: a machine without the C++ workload (it would fail at link, same as any Rust on Windows) |
| Certificates (P-256, self-signed, SPKI hash) | `rcgen` 0.14.10 (Aug 2026) makes an ECDSA P-256 self-signed cert by default. `x509-parser` 0.18 plus `sha2` for SHA-256(SPKI) | **Verified.** The SPKI hash the server computed from the TLS client cert equals the one computed from the phone's own cert, and likewise for the PC cert. Rustls takes the cert and key as DER directly, so there is no PFX export/import step (an open snag in the .NET row) |
| Private key encryption | DPAPI: `CryptProtectData` / `CryptUnprotectData` from the `windows` crate 0.62.2 (Microsoft's own bindings, features `Win32_Security_Cryptography`, `Win32_Foundation`) | **Verified.** A 35-byte secret came back identical after a round trip, and the 262-byte blob did not contain the plaintext. Alternatives: `keyring` 4.2.0 (Credential Manager, Aug 2026), `windows-dpapi` 0.2.0 (tiny, about 56k recent downloads), so prefer the `windows` crate. Not looked at: the Stronghold plugin |
| mDNS advertising | `mdns-sd` 0.21.5 (pure Rust, Oct 2026; 10 releases between May and October 2026, so actively maintained). Alternative: `DnsServiceRegister` in `dnsapi.dll` via the `windows` crate | **Unverified.** I did not run it, because it opens multicast UDP sockets. Windows has its own mDNS responder on port 5353, so sharing the port is the thing to test first. Firewall behaviour is the same as for any stack |
| Clipboard change listener | `AddClipboardFormatListener` on a hidden message-only window via the `windows` crate, on its own thread. `clipboard-master` 4.0.0 (Nov 2025, low usage: about 8k recent downloads) wraps exactly this. `arboard` 3.6.1 (Aug 2025) for read and write, `clipboard-win` 5.4.1 (Jul 2025) for the format names and the opt-out formats | **Unverified.** Tauri's own `tauri-plugin-clipboard-manager` 2.4.2 is read/write, and I did not find a change event in it. Own message-loop thread is a few dozen lines. `arboard` and `clipboard-win` are stable but have not released in 14 to 15 months |
| Toasts with click actions | `tauri-plugin-notification` 2.6.0 does title and body only on desktop: its docs say "The Actions API is only available on mobile platforms", and on Windows "Only works for installed apps" (in dev the toast shows PowerShell's name and icon). For buttons and click callbacks use `tauri-winrt-notification` 0.8.1 (Tauri org, Jul 2026): `Toast::new(app_id)`, `add_button(label, action)`, `on_activated(callback)` | **Unverified, not run.** It is the same risk as the .NET row (windows.md 1.3: "buttons are impractical?"), and it should be tested first. Open points: the app ID has to be a registered AppUserModelID or the toast is attributed to PowerShell; the crate's docs say nothing about what happens when the toast is clicked after the app has exited (an unpackaged app needs a COM activator for that, and I did not see one in this crate); `Windows App SDK` through the `windows` crate is the heavier alternative |
| Tray | Tauri's `tray-icon` feature (locked in `Cargo.lock` at `tray-icon` 0.25.1 and `muda` 0.20.0; the latest on crates.io are 0.26.1 and 0.21.2, both Oct 2026) | **Verified.** Tooltip, menu, double-click and Quit all work. The native menu has the Windows 11 look (rounded, dark when the system is dark), which the .NET `NotifyIcon` menu does not. Tray-only apps must refuse `ExitRequested` with no code, or closing the last window quits the app, and `remove_tray_by_id` before exit avoids a ghost icon. A crash ghost icon was not tried. Windows 11 puts new icons in overflow (same for any stack) |
| Global hotkey | `tauri-plugin-global-shortcut` 2.4.1 (Oct 2026), built on `global-hotkey` 0.8.0 (May 2026, `RegisterHotKey`) | **Unverified.** Registration fails if another app owns the combination, so show that in settings |
| Explorer "Send to" | A `.lnk` in `%APPDATA%\Microsoft\Windows\SendTo`. Either the NSIS installer creates it (NSIS has a `$SENDTO` constant, through an installer hook), or the app writes it with `IShellLinkW` through the `windows` crate. `mslnk` 0.1.8 is unmaintained (May 2022), avoid. Args reach the running instance through the single-instance plugin's callback `(app, args, cwd)` | **Unverified.** The plugin half is partly verified: a second launch is intercepted, exits, and the first instance reacts, but I did not pass `--send <paths>` through it |
| Autostart | `tauri-plugin-autostart` 2.7.0 (Oct 2026), which uses the `auto-launch` crate (0.6.0, Jan 2026, HKCU `Run` key). Or write the key with `winreg` 0.56 or `windows-registry` | **Unverified.** Per user, no admin |
| Firewall rule at install | NSIS installer hooks: `NSIS_HOOK_POSTINSTALL` and `NSIS_HOOK_PREUNINSTALL` in a `hooks.nsh` pointed to by `bundle.windows.nsis.installerHooks`, running `netsh advfirewall` | **Unverified, not run.** Needs admin, so `installMode` must be `perMachine`: the Tauri default is `currentUser` (installs to `%LOCALAPPDATA%`, no admin), and `both` still asks for admin. [windows.md](../windows.md) already wants per-machine. A WiX MSI (WiX v3, `fragmentPaths`) is the other way. Binding a non-loopback port still triggers the Windows Firewall prompt |
| Installer | Tauri NSIS bundle, 1.08 MiB for the hello world | **Built, not run.** Per-user by default (see above). WebView2: the default `downloadBootstrapper` adds 0 MB but needs internet at install time on a PC without the runtime. `embedBootstrapper` about +1.8 MB, `offlineInstaller` about +127 MB, `fixedVersion` about +180 MB (all per the Tauri docs, not measured). Tauri's docs say the runtime ships with Windows 11 and with Windows 10 April 2018 and later; I could only check Windows 11. The bundler's own download of NSIS timed out here (see Snag). Code signing: Unverified |
| Single instance | `tauri-plugin-single-instance` 2.5.3 (official, Oct 2026) | **Verified.** The second process exited with code 0 and the first showed its window. Must be registered first |
| SQLite | `rusqlite` 0.40.2 (Aug 2026) with the `bundled` feature | **Unverified, not built.** Compiles C with the MSVC tools, which `aws-lc-sys` already did successfully here |
| QR rendering | `qrcode` 0.14.1 (SVG output built in) | **Unverified.** Last release Jul 2024 (27 months ago); a small, finished library. `fast_qr` 0.14.0 (Sep 2026) is the active alternative |
| UI | TypeScript + Vite in WebView2, as planned | **Verified** for the hello window and the dev loop. An open WebView2 costs about 73 MB private working set in 6 child processes, and destroying the window frees it (see Metrics). Needs Bun or Node at build time |

Notes for the comparison:

- Everything the app needs on Windows has a crate or plugin, but several of them are 20 to 200 lines of `windows`-crate code (clipboard listener, Send to, firewall hook), where the .NET row names a one-line API or a NuGet package for the same job.
- The Rust side runs in-process with the tray, so the real server, the TLS, the DPAPI call and the tray will share one binary of a few MB.
- The costs are the WebView2 processes when a window is open, and the slower build loop (a release rebuild takes about 60 s).
- The idle tray-only process is tiny (1.5 MB private working set, 2 threads) because there is no runtime and no GC.

## Against [targets.md](../targets.md) (the PC footprint and App size rows)

| Target | Goal | Tauri spike | Result |
|---|---|---|---|
| PC footprint (idle) | Under 30 MB active private working set, all windows closed, no leftover WebView2 | 1.5 MB tray only after 60 s. 4.3 MB after opening and destroying the window, with no WebView2 left over. 73 MB with the window open | Passes with room. It is only a hello world with no networking, and the target is measured after 1 h idle, so it will grow. I did not run 1 h. If windows are hidden instead of destroyed, it fails (73 MB) |
| App size | About 10 MB or less each for APK and installer | `Kin.exe` 3.04 MiB, NSIS installer 1.08 MiB | Passes for the hello world. The real app adds the TLS stack and more, still unmeasured. The installer does not include WebView2 unless `embedBootstrapper` or larger is chosen (+1.8 MB to +180 MB) |

## Side by side: .NET vs Tauri

.NET figures are from [phase-0-dotnet.md](phase-0-dotnet.md), same PC, same day, one run each. Tauri figures are 3 runs unless noted. Where a number is not comparable it says so.

| | .NET (WPF, .NET 10) | Tauri v2 (Rust + WebView2) |
|---|---|---|
| Idle memory, tray only | 57 to 61 MB working set, 8.9 to 10.2 MB private working set, 13.8 to 14.8 MB private bytes | 12.1 MB working set, **1.5 MB** private working set, 2.0 MB private bytes (1 process) |
| After opening and closing the window | Not measured | 29 MB working set, 4.3 MB private working set when the window is destroyed. 73 MB if it is only hidden |
| Window open | 128 MB working set, 73 MB private bytes (one reading) | 326 MB working set, 73 MB private working set, 150 MB private bytes (7 processes) |
| Startup (tray up) | About 125 to 216 ms to `WaitForInputIdle` (130 ms warm) | About 30 to 50 ms until the tray window exists (warm); `WaitForInputIdle` 15 to 45 ms is not a like-for-like number. First launches were 107 to 126 ms |
| Installer / exe size | Self-contained single-file exe 147.6 MB (66.2 MB compressed, with a startup and memory penalty). Framework-dependent 0.2 MB. No installer built yet | `Kin.exe` 3.04 MiB, NSIS installer 1.08 MiB. No installer run |
| Runtime dependencies | Self-contained: none. Framework-dependent: .NET 10 Desktop Runtime, plus the ASP.NET Core Runtime for Kestrel, so two runtimes | WebView2 runtime (Evergreen 155 here; ships with Windows 11 and recent Windows 10; the installer can bootstrap it, which needs internet by default). The MSVC build tools are a build-time need only |
| Windows 10 TLS 1.3 | Kestrel uses Schannel: TLS 1.3 only on Windows 11 / Server 2022 and later (Unverified in the .NET doc). Meeting "TLS 1.3 only" on Windows 10 means relaxing it to 1.2+ or dropping Windows 10 | rustls does its own TLS 1.3 on any OS. Verified on Windows 11 only (in-memory handshake with mutual auth, TLS 1.2 refused); Windows 10 untested but does not depend on Schannel |
| Agent iteration speed | One language (C#) for everything: UI in XAML, server in Kestrel, no web UI. Rebuild time was not measured | Two languages: TypeScript UI, Rust backend. First compile of the spike took 2.5 min and each incremental release rebuild is about 60 s (`lto = true`, `codegen-units = 1`), which slows the edit-test loop. `bun tauri dev` rebuilds in debug in 47 s cold |
| Mutual TLS + WebSocket server | Kestrel, built in. Peer cert on `HttpContext`. Client certs over TLS 1.3 unverified | `axum` + `tokio-rustls`, with a custom accept loop to attach the peer cert. TLS and mutual auth verified, axum and the loop not built |
| Toasts with click actions | Toolkit package with no release since Nov 2022, or Windows App SDK. Unverified | `tauri-plugin-notification` has no desktop actions. `tauri-winrt-notification` 0.8.1 has buttons and callbacks. Unverified, same app ID and cold-click questions |
| Clipboard listener, hotkey, Send to, autostart | Short P/Invoke or registry code, or small NuGet packages. Unverified | Plugins (hotkey, autostart) and `windows`-crate code (clipboard listener, Send to). Unverified |
| Tray menu look | WinForms `NotifyIcon` menu, not Windows 11 styled | Native menu with the Windows 11 look (verified) |
| Firewall rule, installer | WiX MSI, Inno Setup or Velopack. Unverified | NSIS hooks or WiX. Per-user by default, per-machine needed for the rule. Unverified |
| Third-party risk | `Microsoft.Toolkit.Uwp.Notifications` stale. `Makaretu.Dns.Multicast` state unknown | Most crates released in the last 6 weeks. Slower ones: `arboard` (Aug 2025), `qrcode` (Jul 2024), `mslnk` (May 2022, avoid) |
| Dev environment cost | .NET SDK (per user, with a PATH gotcha) | Rust per user, VS Build Tools C++ (needs admin, done), Bun. NSIS download needed a manual workaround here |

### What the numbers suggest (my read, not a decision)

On the PC footprint and App size targets, Tauri has a large lead in this spike: a tray-only process that uses 1.5 MB of private working set against 9 MB, and a 3 MB exe against 148 MB self-contained (or against 0.2 MB plus two runtimes). The window-open cost is the Tauri weak spot: 73 MB of private working set over 6 child processes. In the one pair of columns both spikes have, a window costs more on Tauri (150 MB private bytes vs 73 MB, 326 MB working set vs 128 MB; the .NET window was one reading and a framework-dependent build). That is only acceptable if windows are destroyed on close and the user rarely has one open.

The TLS row is the one that can change the plan. protocol.md demands TLS 1.3 only. Kestrel with Schannel can only meet it on Windows 11 (that is a Kestrel and Schannel limit, not a limit of .NET in general: a .NET app could bring its own TLS stack). Tauri with rustls meets it everywhere, and I verified the handshake including client certificates. If Windows 10 matters, that favours Tauri or a protocol change. If it does not, the TLS point does not separate them.

The cost on the Tauri side is the slower rebuild loop (about 60 s per release rebuild) and the amount of code under the "Unverified" labels: the custom accept loop, the clipboard listener, toasts with buttons, and the installer hooks. None of those looks blocked, but none is proven. The risks that both stacks share and that neither spike tested are toasts with click actions and the firewall rule at install time. The question to ask of those two is what each spike would cost to prove, not which is easier on paper.

## Not done

- Windows 10 was not available, so nothing was run there.
- The installer was not run (by instruction), so per-user vs per-machine, shortcuts, the AUMID and the WebView2 bootstrapper are all unchecked.
- No cold-boot startup, no 1 h idle, no CPU sampling, no 10-minute CPU check from targets.md.
- mDNS, the clipboard listener, toasts, the hotkey, autostart and Send to were not built or run. The axum server was not built.
- The "tray icon present" UI Automation check returned False in one of the four full runs even though the icon was there (Open through the same icon worked right after). It is a flaky check of the flyout, not an app problem.
- The phone-to-PC browser test is still waiting for the user, as in the .NET doc.

## Files

`spikes/desktop-tauri/`: `src-tauri/` (Rust, `tauri.conf.json`, `capabilities/`, `icons/`), `src/` and `index.html` (TypeScript UI), `build.ps1`, `measure.ps1`, `_env.ps1`, `gen-icons.ps1`, `icon-source.png`, `bun.lock`, and `tls-check/` (standalone TLS + DPAPI check).
