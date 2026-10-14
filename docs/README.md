# Kin Relay: plan

The product is called **Kin Relay**. Inside the app it's just **Kin**. Technical identifiers use `kinrelay`. The Phase 2 fallback server (formerly called the relay) is called "the bridge". See D13 in [decisions.md](decisions.md).

**In one line:** your phone and your PC, finally acting like they belong to the same person. Any brand.

Status: planning. Name decided (D13). Phase 0 is in progress. The Android hello world and the .NET spike are built. The phone steps and the Tauri spike are still to do. Last updated 2026-10-14.

## Phases at a glance

| Phase | What | Done when |
|---|---|---|
| [0. Setup](phases/phase-0-setup.md) | Install toolchains, create the repo, hello-world apps, check the network | Both empty apps run, and the phone can reach the PC over Wi-Fi |
| [1. MVP](phases/phase-1-mvp.md) | Pairing, files, text, links and clipboard between one Android phone and one Windows PC on the same Wi-Fi | Targets met on test phones, friends test finished |
| [2. More features](phases/phase-2-features.md) | The bridge, presence, small ecosystem features, pretty UI, public launch | Order decided after Phase 1 |
| [Later](phases/later.md) | Screen mirroring, iPhone, accounts, and anything else | Revisit after Phase 2 |

## Read in this order

1. [product.md](product.md): what we're building, for whom, and why it beats LocalSend
2. [phases/](phases/): what gets done when
3. [architecture.md](architecture.md): how the pieces fit together
4. [protocol.md](protocol.md): the exact messages between phone and PC
5. [android.md](android.md) and [windows.md](windows.md): per-platform design
6. [targets.md](targets.md): what we optimize for and how we measure it
7. [decisions.md](decisions.md): what we decided and why
8. [risks.md](risks.md): what could sink it and how we'll find out early

## Open decisions

Work proceeds on the default until a decision is made. Details are in [decisions.md](decisions.md).

| # | Question | Default until decided |
|---|---|---|
| D2 | Stack | Kotlin on Android, Tauri v2 on Windows. A .NET (C#, WPF) alternative is being compared in Phase 0 |
| D6 | When the bridge is built | Phase 2, before public launch |
| D12 | Firewall rule on networks Windows marks "Public" | Allow, because every request is authenticated |
| - | Test phones | Your own phone, plus a borrowed Xiaomi and Samsung if possible |

## Planned repo layout

```
C:\dev\ecosystem-app\
  docs/       this plan
  android/    Android app (Kotlin, Jetpack Compose)
  desktop/    Windows app (Tauri v2: src-tauri/ is Rust, src/ is the TypeScript UI), unless the .NET spike wins D2
  spikes/     Phase 0 throwaway experiments (desktop-dotnet/, desktop-tauri/)
  protocol/   example messages and test vectors shared by both apps
  tools/      scripts for soak tests and benchmarks
```

## Keeping these docs useful

- When a decision changes, update [decisions.md](decisions.md) first, then the docs it affects.
- Write measured results to `docs/results/` (for example `results/phase-1.md`), not into the plan files.
