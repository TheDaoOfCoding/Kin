# Contributing to Kin Relay

Thanks for helping. Kin Relay is early alpha software, so bug reports, test results from other phones and PCs, and focused pull requests are all welcome.

Before you start on something big, open an issue and describe it, so we can agree on the direction first.

## The CLA

Kin Relay is released under the [AGPL-3.0](LICENSE) and is also available under a [commercial license](COMMERCIAL-LICENSE.md). To keep that possible, **by submitting a contribution (a pull request, a patch) you agree to the terms of the [Contributor License Agreement](CLA.md)**. You keep the copyright to your work; the CLA gives the project the right to license it under both. The pull request template asks you to confirm this.

## Repository layout

| Folder | What |
|---|---|
| `android/` | The Android app (Kotlin, Jetpack Compose) |
| `desktop/` | The Windows app (Tauri v2: `src-tauri/` is Rust, `src/` is the TypeScript UI) and `desktop/native-host/`, the browser extension's native messaging host |
| `extension/` | The browser extension (Manifest V3, TypeScript, built with bun) |
| `protocol/` | Test vectors and fixtures shared by all of the above, and `check-vectors.ts` |
| `tools/` | PowerShell and bun scripts for end-to-end tests on an emulator |
| `docs/` | The protocol, architecture and per-platform documentation |

The wire contract is [docs/protocol.md](docs/protocol.md). **If you change behaviour on the wire, change `docs/protocol.md` and the vectors in `protocol/` in the same pull request**, and run the tests of both apps, because both read the same vector files.

## What you need

Development happens on **Windows 10/11 x64**. Scripts are PowerShell 5.1 (`powershell`), not PowerShell 7.

- [Rust](https://rustup.rs): the toolchain is pinned by `rust-toolchain.toml`; `rustup` installs it on first use (with `rustfmt`, `clippy` and the `x86_64-pc-windows-msvc` target). You also need the MSVC build tools (Visual Studio Build Tools, "Desktop development with C++").
- [bun](https://bun.sh) 1.4 or newer (the UI and the extension).
- JDK 21 and the Android SDK (platform 37) for the Android app. Set `JAVA_HOME` and `ANDROID_HOME`.
- For the end-to-end tests: the Android emulator with two AVDs named **`kin_api37`** (Android 17) and **`kin_api29`** (Android 10), see [tools/emulator/README.md](tools/emulator/README.md).
- For installers: the Tauri CLI is pinned to **exactly 2.12.2** (`desktop/package.json`) because `desktop/src-tauri/windows/installer.nsi` is a modified copy of that version's NSIS template. `desktop\build.ps1` stops if the version differs.

## Building and testing each part

Run the tests of the part you changed before you open a pull request. CI runs all of them (see [`.github/workflows/ci.yml`](.github/workflows/ci.yml)).

### Protocol vectors

```powershell
bun protocol/check-vectors.ts
```

### Windows app (`desktop/src-tauri`) and native host (`desktop/native-host`)

```powershell
cd desktop
bun install --frozen-lockfile
bun run build                 # the UI into dist/; cargo needs dist/ to exist
cd src-tauri
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cd ..\native-host
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

The tests use a fake phone over real TLS on loopback and never touch your real clipboard, Downloads folder or an installed Kin. To build the installer: `powershell -ExecutionPolicy Bypass -File desktop\build.ps1` (the output is `desktop\src-tauri\target\release\bundle\nsis\`). More in [desktop/README.md](desktop/README.md).

### Android app (`android/`)

```powershell
cd android
.\gradlew.bat testDebugUnitTest          # unit tests, no signing needed
.\gradlew.bat lintDebug assembleDebug
```

The release build is signed with a key that is not in the repository; without one `assembleRelease` produces an unsigned APK. Never commit a keystore or `signing.properties`. More in [android/README.md](android/README.md).

### Browser extension (`extension/`)

```powershell
cd extension
bun install --frozen-lockfile
bun run typecheck
bun test tests/unit
bun run smoke                  # headless Chromium against a mock host (optional; needs the Playwright Chromium)
```

### End-to-end tests on an emulator

The scripts in `tools/` (see [tools/README.md](tools/README.md)) start the real `Kin.exe` as a **test instance beside any installed Kin** (own data directory, loopback only) and the debug APK on an emulator. Build both first:

```powershell
powershell -ExecutionPolicy Bypass -File desktop\build.ps1
cd android; .\gradlew.bat assembleDebug; cd ..
powershell -ExecutionPolicy Bypass -File tools\e2e-1-1.ps1                       # kin_api37 by default
powershell -ExecutionPolicy Bypass -File tools\e2e-1-1.ps1 -Avd kin_api29        # Android 10
```

These scripts never touch an installed Kin, your clipboard or a real phone, and they refuse to run while a full-screen game or an anti-cheat is running. Only one of them can run at a time (they share the emulator and a lock). Please do not add scripts that inject mouse or keyboard input.

## Code style

- **Rust:** `cargo fmt` (the settings are in `rustfmt.toml`: 140 columns) and `cargo clippy --all-targets -- -D warnings` must be clean. No blanket `#[allow]`; if you must silence a lint, do it on the smallest item and say why in a comment.
- **Kotlin:** follow the surrounding code; no new dependency injection framework, keep the app small. Pure logic goes in small classes that the unit tests can run without Android.
- **TypeScript:** `bun run typecheck` must be clean.
- Match the surrounding code's naming and comment density. Comments explain **why**, not what.
- Some older comments cite milestones (`2B.3`), decisions (`D12`) or risks (`R7`) from the maintainer's design notes, which are not public. New comments should give the reason itself.
- Add or update tests with the change. A bug fix comes with a test that fails without the fix.
- Keep the apps' privacy promises: no telemetry, no network access beyond the local network and the user's own devices, and never log the text of what was sent.

## Pull requests

- Keep them focused: one change, one pull request.
- Write a clear description: what changed, why, and how you tested it (which phone or emulator, which Windows version).
- Do not include secrets, personal data, real pairing codes or logs with private content.
- Be kind. We follow the spirit of the [Contributor Covenant](https://www.contributor-covenant.org/version/2/1/code_of_conduct/).

## Reporting bugs and security problems

Bugs go into [GitHub issues](https://github.com/TheDaoOfCoding/Kin/issues/new/choose); please use the template and **do not paste notification contents, clipboard text or pairing codes**. Security problems go through GitHub's private vulnerability reporting, see [SECURITY.md](SECURITY.md).
