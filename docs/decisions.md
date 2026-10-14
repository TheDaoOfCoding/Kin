# Decisions

Status values:

- **Decided:** settled. Change it only with a new entry that explains why.
- **Default:** what we do unless you say otherwise.
- **Open:** still needs an answer.

| # | Decision | Why | Status |
|---|---|---|---|
| D1 | The MVP covers Android and Windows only | No iPhone to test on, and Android + Windows is the biggest group nobody serves | Decided |
| D2 | Kotlin (native) on Android, Tauri v2 (Rust + TypeScript UI) on Windows | Most of the Android app is system integration that needs Kotlin anyway. Tauri is light at idle, its UI uses TypeScript you already know, and it brings Mac and Linux later. Alternative: Flutter on both, which is quicker to a first version but heavier at idle, and the Android service parts would still need Kotlin. A .NET alternative (C#, WPF) is now being compared against Tauri in Phase 0. The .NET hello world is built in `spikes/desktop-dotnet/`. The Tauri one is still to come, in `spikes/desktop-tauri/`. The winner moves to `desktop/` | Default (Tauri) |
| D3 | The PC serves and the phone connects out | The phone never listens, scans or advertises, which protects battery | Decided, pending the 1.1 spike |
| D4 | HTTPS/1.1 plus one WebSocket, with mutual TLS and keys pinned at pairing | Easy to debug with curl, HTTP Range requests give resume for free, and it's a familiar stack. Alternatives were a custom TCP protocol, QUIC or Noise | Decided |
| D5 | Pair by QR code, no account | No sign-up, no server, more private | Decided |
| D6 | No bridge (the Phase 2 fallback server, formerly called the relay) in Phase 1. It's required before a public launch | It's a server to run and pay for, and it doesn't help prove the core idea. Some networks block device-to-device traffic, so launching without it isn't an option | Default |
| D7 | No Bluetooth for moving data | Wi-Fi is far faster. Bluetooth LE might be used later, only for proximity | Decided |
| D8 | Android 10+ (minSdk 29) | Simpler storage APIs (MediaStore downloads), and covers the large majority of phones in use | Decided |
| D9 | Basic UI until Phase 2 | Your call: make it work first, then make it pretty | Decided |
| D10 | No telemetry in the MVP. Local logs with an export button | Privacy, and fewer moving parts | Decided |
| D11 | Clipboard: text only. PC → phone automatic, phone → PC manual | Android 10+ blocks background apps from reading the clipboard | Decided |
| D12 | Firewall rule on networks Windows marks "Public" | Windows 11 marks new networks Public by default, so a Private-only rule fails for many people. Allowing Public exposes the port on café Wi-Fi, but every request is authenticated | Open. Default: allow both |
| D13 | Product name and app identifiers | The brand, store and website name is **Kin Relay**. The short name in the app (UI, tray, notifications) is **Kin**. User-visible folders use `Kin`: `Downloads\Kin\`, `Pictures/Kin`, `Movies/Kin`, `Download/Kin` and `%APPDATA%\Kin\`. Identifiers that must be globally unique use `kinrelay`: Android applicationId `app.kinrelay.android`, Windows app identifier `app.kinrelay.desktop`, mDNS service `_kinrelay._tcp` and pairing scheme `kinrelay://`. The Windows executable is `kin.exe` and the firewall rule is named "Kin Relay". The Phase 2 relay server is called "the bridge", so "relay" only appears in the product name | **Decided (2026-10-14)** |

## Changes from the original chat

- **Bridge timing.** The chat said to build the fallback server (formerly called the relay, now the bridge) from day one. It's now Phase 2, before launch (D6).
- **Dragging onto the tray icon.** Windows doesn't support dropping files onto a tray icon. Clicking the tray icon opens a small drop window instead.
- **Hotspot handoff.** Android doesn't let regular apps turn on the phone's hotspot, so this became "hotspot help" in Phase 2.
