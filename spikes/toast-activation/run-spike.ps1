# Spike S1 driver: registers a spike-only AUMID + CLSID under HKCU, runs the scenarios, and ALWAYS
# removes every key and every toast again (finally). No input injection: activation is driven by a
# test client that calls the registered COM class the way the shell does.
#
#   powershell -File run-spike.ps1     (about 1.5 minutes, 3 silent popups, the rest suppressed)
$ErrorActionPreference = 'Stop'
$root = $PSScriptRoot
$out = Join-Path $root 'out'
$binDir = Join-Path $out 'with space'              # a space on purpose: proves the quoted LocalServer32 path
$exe = Join-Path $binDir 'toast-spike.exe'
$log = Join-Path $out 'spike.log'
$clsidKey = 'HKCU:\Software\Classes\CLSID\{D4F1A6B2-7C3E-4B58-9A10-5E6C0B7A21F3}'
$started = New-Object System.Collections.Generic.List[int]

function Say($m) { Write-Host "[driver] $m" }

# ---- rule 6: nothing that could pop up over a game
$bad = Get-Process -ErrorAction SilentlyContinue | Where-Object { $_.ProcessName -match 'EasyAntiCheat|^EAC|BEService|vgc|FortniteClient|VALORANT' }
if ($bad) { Say "STOP: anti-cheat/game process running: $($bad.ProcessName -join ', ')"; exit 2 }

# ---- build (own target dir) and copy to a path with a space
Push-Location $root
try { cargo build --release --offline --quiet; if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' } } finally { Pop-Location }
New-Item -ItemType Directory -Force $binDir | Out-Null
Copy-Item (Join-Path $root 'target\release\toast-spike.exe') $exe -Force
if (-not (Test-Path (Join-Path $out 'icon.png'))) { throw 'out\icon.png missing' }
Remove-Item $log -ErrorAction SilentlyContinue

try {
    Say 'register (HKCU only)'
    & $exe register

    Say 'SCENARIO 1: app running (class object registered), toasts shown, then activation through COM'
    $serve = Start-Process -FilePath $exe -ArgumentList 'serve', '120', '2' -PassThru -WindowStyle Hidden
    $started.Add($serve.Id)
    $deadline = (Get-Date).AddSeconds(150)
    while ((Get-Date) -lt $deadline) {
        if ((Test-Path $log) -and (Select-String -Path $log -Pattern 'SCENARIO_DONE|REFUSED' -Quiet)) { break }
        if ($serve.HasExited) { break }
        Start-Sleep -Milliseconds 500
    }
    if (-not $serve.HasExited) {
        Say '(a) activation while the app is running: button + reply text, as the shell would pass them'
        & $exe client 'kin=1&n=t1&a=reply' 'reply=hello from the test client'
        & $exe client 'kin=1&n=t1&a=copy'
        Wait-Process -Id $serve.Id -Timeout 30 -ErrorAction SilentlyContinue
    }

    Say 'SCENARIO 2: the creating process is gone; toasts still in the Action Center? (fresh process)'
    & $exe history
    Say '   ... and can a fresh process remove one of them by tag + group (notif.remove after a restart)?'
    & $exe remove t1

    Say 'SCENARIO 3: app NOT running -> COM must start it through LocalServer32 and deliver the call'
    $alive = Get-Process -Name 'toast-spike' -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $exe }
    if ($alive) { Say 'a spike process is still alive; waiting'; $alive | Wait-Process -Timeout 20 -ErrorAction SilentlyContinue }
    & $exe client 'kin=1&n=t2&a=reply' 'reply=hello cold start'
    # the COM-launched process ends itself ~1 s after the call
    Start-Sleep -Seconds 3
}
finally {
    Say 'CLEANUP: toasts, then registry'
    try { & $exe clear } catch { Say "clear failed: $_" }
    try { & $exe unregister } catch { Say "unregister failed: $_" }
    # only processes of the spike exe (we started them, or COM started them for us)
    foreach ($id in $started) { Stop-Process -Id $id -Force -ErrorAction SilentlyContinue }
    Get-Process -Name 'toast-spike' -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $exe } | Stop-Process -Force -ErrorAction SilentlyContinue
    Say "leftover CLSID key: $(Test-Path $clsidKey)"
    Say 'leftover search in HKCU\Software for "app.kinrelay.spike" (reg.exe, read only):'
    $hits = reg query 'HKCU\Software' /s /f 'app.kinrelay.spike' 2>$null | Where-Object { $_ -match 'kinrelay.spike|End of search: [1-9]' }
    if ($hits) { $hits | ForEach-Object { Say "  $_" } } else { Say '  none' }
}
Say "log: $log"
exit 0
