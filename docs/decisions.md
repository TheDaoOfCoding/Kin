# Decisions

Status values:

- **Decided:** settled. Change it only with a new entry that explains why.
- **Default:** what we do unless you say otherwise.
- **Open:** still needs an answer.

| # | Decision | Why | Status |
|---|---|---|---|
| D1 | The MVP covers Android and Windows only | No iPhone to test on, and Android + Windows is the biggest group nobody serves | Decided |
| D2 | Kotlin (native) on Android, Tauri v2 (Rust backend, TypeScript UI) on Windows | Most of the Android app is system integration that needs Kotlin anyway. Tauri won the Phase 0 comparison against .NET (WPF) on the numbers in [phase-0-tauri.md](results/phase-0-tauri.md) and [phase-0-dotnet.md](results/phase-0-dotnet.md): a 1.1 MB NSIS installer against 148 MB for self-contained .NET, so it meets the ~10 MB size target. rustls gives TLS 1.3 on Windows 10 and 11, as [protocol.md](protocol.md) requires, while Kestrel with Schannel gives TLS 1.2 on Windows 10. It idles at about 1.5 MB private working set with no window. It also brings Mac and Linux later. How hard the languages are to learn was not a factor, because AI agents write all the code. Flutter was considered and rejected: heavier at idle, and the Android service parts would still need Kotlin. The .NET spike stays in `spikes/desktop-dotnet/` as a record of the comparison. The Tauri app moves to `desktop/` in Phase 1 | **Decided (2026-10-14)** |
| D3 | The PC serves and the phone connects out | The phone never listens, scans or advertises, which protects battery | Decided, pending the 1.1 spike |
| D4 | HTTPS/1.1 plus one WebSocket, with mutual TLS and keys pinned at pairing | Easy to debug with curl, HTTP Range requests give resume for free, and it's a familiar stack. Alternatives were a custom TCP protocol, QUIC or Noise | Decided |
| D5 | Pair by QR code, no account | No sign-up, no server, more private | Decided |
| D6 | No bridge (the Phase 2 fallback server, formerly called the relay) in Phase 1. It's required before a public launch | It's a server to run and pay for, and it doesn't help prove the core idea. Some networks block device-to-device traffic, so launching without it isn't an option | Default |
| D7 | No Bluetooth for moving data | Wi-Fi is far faster. Bluetooth LE might be used later, only for proximity | Decided |
| D8 | Android 10+ (minSdk 29) | Simpler storage APIs (MediaStore downloads), and covers the large majority of phones in use | Decided |
| D9 | Basic UI until Phase 2 | Your call: make it work first, then make it pretty | Decided |
| D10 | No telemetry in the MVP. Local logs with an export button | Privacy, and fewer moving parts | Decided |
| D11 | Clipboard: text only. PC → phone automatic, phone → PC manual | Android 10+ blocks background apps from reading the clipboard | Decided |
| D12 | Firewall rule profiles | Windows 11 marks new networks Public by default, so a Private-only rule fails for many people. The installer's rule allows both the Private and Public profiles. That exposes the port on café Wi-Fi, but every request is authenticated (see R8) | **Decided (2026-10-14)** |
| D13 | Product name and app identifiers | The brand, store and website name is **Kin Relay**. The short name in the app (UI, tray, notifications) is **Kin**. User-visible folders use `Kin`: `Downloads\Kin\`, `Pictures/Kin`, `Movies/Kin`, `Download/Kin` and `%APPDATA%\Kin\`. Identifiers that must be globally unique use `kinrelay`: Android applicationId `app.kinrelay.android`, Windows app identifier `app.kinrelay.desktop`, mDNS service `_kinrelay._tcp` and pairing scheme `kinrelay://`. The Windows executable is `kin.exe` and the firewall rule is named "Kin Relay". The Phase 2 fallback server (formerly called the relay) is called the bridge, so "relay" only appears in the product name | **Decided (2026-10-14)** |
| D14 | Build order | Agents build all of Phase 1 (milestones 1.1–1.4, plus the packaging in 1.5) before you test on real phones. During development, agents test on the Android emulator (`tools/emulator/`) against the desktop app on the same PC. The 1.1 soak gate still applies. It runs on the real phones once the build is ready, and you can install the 1.1 build early and soak it while agents continue. If the gate fails, the connection layer gets reworked (for example the FCM wake-up moves into Phase 1) and the features above it mostly stay. See [phase-1-mvp.md](phases/phase-1-mvp.md) | **Decided (2026-10-14)** |
| D15 | Android identity key storage | The P-256 identity key is a software key, stored encrypted with an AES-GCM key held in the Android Keystore. Keystore-resident private keys can't reliably be used for TLS client authentication through OkHttp/JSSE on every phone. A hardware-backed key can be an upgrade later. Resolves R11 | **Decided (2026-10-14)** |

## Changes from the original chat

- **Bridge timing.** The chat said to build the fallback server (formerly called the relay, now the bridge) from day one. It's now Phase 2, before launch (D6).
- **Dragging onto the tray icon.** Windows doesn't support dropping files onto a tray icon. Clicking the tray icon opens a small drop window instead.
- **Hotspot handoff.** Android doesn't let regular apps turn on the phone's hotspot, so this became "hotspot help" in Phase 2.
