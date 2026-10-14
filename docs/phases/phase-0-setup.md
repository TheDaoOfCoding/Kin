# Phase 0: setup

Goal: everything is installed and proven to work, so Phase 1 is only about building.

## 1. Install toolchains

Checked on this PC on 2026-10-14, before any installs: Git 2.55, Node 22 and Bun 1.4 were present. Android SDK, Java, .NET SDK and Rust were not.

Update 2026-10-14: installed per user, with no admin: Temurin JDK 21 and the Android command-line SDK (see [phase-0-android.md](../results/phase-0-android.md)), and .NET SDK 10.0.402 (see [phase-0-dotnet.md](../results/phase-0-dotnet.md)). Still not installed: Rust and the Visual Studio Build Tools.

| Tool | Why | Notes |
|---|---|---|
| Android SDK (command line) plus a JDK. Android Studio is optional | Build the Android app, `adb` | Installed: the command-line SDK and Temurin JDK 21. Android Studio is only needed for the emulator and the IDE. Platform-Tools and the latest SDK Platform come from `sdkmanager` |
| Visual Studio Build Tools 2022 or later with the "Desktop development with C++" workload | Linker that Rust needs on Windows | Install before Rust. Installing it needs admin (UAC), so do it when you're at the PC |
| Rust via rustup (stable, MSVC toolchain) | Tauri backend | `rustup default stable` |
| WebView2 runtime | Tauri windows | Already present on Windows 11 |
| Tauri CLI v2 | Build and run the Tauri spike | `bun create tauri-app` adds it (step 4) |
| .NET SDK (latest LTS) | Build the .NET (C#, WPF) spike | `& "$env:LOCALAPPDATA\Microsoft\dotnet\dotnet.exe" --version`. A bare `dotnet` does not find the per-user SDK, see the PATH gotcha in [phase-0-dotnet.md](../results/phase-0-dotnet.md) |

Check with: `rustc --version`, `cargo --version`, `adb --version` and the .NET command in the table (not a bare `dotnet --version`). Add `%LOCALAPPDATA%\Android\Sdk\platform-tools` to PATH so `adb` works in any terminal (already done for the per-user install).

## 2. Prepare test devices

- **Your phone:** Settings → About phone → tap the build number 7 times to unlock Developer options. Turn on USB debugging, and on Android 11+ also Wireless debugging so you can install builds without a cable.
- **More phones:** ideally borrow a Xiaomi/Redmi/POCO and a Samsung, since they're the most aggressive at killing background apps. The emulator is fine for UI work but useless for battery and network tests.
- **Record the details** of every test device (model, Android version, OEM skin, router) in the reference devices table in [targets.md](../targets.md).

## 3. Create the repo

1. `git init` in `C:\dev\ecosystem-app`.
2. Create the folders from the layout in [README.md](../README.md).
3. Add a `.gitignore` covering Android (`.gradle/`, `build/`, `local.properties`), Rust (`target/`), Node (`node_modules/`) and .NET (`bin/`, `obj/`).
4. Optional: push to a private GitHub repo.
5. Use the identifiers decided in D13: Android `applicationId` `app.kinrelay.android`, Windows app identifier `app.kinrelay.desktop`. They're painful to change after a Play Store release.

## 4. Hello worlds

- **Android:** an "Empty Activity" (Compose) project with minSdk 29 is built in `android/`. Android Studio is optional. Run it on your phone.
- **Desktop: build both spikes.** Each one needs a tray icon with a Quit item.
  - **.NET:** a C# (WPF) app in `spikes/desktop-dotnet/`.
  - **Tauri:** run `bun create tauri-app` into `spikes/desktop-tauri/` (TypeScript, vanilla or Svelte), then `bun tauri dev`. It needs the Visual Studio Build Tools C++ workload from step 1, which needs admin to install.
  - **Compare** idle memory, install size and how hard the Windows integrations are (tray, toasts, clipboard listener, firewall rule, autostart). Then decide D2. The winner moves to `desktop/`.

## 5. Check the network before writing any networking code

This shows early whether your home network has the problems listed in [risks.md](../risks.md).

1. On the PC, run `bun tools/net-check.ts`. It serves a short text page on port 47821, prints a URL for each LAN address (the likely one is marked), and logs every request. It stops on Ctrl+C or after 10 minutes. See [tools/README.md](../../tools/README.md).
2. If Windows shows a firewall prompt, allow it, and note whether the network is "Private" or "Public". That feeds decision D12.
3. In the phone's browser, on Wi-Fi, open a printed URL. It should show "Kin Relay network check OK", and the PC's log should show a request from the phone's address.
4. If the page doesn't load, check the Windows firewall, whether you're on a guest network, and whether the router has "AP isolation" or "client isolation" turned on.

## 6. Warm-up (optional)

- **Kotlin:** coming from TypeScript, focus on null safety, data classes, and coroutines with Flow.
- **Rust:** the first chapters of The Rust Book, then the tokio tutorial. Ownership is the only part that feels truly new.

## Done when

- [ ] All tools are installed and the check commands work
- [ ] The Android hello world runs on your phone
- [ ] Both desktop spikes run with a tray icon and Quit
- [ ] The phone's browser reached the PC's test server
- [ ] The repo exists with the planned layout
- [ ] D2 (stack) is decided from the spike comparison. D13 (name) was decided on 2026-10-14
