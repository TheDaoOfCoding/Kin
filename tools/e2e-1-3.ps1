# End-to-end check of milestone 1.3 (files in both directions, and the text rules of the 1.2 review) on the emulator: the real Windows app
# and the real debug APK. This is tools\e2e-1-1.ps1 -Milestone 1.3 (it does the setup, the pairing and the teardown); the scenarios are in
# tools\e2e-1-3-scenarios.ps1: S1 one photo and S2 50 photos phone -> PC, S3 a big file with a Wi-Fi drop, S4 one photo, S5 a mixed batch and
# S6 a big file with a drop PC -> phone, S7 cancel from each side in both directions, S8 disk_full and source_changed, S9 tricky file names
# both ways, S10 the text rules (sensitive, 128 KiB as a .txt, LF/CRLF, openable URLs), S12 the logs.
#
#   tools\e2e-1-3.ps1                      # kin_api37
#   tools\e2e-1-3.ps1 -Avd kin_api29
#   tools\e2e-1-3.ps1 -Build               # build Kin.exe and the debug APK first
#   tools\e2e-1-3.ps1 -BigMb 300           # a bigger "big file" for S3, S6 and S8 (the emulator does about 2 MB/s: 40 MB is about 20 s)
#   tools\e2e-1-3.ps1 -Only S1,S4          # only these scenarios (the pairing still happens)
#
# It uses the REAL Windows clipboard: every format is saved first and put back at the end, after Kin has quit. Received files go to a throwaway
# folder inside the throwaway data dir, never to the real Downloads. Exit code 0 only if every check passed.
param(
    [string]$Avd = 'kin_api37',
    [switch]$Build,
    [switch]$KeepRunning,
    [string[]]$Only = @(),
    [int]$BigMb = 40,
    [switch]$UpdateScreens,
    [string]$ScreenshotDir,
    [string]$Kin,
    [string]$Apk
)
# Only one of the Kin scripts (e2e, smoke, capture, check) may run at a time: they share the real clipboard, the Kin.exe slot and the emulator (tools\kin-lock.ps1).
. (Join-Path $PSScriptRoot 'kin-lock.ps1'); Enter-KinLock 'e2e-1-3'
$ErrorActionPreference = 'Continue'
& (Join-Path $PSScriptRoot 'e2e-1-1.ps1') -Milestone 1.3 @PSBoundParameters
exit $LASTEXITCODE
