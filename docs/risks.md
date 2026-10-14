# Risks

| # | Risk | Likelihood | Impact | What we do about it | When we find out |
|---|---|---|---|---|---|
| R1 | OEMs (Xiaomi, Oppo, Vivo, Samsung) kill the background service, so PC → phone stops arriving | High | High | Battery-exemption onboarding with OEM-specific steps, plus the queue. If needed, the FCM wake-up push moves into Phase 1 | 1.1 soak test |
| R2 | The network blocks devices from talking to each other (guest Wi-Fi, campus networks, AP isolation) | Medium | High | Check your own network in Phase 0. Show a clear message instead of failing silently. The bridge arrives in Phase 2 | Phase 0, then the friends test |
| R3 | The Windows firewall blocks connections, especially on networks marked "Public" | Medium | High | The installer adds the rule (D12). The debug screen shows the current network profile | 1.5 |
| R4 | The Play Store rejects or delays the foreground service | Medium | Medium | Install the APK directly in the MVP. Prepare the declaration and demo video in Phase 2 | Phase 2 |
| R5 | Access to shared files expires before a queued file is sent | Medium | Medium | Pass file access on to the service. Copy small files to the cache. Ask the user to share large ones again | 1.3 |
| R6 | Agent-written Android and Windows code drifts from the protocol, or from each other | Medium | High | Shared protocol test vectors in `protocol/`, used by both apps' tests. Emulator-based integration tests every milestone. An Opus review before every commit | Every milestone |
| R7 | Appending to a file for resume behaves differently on some phones | Low | Medium | Fall back to writing in the app's folder and moving the file when it's complete | 1.3 |
| R8 | The PC's open port is reachable on public Wi-Fi | Low | Medium | Every request is authenticated, TLS is required, and pairing attempts are rate-limited. A setting can limit listening to Private networks | 1.5 |
| R9 | Clipboard sync leaks secrets such as passwords | Medium | Medium | Skip password-manager content, provide an on/off toggle, and keep clipboard sync out of history | 1.2 |
| R10 | Toast buttons don't work for an app that isn't packaged | Low | Low | Click-only toasts | 1.3 |
| R11 | The Android Keystore key can't be used for the TLS client certificate on some phones | Low | Medium | Resolved by D15: the identity key is a software key, encrypted with a Keystore-held AES-GCM key. A hardware-backed key can be an upgrade later | 1.1 |
