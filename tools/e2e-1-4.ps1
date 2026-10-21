# End-to-end check of milestone 1.4 (the queue, expiry, the 404 rules, sensitive text offline, the automatic clipboard offline, history, and
# notification grouping) on the emulator: the real Windows app and the real debug APK. This is tools\e2e-1-1.ps1 -Milestone 1.4 (it does the
# setup, the pairing and the teardown); the scenarios are in tools\e2e-1-4-scenarios.ps1:
#   S1 PC -> phone with Wi-Fi off (a text, a file, a text: queued, then delivered in order, timed from `svc wifi enable` and from the hello)
#   S2 phone -> PC with the PC stopped (a text and a shared file: queued, then delivered in order)
#   S3 the queue survives a restart (Kin.exe restart; phone force-stop and relaunch)
#   S4 cancel a queued item on each side: it never arrives
#   S5 expiry (--dev-clock-offset on the PC, EXPIRE --el ahead on the phone): failed "expired", never sent
#   S6 no ack: the item is re-queued, not failed, and arrives once (the receiver is frozen: NtSuspendProcess on Kin.exe, SIGSTOP on the phone app)
#   S7 the PC ends a phone upload while the phone is offline. (a) The PC cancels it: its stored cancel reaches the phone after the reconnect and the
#      phone ends canceled (peer_canceled), not failed (changelog 52). (b) The PC forgets it (--dev-clock-offset 25 h, no progress for a day): the
#      phone's automatic resume gets a 404 and fails it (not_found); a user Retry sends only the files without a 200, under a new ID; no "name (1)" copies
#   S8 a sensitive text while the peer is offline is not queued (both directions)
#   S9 the automatic clipboard while the phone is offline is neither queued nor sent stale
#   S10 history parity between the two sides
#   S11 notification burst and storm (-BurstTexts, -BurstFiles; the full check is 60 and 15)
#   S13 logs (no names, texts or secrets) and Export logs on both sides
#
#   tools\e2e-1-4.ps1                       # kin_api37
#   tools\e2e-1-4.ps1 -Avd kin_api29
#   tools\e2e-1-4.ps1 -Build                # build Kin.exe and the debug APK first
#   tools\e2e-1-4.ps1 -Only S1,S8           # only these scenarios (the pairing still happens); S10 and S13 need what the others made
#   tools\e2e-1-4.ps1 -BurstTexts 60 -BurstFiles 15
#
# It uses the REAL Windows clipboard: every format is saved first and put back at the end, after Kin has quit. Received files go to a throwaway
# folder inside the throwaway data dir, never to the real Downloads; the autostart, SendTo and the Run key are never touched (Kin runs with
# --dev-bind 127.0.0.1 and --data-dir). Exit code 0 only if every check passed. A scenario that dies is recorded as a failed check and the run goes on
# with the next one. The script refuses a Kin.exe that is not from `tauri build` (`cargo test --release` replaces it with a dev-server build): use -Build.
param(
    [string]$Avd = 'kin_api37',
    [switch]$Build,
    [switch]$KeepRunning,
    [string[]]$Only = @(),
    [int]$BigMb = 40,
    [int]$BurstTexts = 20,
    [int]$BurstFiles = 6,
    [switch]$UpdateScreens,
    [string]$ScreenshotDir,
    [string]$Kin,
    [string]$Apk
)
# Only one of the Kin scripts (e2e, smoke, capture, check) may run at a time: they share the real clipboard, the Kin.exe slot and the emulator (tools\kin-lock.ps1).
. (Join-Path $PSScriptRoot 'kin-lock.ps1'); Enter-KinLock 'e2e-1-4'
$ErrorActionPreference = 'Continue'
& (Join-Path $PSScriptRoot 'e2e-1-1.ps1') -Milestone 1.4 @PSBoundParameters
exit $LASTEXITCODE
