# The 1-minute HUMAN check of what a script cannot do (a real click, a typed reply, from the popup and
# from the Action Center). Nothing here clicks for you. Spike-only keys, removed by -Step Cleanup.
#
#   1.  .\manual-check.ps1 -Step Show              registers, shows two silent toasts, and no process stays running
#   2.  click Send (after typing a word) on the popup within ~7 s; then open the Action Center (Win+N),
#       type a word in the "Kin spike m2" toast and click Send; click Copy on one of them
#   3.  .\manual-check.ps1 -Step Check             shows what the app received
#   4.  .\manual-check.ps1 -Step Show -Stay 60     same toasts, but the app is RUNNING for 60 s: click Send on the
#       popup and on m2, then press the X of one toast; then -Step Check again
#   5.  .\manual-check.ps1 -Step Cleanup           removes the toasts and every spike key
#
# Expected in Check: lines "ACTIVATE aumid=... args=... input[reply]=..." for every click. After step 1-2 they come
# from a process that COM started (a line "start mode=--toast-activated ... -Embedding"); after step 4 they come
# from the process that showed the toasts. "EVENT Activated (in-process)" tells whether the popup click ALSO reaches
# the in-process handler; "EVENT Dismissed ... reason=UserCanceled" is the X.
param([Parameter(Mandatory)][ValidateSet('Show', 'Check', 'Cleanup')][string]$Step, [int]$Stay = 0)
$ErrorActionPreference = 'Stop'
$root = $PSScriptRoot
$out = Join-Path $root 'out'
$exe = Join-Path (Join-Path $out 'with space') 'toast-spike.exe'
$log = Join-Path $out 'spike.log'

switch ($Step) {
    'Show' {
        $bad = Get-Process -ErrorAction SilentlyContinue | Where-Object { $_.ProcessName -match 'EasyAntiCheat|^EAC|BEService|vgc|FortniteClient|VALORANT' }
        if ($bad) { Write-Host "STOP: game/anti-cheat running ($($bad.ProcessName -join ', '))"; exit 2 }
        Push-Location $root
        try { cargo build --release --offline --quiet; if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' } } finally { Pop-Location }
        New-Item -ItemType Directory -Force (Split-Path $exe) | Out-Null
        Copy-Item (Join-Path $root 'target\release\toast-spike.exe') $exe -Force
        if (-not (Test-Path (Join-Path $out 'icon.png'))) { throw 'out\icon.png missing' }
        & $exe register | Out-Null
        & $exe manual-show $Stay | Out-Null
        Write-Host "Toasts shown (stay=$Stay s). Now click, then run: .\manual-check.ps1 -Step Check"
    }
    'Check' {
        $lines = Get-Content $log
        $first = ($lines | Select-String 'MANUAL-SHOW m1' | Select-Object -First 1).LineNumber
        if (-not $first) { Write-Host 'no manual run found'; return }
        $lines | Select-Object -Skip ($first - 1) | Where-Object { $_ -match 'MANUAL-SHOW|ACTIVATE|EVENT|COM-LAUNCHED|mode="--toast-activated"' }
    }
    'Cleanup' {
        if (Test-Path $exe) {
            & $exe clear | Out-Null
            & $exe unregister | Out-Null
        }
        Get-Process -Name 'toast-spike' -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $exe } | Stop-Process -Force -ErrorAction SilentlyContinue
        Write-Host 'cleaned: toasts and HKCU keys removed'
        reg query 'HKCU\Software' /s /f 'app.kinrelay.spike' 2>$null | Select-Object -Last 1
    }
}
exit 0
