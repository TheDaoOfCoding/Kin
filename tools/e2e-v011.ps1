# The end-to-end entry of milestone v0.1.1 (up to 4 files of a transfer at once, both directions): the real Kin.exe of desktop\build.ps1 as a test Kin
# BESIDE the installed Kin (own --data-dir; tools\kin-dev.ps1) and the real debug APK on the emulator. The scenarios are in android\tools\e2e-v011.ps1
# (this runs it; every parameter is passed on):
#
#   tools\e2e-v011.ps1                          # kin_api37: S1 to S7, S10, S11 (50 files each way, same names, cancel, Retry, the queue, Wi-Fi cut)
#   tools\e2e-v011.ps1 -Avd kin_api29 -Only S1,S2,S3,S7
#   tools\e2e-v011.ps1 -Only S8 -S8Runs 30      # the stall hunt: 30 Wi-Fi cuts per direction with the phone and PC logs kept (-StallDir)
#   tools\e2e-v011.ps1 -Only S9 -S9Runs 3       # 50 x 3.4 MB timings
#
# The regression of milestones 1.1 to 1.4 is tools\e2e-1-1.ps1 ... e2e-1-4.ps1 (they run beside the installed Kin the same way). One Kin script at a time
# (tools\kin-lock.ps1). Results: docs\results\v0-1-1-integration.md. Exit code 0 only if every check passed.
$ErrorActionPreference = 'Continue'
& (Join-Path $PSScriptRoot '..\android\tools\e2e-v011.ps1') @args
exit $LASTEXITCODE
