# Kin Relay

Your Android phone and your Windows PC, acting like they belong to the same person. Any phone brand. Everything stays on your own network: no account, no cloud, no telemetry.

> **Status: early alpha (0.2.x).** It works, but it has been tested on very little hardware. Please read [Status and testing](#status-and-testing) and [Known issues](#known-issues) before you rely on it.

## What it does

- **Send files, text and links** between phone and PC, from the share sheet, the text selection menu, the tray, a hotkey, the "Send to" menu or drag and drop. Nothing to open on the receiving side.
- **Clipboard sync:** copy on the PC, paste on the phone (and send selected text the other way). Content that a password manager marked as sensitive is never synced automatically.
- **Phone notifications on the PC** (opt-in, per app), with reply, buttons and "Copy code" for one-time codes.
- **Media controls both ways:** the phone's player in the Windows media flyout, the PC's player on the phone's lock screen and quick settings.
- **Battery and presence:** see the other device's battery, and get a warning when it runs low.
- **Ring my phone** from the PC (it plays on the alarm volume, so it works on silent), and **Pause** everything with one tap.
- **Do Not Disturb sync:** the phone goes quiet while your PC is in Focus.
- **Browser extension** (Chrome, Edge, Brave, Vivaldi, Opera, Firefox): right-click "Send to Kin" on links, text, images and pages, and "Send this tab" with the YouTube time.
- **Alerts:** `Kin.exe --notify "Build finished"` from a script, or "Tell my phone when a download finishes" from the browser extension.
- Messages wait in a queue when the other device is offline, and a history shows what was sent.

Screenshots are coming.

## Status and testing

Early alpha. Honest list of what has been tried:

- **Tested on:** Android emulators (API 29 = Android 10 and API 37 = Android 17), and one real Android 14 phone.
- **Not tested:** Android 15 and 16 on real phones, other phone brands (the battery-optimisation setup for Xiaomi, Samsung, Oppo and others is written from documentation, not from a phone).
- **Windows:** Windows 10 and 11, x64 only. ARM is not supported. Most testing was on Windows 11.
- **Network:** the phone and the PC must be on the **same local network**. There is no relay server yet, so it does not work across the internet or between different networks.
- The Windows installer is **not code-signed**, so SmartScreen shows "Windows protected your PC" (More info, then Run anyway).
- The browser extension is **side-load only** (not in any store yet).

## Install

Everything is on the [GitHub Releases](https://github.com/TheDaoOfCoding/Kin/releases) page:

1. **Windows:** run `Kin_<version>_x64-setup.exe`. It installs for all users (administrator rights needed), adds a Windows Firewall rule for your local subnet, and starts Kin in the tray.
2. **Android:** download the APK to the phone and install it (allow "install unknown apps" for your browser or file manager when Android asks). Android 10 or newer.
3. **Pair them:** on the PC, click the Kin tray icon and choose "Pair new phone"; in the phone app, tap "Scan code" and scan the QR code. If the phone's camera cannot scan it, use "Copy pairing link" on the PC and paste the link in the phone app.
4. **Browser extension (optional):**
   - Chrome, Edge, Brave, Vivaldi, Opera: unzip `kin-chromium-<version>-sideload.zip` into a folder you keep, open the browser's extensions page, switch on *Developer mode*, choose *Load unpacked* and pick that folder.
   - Firefox: `about:debugging#/runtime/this-firefox`, *Load Temporary Add-on*, pick `manifest.json` from the unzipped `kin-firefox-<version>.zip` (it is removed when Firefox closes).
   - The extension only works when Kin is installed on the PC. Details: [extension/README.md](extension/README.md).

## Build from source

See [CONTRIBUTING.md](CONTRIBUTING.md) for the toolchain and for how to build and test each part (Windows app, Android app, browser extension, protocol vectors, emulator end-to-end tests). In short: Windows, Rust (pinned by `rust-toolchain.toml`), bun, JDK 21 and the Android SDK.

## How it works

- The PC runs a small HTTPS server and announces itself on the local network with mDNS. The phone always connects out to it and keeps one idle connection open.
- You pair once with a **QR code** that holds the PC's address, its key fingerprint and a one-time token.
- All traffic uses **mutual TLS 1.3**. Each device has its own key and only trusts the public key of the device it was paired with. There is no certificate authority and no account.
- **Nothing goes through a server.** Files go straight from device to device over your Wi-Fi or Ethernet.
- The protocol is documented in [docs/protocol.md](docs/protocol.md) with shared test vectors in [protocol/](protocol/); the design is in [docs/architecture.md](docs/architecture.md).

## Known issues

- **Some phones kill background apps.** Xiaomi, Oppo, Vivo, Huawei and some Samsung settings stop Kin's connection service unless you exempt it from battery optimisation (the app's setup screen guides you). PC-to-phone sends wait in a queue until the phone reconnects.
- **Guest networks and "client isolation"** on a router stop the phone from reaching the PC. So does a phone on a different subnet than the PC (the firewall rule is limited to the PC's subnet; its scope can be edited).
- **Notification mirroring on a sideloaded APK** needs one extra step on Android 13 and later: "Allow restricted settings" in the app's info page before notification access can be granted. The setup screen explains it.
- **Windows 10** is supported but much less tested than Windows 11.
- **Firefox** keeps the extension only until it closes (no signed add-on yet), and Firefox for Android is not supported.
- Transfer speed has only been measured on one phone and one home network.
- No iPhone, macOS or Linux support.
- The installer is not code-signed and the APK is signed with the project's own key, so you will see warnings.

Please report anything else through [GitHub issues](https://github.com/TheDaoOfCoding/Kin/issues/new/choose).

## Security

See [SECURITY.md](SECURITY.md) for the security model and how to report a vulnerability privately.

## License

Kin Relay is free software under the **[GNU AGPL-3.0](LICENSE)** (`AGPL-3.0-only`). A **[commercial license](COMMERCIAL-LICENSE.md)** is available from the copyright holder if the AGPL does not fit your use. Third-party software that ships with Kin is listed in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

## Contributing

Contributions are welcome; read [CONTRIBUTING.md](CONTRIBUTING.md) first. By contributing you agree to the [Contributor License Agreement](CLA.md), which lets the project keep offering the dual license.
