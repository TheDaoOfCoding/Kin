# End-to-end check of milestone 1.2 (text, links, clipboard) on the emulator: the real Windows app and the real debug APK.
# This is tools\e2e-1-1.ps1 -Milestone 1.2 (it does the setup, the pairing and the teardown); the scenarios are in
# tools\e2e-1-2-scenarios.ps1: 1 PC copy -> phone clipboard, 2 password-manager copy, 3 hotkey path, 4 share / PROCESS_TEXT /
# clipboard alias / notification action -> PC, 5 loop prevention, 6 links, 10 pairing confirmation, 11 logs.
#
#   tools\e2e-1-2.ps1                      # kin_api37
#   tools\e2e-1-2.ps1 -Avd kin_api29
#   tools\e2e-1-2.ps1 -Build               # build Kin.exe and the debug APK first
#   tools\e2e-1-2.ps1 -DiscardClipboard    # go on although the PC clipboard holds more than plain text (it is lost)
#
# It uses the REAL Windows clipboard: it saves the text on it first and puts it back at the end (a copy is also in
# %TEMP%\kin-e2e-clipboard-backup.txt until it is restored). Exit code 0 only if every check passed.
param(
    [string]$Avd = 'kin_api37',
    [switch]$Build,
    [switch]$KeepRunning,
    [switch]$DiscardClipboard,
    [string]$Kin,
    [string]$Apk,
    [string]$ScreenshotDir
)
$ErrorActionPreference = 'Continue'
& (Join-Path $PSScriptRoot 'e2e-1-1.ps1') -Milestone 1.2 @PSBoundParameters
exit $LASTEXITCODE
