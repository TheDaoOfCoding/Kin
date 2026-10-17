# toast-activation (spike S1 of 2B.2)

Throwaway. Tests whether an unpackaged Win32 app can show toasts under its own AUMID (registered under
`HKCU\Software\Classes\AppUserModelId`) and be activated through a COM class. Results and the registration
recipe: [docs/results/phase-2b-2-spike-s1.md](../../docs/results/phase-2b-2-spike-s1.md).

- `run-spike.ps1`: builds, registers spike-only keys (AUMID `app.kinrelay.spike.toast`), runs the scenarios, and
  removes every key and toast in a `finally`. About 1.5 minutes, silent. No input injection.
- `manual-check.ps1`: the 1-minute human check (real clicks, typed reply). Steps are at the top of the file.
- `src/main.rs`: the exe (modes: `register`, `unregister`, `serve`, `client`, `history`, `remove`, `clear`,
  `manual-show`, `--toast-activated`).
- `evidence/full-run.log`: the log of the full run the results quote. `out/` and `target/` are ignored.

Never touches the installed Kin (`app.kinrelay.desktop`).
