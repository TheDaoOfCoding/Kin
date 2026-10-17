# Milestone 2B.2 integration (phone notifications mirrored to the PC): the real debug APK on an emulator against the REAL desktop build as a test Kin beside the
# user's installed one (own --data-dir, loopback only, no window, --dev-toast-quiet: every phone toast goes to the Action Center with no popup and no sound).
# ASCII only. No OS input injection: a "click" is what the shell does (COM CoCreateInstance of the registered class + Activate, tools toast_probe), the X is the
# same function the Dismissed handler calls (--dev-toast dismiss).
#
#   tools\e2e-2b-2.ps1                              the whole run on kin_api37 (S1 .. S8; S7 last: privacy + registry)
#   tools\e2e-2b-2.ps1 -Only S3,S5                  some of them (setup, pairing and S7 always run)
#   tools\e2e-2b-2.ps1 -Avd kin_api29 -Only S1,S2,S3,S4,S5 -NoClipboard -LaterCheckSeconds 0     the Android 10 pass
#
# S1 capability notif both ways, the master switches (PC, phone), status + sync on reconnect, offline / PC down -> dropped
# S2 allow list, icon cached once, MessagingStyle, the skip rules (own, not allowed, ongoing/FGS, summary, media, call/alarm/progress, secret, importance)
# S3 an update replaces the toast (one toast), a removal on the phone removes it on the PC
# S4 reply / button through the COM class reach the phone's receiver (ack); Copy code (S4c): the real clipboard, sensitive formats, cleared after 60 s
# S5 dismiss from the PC (the X path) cancels on the phone; the phone's setting and the PC's setting off -> no dismiss
# S6 a flagged app (the throwaway "Demo Bank" app) -> app name only; a redacted placeholder -> "Code hidden by Android"; hidden-on-lock-screen allowed -> private
# S8 a flood of 50 from one app: PC responsive, the limits as specified
# S7 privacy: no test text, code, reply or key in the test Kin's data dir / log / DB, nor in the phone's Kin log or app data; the registry of the dev identity clean
#
# It never touches the installed Kin (process, data, AUMID app.kinrelay.desktop, Run key, SendTo, Downloads, firewall), saves and restores the PC clipboard (every
# format), never presses the hotkey, and refuses to run while a game or an anti-cheat is running. Scenario 9 (2B.1's S6) is tools\e2e-2b-1.ps1 -Only S6.
param(
    [string]$Avd = 'kin_api37',
    [string[]]$Only = @(),
    [string]$Kin = (Join-Path $PSScriptRoot '..\desktop\src-tauri\target\release\Kin.exe'),
    [string]$Apk = (Join-Path $PSScriptRoot '..\android\app\build\outputs\apk\debug\app-debug.apk'),
    [int]$Port = 49361,
    [int]$LaterCheckSeconds = 100,
    [switch]$NoClipboard,
    [switch]$KeepRunning,
    [string]$Work = ''
)
. (Join-Path $PSScriptRoot 'kin-lock.ps1'); Enter-KinLock 'e2e-2b-2'
. (Join-Path $PSScriptRoot 'kin-dev.ps1')
. (Join-Path $PSScriptRoot '..\desktop\clipboard-backup.ps1')
if (-not $Work) { $Work = Join-Path ([IO.Path]::GetFullPath($env:LOCALAPPDATA)) ('Temp\kin-e2e-2b-2-' + (Get-Date -Format 'yyyyMMdd-HHmmss')) }
$script:OrigComputerName = $env:COMPUTERNAME
$env:COMPUTERNAME = 'DESK-PC'
$ErrorActionPreference = 'Continue'
foreach ($name in 'ANDROID_HOME', 'JAVA_HOME') {
    if (-not (Get-Item "env:$name" -ErrorAction SilentlyContinue)) { $v = [Environment]::GetEnvironmentVariable($name, 'User'); if ($v) { Set-Item "env:$name" $v } }
}
$cargoBin = Join-Path $env:USERPROFILE '.cargo\bin'
if (-not (($env:Path -split ';') | Where-Object { $_.TrimEnd('\') -ieq $cargoBin })) { $env:Path = "$cargoBin;$env:Path" }
$Repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$Emu = Join-Path $Repo 'tools\emulator'
$Adb = Join-Path $(if ($env:ANDROID_HOME) { $env:ANDROID_HOME } else { Join-Path $env:LOCALAPPDATA 'Android\Sdk' }) 'platform-tools\adb.exe'
$Target = Join-Path $Repo 'desktop\src-tauri\target\debug'
$Probe = Join-Path $Target 'examples\toast_probe.exe'
$Pkg = 'app.kinrelay.android'
$Service = "$Pkg/$Pkg.notifmirror.MirrorListenerService"
$Bank = 'test.bank.demo'
$Serial = 'emulator-5554'
$Data = Join-Path $Work 'data'
$PhoneLog = Join-Path $Work 'phone-kin-log.txt'
$SystemHits = Join-Path $Work 'phone-system-log-hits.txt'
$script:Failures = New-Object System.Collections.Generic.List[string]
$script:Results = [ordered]@{}
$script:Notes = New-Object System.Collections.Generic.List[string]
$started = @{ Emulator = $false; Kin = $false }
$script:Aumid = $null; $script:Clsid = $null; $script:Short = $null; $script:KinProc = $null; $script:Backup = $null; $script:Sdk = 0
# The strings of this run that must never be found in any data file or log. (The debug test notifications' own texts, the shell posts' texts, the reply, the code.) Every one is long enough that it can't turn up by chance in a few hundred KB of binary data (a 2-byte marker did, once).
$TestStrings = @('ZQ first', 'ZQ second', 'ZQ plain', 'ZQ media', 'ZQ offline', 'ZQ flood', 'ZQ switch', 'ZQ phone', 'ZQ one', 'ZQ two', 'ZQ three', 'ZQUpdate', 'ZQPlain', 'ZQSkip', 'ZQBank', 'ZQChat', 'ZQConv', 'ZQReply', 'ZQOtp', 'ZQSwitch', 'ZQPhoneSwitch', 'ZQOffline', 'ZQMedia', 'ZQChild', 'ZQFlood', 'ZQBankPre', 'Are you coming tonight', 'Yes, at eight', 'bring the charger', 'Example Service', '482913', "Don't share it", 'Looks redacted', 'hidden on the lock screen', 'Low importance', 'Group child', 'Ongoing task', '731946', '846201', '517308', 'SAVE20', '483921')

function A { & $Adb -s $Serial @args }
function Say($m) { Write-Host ("[{0:HH:mm:ss}] {1}" -f (Get-Date), $m) }
function Check([string]$name, [bool]$ok, [string]$detail = '') {
    $script:Results[$name] = if ($ok) { 'PASS' } else { 'FAIL' }
    if ($ok) { Say "PASS  $name $detail" } else { Say "FAIL  $name $detail"; $script:Failures.Add($name) }
}
function Note([string]$m) { $script:Notes.Add($m); Say "NOTE  $m" }
function Want([string]$Id) { $ids = @($Only | ForEach-Object { $_ -split ',' } | Where-Object { $_ }); $ids.Count -eq 0 -or $ids -contains $Id }
function Wait-Until([scriptblock]$Cond, [double]$Seconds, [int]$PollMs = 250) {
    $sw = [Diagnostics.Stopwatch]::StartNew()
    while ($sw.Elapsed.TotalSeconds -lt $Seconds) { if (& $Cond) { return $true }; Start-Sleep -Milliseconds $PollMs }
    return $false
}
function Invoke-EmuScript([string]$Script, [string[]]$ScriptArgs = @(), [int]$TimeoutSec = 400) {
    $out = Join-Path $env:TEMP "kin-e2e-2b-2-$Script.out"
    $p = Start-Process powershell -WindowStyle Hidden -PassThru -RedirectStandardOutput $out -RedirectStandardError "$out.err" `
        -ArgumentList (@('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', ('"{0}"' -f (Join-Path $Emu $Script))) + $ScriptArgs)
    if (-not $p.WaitForExit($TimeoutSec * 1000)) { Stop-Process -Id $p.Id -Force; return $false }
    return $true
}
function Read-Shared([string]$Path) {
    try {
        $fs = [IO.File]::Open($Path, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::ReadWrite)
        try { $ms = New-Object IO.MemoryStream; $fs.CopyTo($ms); return [Text.Encoding]::GetEncoding(28591).GetString($ms.ToArray()) } finally { $fs.Dispose() }
    } catch { return '' }
}
function Get-Status { try { Get-Content (Join-Path $Data 'status.json') -Raw -ErrorAction Stop | ConvertFrom-Json } catch { $null } }
function Counts { (Get-Status).mirror.counts }
function Dismiss-Sent { ([regex]::Matches((Kin-Log), 'notif\.dismiss( \S+)? sent to device')).Count }
function Kin-Log { Read-Shared (Join-Path $Data 'logs\kin.log') }
function Sha8([string]$Text) {
    $sha = [Security.Cryptography.SHA256]::Create()
    (($sha.ComputeHash([Text.Encoding]::UTF8.GetBytes($Text)) | ForEach-Object { $_.ToString('x2') }) -join '').Substring(0, 8)
}
function Nid-Of([string]$Key) {
    $sha = [Security.Cryptography.SHA256]::Create()
    ((($sha.ComputeHash([Text.Encoding]::UTF8.GetBytes($Key)) | ForEach-Object { $_.ToString('x2') }) -join '')).Substring(0, 32)
}

# ---------------------------------------------------------------------------------------------------------------------------------- the phone
# The phone's Kin log is saved to a file before every debug command (and the buffer cleared), so nothing rolls over and the privacy grep of S7 reads all of it.
function Flush-PhoneLog {
    try {
        $all = @(A logcat -d -v threadtime)
        $kinLines = @($all | Where-Object { $_ -match ' KinRelay\s*:' -or $_ -match 'KinRelay' })
        if ($kinLines.Count -gt 0) { Add-Content -Path $PhoneLog -Value $kinLines -Encoding UTF8 }
        $hits = @($all | Where-Object { $l = $_; $l -notmatch 'KinRelay' -and $l -notmatch '\sadbd\s*:' -and ($TestStrings | Where-Object { $l.Contains($_) }) })
        if ($hits.Count -gt 0) { Add-Content -Path $SystemHits -Value ($hits | ForEach-Object { ($_ -replace '^\S+\s+\S+\s+\d+\s+\d+\s+\w\s+', '') -replace '(.{0,160}).*', '$1' }) -Encoding UTF8 }
        A logcat -c | Out-Null
    } catch { }
}
function Phone-Cmd([string]$Command, [string]$Extras = '', [int]$Wait = 900) {
    A shell "am broadcast -a $Pkg.debug.$Command -n $Pkg/.debug.DebugCommandReceiver $Extras" | Out-Null
    Start-Sleep -Milliseconds $Wait
}
function Phone-State {
    $n0 = @(A logcat -d -s KinRelay | Where-Object { $_ -match 'CMD MIRROR_STATE' }).Count
    A shell "am broadcast -a $Pkg.debug.MIRROR_STATE -n $Pkg/.debug.DebugCommandReceiver" | Out-Null
    $script:l = $null
    Wait-Until { $all = @(A logcat -d -s KinRelay | Where-Object { $_ -match 'CMD MIRROR_STATE' }); if ($all.Count -gt $n0) { $script:l = $all | Select-Object -Last 1; $true } else { $false } } 6 200 | Out-Null
    return ($script:l -replace '^.*CMD ', 'CMD ')
}
function SVal([string]$State, [string]$Name) { if ($State -match ("\b{0}=(\S+)" -f $Name)) { return $Matches[1] } return $null }
function SInt([string]$State, [string]$Name) { $v = SVal $State $Name; if ($null -ne $v -and $v -match '^\d+$') { return [int]$v } return -1 }
function SDrop([string]$State, [string]$Reason) {
    if ($State -match 'dropped=\{([^}]*)\}') { $m = [regex]::Match($Matches[1], ('\b{0}[=:]\s*(\d+)' -f [regex]::Escape($Reason))); if ($m.Success) { return [int]$m.Groups[1].Value } }
    return 0
}
function Active-Keys {
    $d = (A shell dumpsys notification --noredact | Out-String)
    $i = $d.IndexOf('Notification List:'); if ($i -lt 0) { return @() }
    $m = [regex]::Match($d.Substring($i), '(?m)^\s*(Archive|mArchive|Snoozed|Archived)')
    $part = if ($m.Success) { $d.Substring($i, $m.Index) } else { $d.Substring($i) }
    @([regex]::Matches($part, 'key=(0\|[^|\s]+\|-?\d+\|[^|\s:]*\|\d+)') | ForEach-Object { $_.Groups[1].Value } | Select-Object -Unique)   # API 29 prints "appImportanceLocked=..." right after the key
}
function Key-Like([string]$Pattern) { @(Active-Keys | Where-Object { $_ -match $Pattern }) | Select-Object -First 1 }
function Wait-Key([string]$Pattern, [int]$Seconds = 10) {
    $script:foundKey = $null
    Wait-Until { $script:foundKey = Key-Like $Pattern; [bool]$script:foundKey } $Seconds 400 | Out-Null
    return $script:foundKey
}
function Kin-Post([string]$Kind, [string]$Tag = 't') { Phone-Cmd 'MIRROR_POST' "--es kind $Kind --es tag $Tag" 700 }
function Kin-Cancel([string]$Tag, [int]$Id) { Phone-Cmd 'MIRROR_CANCEL' "--es tag $Tag --ei id $Id" 700 }
function Shell-Post([string]$Tag, [string]$Text, [string]$Title = 'Example', [string]$Style = '') { A shell "cmd notification post -t '$Title' $Style $Tag '$Text'" | Out-Null }
function Bank-Post([string]$Kind, [string]$Title = 'T', [string]$Text = 'x', [string]$Tag = 'd', [int]$Id = 1) {
    A shell "am broadcast -a test.bank.demo.POST -n test.bank.demo/.PostReceiver --es kind $Kind --es title '$Title' --es text '$Text' --es tag $Tag --ei id $Id" | Out-Null
}
function Phone-Online { $s = Phone-State; ($s -match 'online=true') -and ($s -match 'pcAcceptsNotif=true') }
function Net-Tx {
    # transmitted bytes of the emulator's Wi-Fi interface (an indication of what the phone sent; the icon is the one big thing)
    $l = @(A shell cat /proc/net/dev) | Where-Object { $_ -match '^\s*wlan0:' } | Select-Object -First 1
    if ($l -match ':\s*\d+\s+\d+\s+\d+\s+\d+\s+\d+\s+\d+\s+\d+\s+\d+\s+(\d+)') { return [int64]$Matches[1] }
    return -1
}

# ---------------------------------------------------------------------------------------------------------------------------------- the PC
function Invoke-Probe {
    for ($i = 0; $i -lt 5; $i++) { try { return @(& $Probe @args) } catch { Start-Sleep -Milliseconds 500 } }
    return @()
}
function Get-Toasts([switch]$Xml) {
    $a = @('history', $script:Aumid); if ($Xml) { $a += '--xml' }
    $res = New-Object System.Collections.Generic.List[object]; $cur = $null
    foreach ($l in (Invoke-Probe @a)) {
        if ($l -match '^toast tag=(\S+) group=(\S+) bytes=(\d+) buttons=(\d+) input=(\w+) private=(\w+)') {
            $cur = [pscustomobject]@{ Tag = $Matches[1]; Group = $Matches[2]; Bytes = [int]$Matches[3]; Buttons = [int]$Matches[4]; Input = ($Matches[5] -eq 'true'); Private = ($Matches[6] -eq 'true'); Xml = '' }
            $res.Add($cur)
        } elseif ($l -match '^\d+ toast\(s\) for ') { $cur = $null }
        elseif ($cur -and $Xml) { $cur.Xml += $l }
    }
    return $res.ToArray()
}
function Toast-Of([string]$Nid, [switch]$Xml) { Get-Toasts -Xml:$Xml | Where-Object { $_.Tag -eq $Nid } | Select-Object -First 1 }
function Toast-Count { @(Get-Toasts).Count }
function Toast-Args([string]$Xml, [string]$Action) {
    # the arguments of a button of the toast, as Windows holds them (&amp; undone)
    foreach ($m in [regex]::Matches($Xml, 'arguments="([^"]+)"')) { $a = $m.Groups[1].Value.Replace('&amp;', '&'); if ($a -match ("(^|&)a={0}(&|$)" -f $Action)) { return $a } }
    return $null
}
function Activate([string]$Arguments, [string[]]$Inputs = @()) { Invoke-Probe activate $script:Clsid $script:Aumid $Arguments @Inputs }
function Count-Field([string]$Field) { $c = Counts; $v = $c; foreach ($p in ($Field -split '\.')) { $v = $v.$p }; if ($null -eq $v) { return -1 } [int]$v }
function Reg-Exists { @(Invoke-Probe registry $script:Aumid $script:Clsid | Where-Object { $_ -match 'EXISTS' }) }
function Start-TestKin {
    $KinArgs = @('--data-dir', ('"{0}"' -f $Data), '--dev-bind', '127.0.0.1', '--dev-qr-address', '10.0.2.2', '--dev-port', "$Port",
        '--dev-received-dir', ('"{0}"' -f (Join-Path $Data 'received')), '--dev-print-pair-url', '--dev-no-windows', '--dev-toast-quiet')
    $script:KinProc = Start-DevKin $KinArgs
}
function Read-Identity {
    $line = (Kin-Log -split "`n" | Where-Object { $_ -match 'phone notifications are on' } | Select-Object -Last 1)
    if ($line -match 'toasts under (\S+), activator class (\{[0-9A-F-]+\})') { $script:Aumid = $Matches[1]; $script:Clsid = $Matches[2] }
}
function Quiet-Settle([int]$Seconds = 4) { Start-Sleep -Seconds $Seconds }
function Icon-Files { @(Get-ChildItem (Join-Path $Data 'notif-icons') -File -ErrorAction SilentlyContinue).Count }
function Clean-All {
    # CLEAR_ALL cancels through the listener (reason LISTENER_CANCEL(_ALL): the phone does not echo those as notif.remove, by design), so the PC's toasts stay:
    # the dev identity's toasts are removed by the probe (test hygiene; Kin's ledger is stale after it, which no later step relies on)
    Phone-Cmd 'MIRROR_CLEAR_ALL' '' 1200
    Invoke-Probe clear $script:Aumid | Out-Null
}
function Rec-Info([string]$KeyPattern) {
    $d = (A shell dumpsys notification --noredact | Out-String)
    $m = [regex]::Match($d, ('key=(0\|{0}[^\s:]*): Notification\([^\n]*?flags=(0x[0-9a-f]+)[^\n]*?category=(\S*)' -f $KeyPattern))
    if ($m.Success) { return "flags=$($m.Groups[2].Value) category=$($m.Groups[3].Value)" }
    $m = [regex]::Match($d, ('key=(0\|{0}[^\s:]*): Notification\([^\n]*?flags=(0x[0-9a-f]+)' -f $KeyPattern))
    if ($m.Success) { return "flags=$($m.Groups[2].Value)" }
    return 'record not found'
}
function Try-Skip([string]$Label, [string[]]$Reasons, [scriptblock]$Post, [string]$KeyPattern = '') {
    $s0 = Phone-State; $c0 = Counts; $tags0 = @(Get-Toasts | ForEach-Object { $_.Tag })
    & $Post
    Start-Sleep -Milliseconds 2500
    $s1 = Phone-State; $c1 = Counts
    $dShown = ([int]$c1.shown + [int]$c1.replaced) - ([int]$c0.shown + [int]$c0.replaced)
    $dReason = 0; foreach ($r in $Reasons) { $dReason += (SDrop $s1 $r) - (SDrop $s0 $r) }
    $others = @('not_allowed', 'own', 'ongoing', 'summary', 'local_only', 'media', 'category', 'importance', 'secret', 'empty') | Where-Object { $Reasons -notcontains $_ -and ((SDrop $s1 $_) - (SDrop $s0 $_)) -gt 0 } | ForEach-Object { "$_ +$((SDrop $s1 $_) - (SDrop $s0 $_))" }
    $ok = ($dShown -eq 0) -and ($dReason -ge 1)
    $detail = "shown +$dShown, $($Reasons -join '/') +$dReason" + $(if ($others) { "; other counters: " + ($others -join ', ') } else { '' })
    if (-not $ok) {
        if ($KeyPattern) { $detail += '; record: ' + (Rec-Info $KeyPattern) }
        $newTags = @(Get-Toasts | ForEach-Object { $_.Tag } | Where-Object { $tags0 -notcontains $_ })
        foreach ($k in Active-Keys) { $n = Nid-Of $k; if ($newTags -contains $n) { $detail += "; new toast for key " + ($k -replace '^0\|', '' -replace '\|\d+$', '') } }
    }
    Check "S2 skip: $Label -> not mirrored, counted as '$($Reasons -join "' or '")'" $ok $detail
}

# ================================================================================================================================== run
try {
    Assert-NoGameRunning
    $Kin = (Resolve-Path $Kin -ErrorAction Stop).Path
    foreach ($f in $Apk, $Probe) { if (-not (Test-Path $f)) { throw "missing: $f" } }
    New-Item -ItemType Directory -Force $Data | Out-Null
    $installedBefore = @(Get-CimInstance Win32_Process -Filter "Name='Kin.exe'" | Where-Object { $_.ExecutablePath -like "$env:ProgramFiles\Kin\*" } | ForEach-Object { $_.ProcessId })
    $KinCopy = Join-Path $Work 'Kin.exe'
    Copy-Item $Kin $KinCopy
    Set-KinDev -Exe $KinCopy -DataDir $Data
    Assert-NoDevKinOnDataDir
    Set-Content -Path (Join-Path $Data 'settings.json') -Value '{"mirrorNotifications":true,"hotkey":"","autoClipboardSync":false,"openLinksAutomatically":false}' -Encoding ASCII

    Say "starting $Avd"
    $started.Emulator = $true
    Invoke-EmuScript 'start.ps1' @('-Avd', $Avd) | Out-Null
    $isBooted = { ((A shell getprop sys.boot_completed 2>$null | Out-String).Trim() -eq '1') }
    if (-not (& $isBooted)) { & $Adb kill-server 2>$null | Out-Null; Start-Sleep -Seconds 2; & $Adb start-server 2>$null | Out-Null; Wait-Until $isBooted 90 | Out-Null }
    if (-not (& $isBooted)) { throw "emulator $Avd did not start" }
    A shell svc wifi enable | Out-Null
    $script:Sdk = [int](A shell getprop ro.build.version.sdk | Out-String).Trim()
    A uninstall $Pkg 2>&1 | Out-Null
    $out = (& $Adb -s $Serial install -r (Resolve-Path $Apk).Path 2>&1 | Out-String)
    Check 'setup: install Kin (debug build)' ($out -match 'Success')
    if ($script:Sdk -ge 37) { A shell pm grant $Pkg android.permission.ACCESS_LOCAL_NETWORK | Out-Null }
    if ($script:Sdk -ge 33) { A shell pm grant $Pkg android.permission.POST_NOTIFICATIONS | Out-Null }
    # the throwaway "Demo Bank" app (test.bank.demo): posts the kinds the shell cannot; its name has the whole word "bank", so Kin flags it
    $bankDir = Join-Path $Work 'bankdemo'
    & (Join-Path $PSScriptRoot 'e2e-2b-2\bankdemo\build.ps1') -Out $bankDir | Out-Null
    A uninstall $Bank 2>&1 | Out-Null
    $out = (& $Adb -s $Serial install -r -g (Join-Path $bankDir 'bankdemo.apk') 2>&1 | Out-String)
    Check 'setup: install the throwaway Demo Bank test app' ($out -match 'Success') ($out.Trim())
    A shell am start -n "$Pkg/.MainActivity" | Out-Null

    $started.Kin = $true
    Start-TestKin
    Check 'setup: the test Kin started' (Wait-Until { (Get-Status).state -eq 'not_paired' } 30)
    Wait-Until { (Kin-Log) -match 'activator class' } 30 | Out-Null
    Read-Identity
    Check 'setup: the identity is the dev one, never the installed Kin''s (app.kinrelay.desktop.phone)' ($script:Aumid -like 'app.kinrelay.desktop.dev-*.phone' -and $script:Aumid -ne 'app.kinrelay.desktop.phone') $script:Aumid
    Invoke-Kin --dev-start-pairing | Out-Null
    $urlFile = Join-Path $Data 'pair-url.txt'
    Wait-Until { Test-Path $urlFile } 10 | Out-Null
    $url = (Get-Content $urlFile -Raw).Trim()
    $paired = $false
    for ($try = 1; $try -le 3 -and -not $paired; $try++) {
        A shell am start -a android.intent.action.VIEW -p $Pkg -d "'$url'" --ez app.kinrelay.android.AUTO_CONFIRM true 2>$null | Out-Null
        $paired = Wait-Until { (Get-Status).state -eq 'phone_connected' } 25
    }
    Check 'setup: paired and connected to the real test Kin' $paired
    if (-not $paired) { throw 'not paired' }
    A shell input keyevent KEYCODE_HOME | Out-Null
    A logcat -G 16M | Out-Null
    Start-Sleep -Seconds 2
    A shell cmd notification allow_listener $Service | Out-Null
    Phone-Cmd 'MIRROR_ALLOW' "--es pkg 'com.android.shell,$Pkg'"
    Phone-Cmd 'MIRROR_ENABLE'
    Wait-Until { (Phone-State) -match 'listener=true' } 20 500 | Out-Null
    Start-Sleep -Seconds 3
    Phone-Cmd 'MIRROR_CLEAR_ALL'
    Say ('the test Kin status: ' + ((Get-Status).mirror | ConvertTo-Json -Compress -Depth 6))

    # ============================================================================================================================ S1
    if (Want 'S1') {
        Say '--- S1 capability, switches, status/sync, offline'
        Flush-PhoneLog
        $st = Phone-State
        Check 'S1 the phone lists notif and sees the PC list notif (PC switch on)' (($st -match 'pcAcceptsNotif=true') -and ($st -match 'online=true') -and ($st -match 'access=') ) $st
        Check 'S1 the PC keys exist while the switch is on (CLSID, AppUserModelId)' ((Reg-Exists).Count -ge 2) ((Reg-Exists) -join ' | ')
        Kin-Post 'a' 'sw1'
        $keyS = Wait-Key '^0\|app\.kinrelay\.android\|1\|sw1\|' 8
        $nidS = if ($keyS) { Nid-Of $keyS } else { 'x' }
        Check 'S1 (on) a notification is mirrored' (Wait-Until { [bool](Toast-Of $nidS) } 10)
        $script:Short = ([regex]::Match((Toast-Of $nidS).Group, 'kin-([A-Z2-7]{8})')).Groups[1].Value

        # --- the PC switch off: a new hello without notif; nothing is mirrored; the keys and toasts go
        $logBefore = ([regex]::Matches((Kin-Log), 'status of device')).Count
        Invoke-Kin --dev-set mirrorNotifications false | Out-Null
        Check 'S1 PC switch off: the phone sees the PC without notif (a new hello on the open connection)' (Wait-Until { (Phone-State) -match 'pcAcceptsNotif=false' } 15 500)
        Check 'S1 PC switch off: its toasts are gone from Windows' (Wait-Until { (Toast-Count) -eq 0 } 10)
        Check 'S1 PC switch off: its registry keys are removed' (Wait-Until { (Reg-Exists).Count -eq 0 } 10) ((Reg-Exists) -join ' | ')
        $c0 = Counts; $sent0 = SInt (Phone-State) 'sent'
        Kin-Post 'b' 'sw2'; Shell-Post 'zqsw' 'ZQ switch text' 'ZQSwitch'
        Quiet-Settle 5
        $c1 = Counts
        Check 'S1 PC switch off: a new notification is not mirrored (nothing shown, nothing on Windows, nothing sent by the phone)' (([int]$c1.shown -eq [int]$c0.shown) -and ((Toast-Count) -eq 0) -and ((SInt (Phone-State) 'sent') -eq $sent0)) ("shown $($c0.shown)->$($c1.shown) sent $sent0->$(SInt (Phone-State) 'sent')")
        Phone-Cmd 'MIRROR_CLEAR_ALL'
        Invoke-Kin --dev-set mirrorNotifications true | Out-Null
        Check 'S1 PC switch on again: the phone sees notif again' (Wait-Until { (Phone-State) -match 'pcAcceptsNotif=true' } 20 500)
        Check 'S1 PC switch on again: the keys are back' (Wait-Until { (Reg-Exists).Count -ge 2 } 15)
        Kin-Post 'b' 'sw3'
        Check 'S1 PC switch on again: mirroring works again' (Wait-Until { (Toast-Count) -ge 1 } 10)

        # --- the phone's master switch
        $keyB = Wait-Key '^0\|app\.kinrelay\.android\|2\|sw3\|' 6
        $nidB3 = if ($keyB) { Nid-Of $keyB } else { 'x' }
        $r0 = Count-Field 'removed'
        Phone-Cmd 'MIRROR_ENABLE' '--ez on false' 1500
        Check 'S1 phone switch off: the active mirrored notification is removed on the PC (notif.remove) and the toast is gone' ((Wait-Until { -not (Toast-Of $nidB3) } 10) -and (Count-Field 'removed') -gt $r0)
        $c0 = Counts
        Kin-Post 'a' 'sw4'; Shell-Post 'zqph' 'ZQ phone switch text' 'ZQPhoneSwitch'
        Quiet-Settle 5
        Check 'S1 phone switch off: nothing is mirrored' ((([int](Counts).shown) -eq [int]$c0.shown) -and (Toast-Count) -eq 0) ((Counts | ConvertTo-Json -Compress))
        Phone-Cmd 'MIRROR_CLEAR_ALL'
        Phone-Cmd 'MIRROR_ENABLE' '--ez on true' 1500
        $c0 = Counts
        Kin-Post 'b' 'sw5'
        Check 'S1 phone switch on again: mirroring works again' (Wait-Until { [int](Counts).shown -gt [int]$c0.shown } 10)
        Phone-Cmd 'MIRROR_CLEAR_ALL'; Quiet-Settle 2

        # --- offline: Wi-Fi off. A toast that is on the PC when the link drops, removed on the phone meanwhile, is removed by notif.sync on reconnect;
        #     what was posted while offline is dropped (never delivered later)
        Kin-Post 'b' 'off1'
        $keyO = Wait-Key '^0\|app\.kinrelay\.android\|2\|off1\|' 6
        $nidO1 = if ($keyO) { Nid-Of $keyO } else { 'x' }
        Check 'S1 (before offline) the toast of off1 is on the PC' (Wait-Until { [bool](Toast-Of $nidO1) } 10)
        $statusLines0 = ([regex]::Matches((Kin-Log), 'status of device')).Count
        $sw0 = Count-Field 'swept'
        A shell svc wifi disable | Out-Null
        Check 'S1 Wi-Fi off: the phone is offline' (Wait-Until { (Phone-State) -match 'online=false' } 30 1000)
        Kin-Cancel 'off1' 2
        Kin-Post 'a' 'off2'; Kin-Post 'b' 'off3'
        Shell-Post 'zqoff' 'ZQ offline text' 'ZQOffline'
        Start-Sleep -Seconds 3
        $stOff = Phone-State
        Say "phone while offline: $stOff"
        $c0 = Counts
        A shell svc wifi enable | Out-Null
        Check 'S1 Wi-Fi on: the phone is online again and the PC lists notif' (Wait-Until { Phone-Online } 60 1500)
        Check 'S1 reconnect: the PC got notif.status again (status line in its log)' (Wait-Until { ([regex]::Matches((Kin-Log), 'status of device')).Count -gt $statusLines0 } 20)
        Check 'S1 reconnect: notif.sync removed the toast that was removed on the phone while offline (swept +1, toast gone)' ((Wait-Until { -not (Toast-Of $nidO1) } 20) -and (Count-Field 'swept') -gt $sw0) ("swept $sw0 -> $(Count-Field 'swept')")
        Quiet-Settle 6
        $c1 = Counts
        Check 'S1 what was posted while offline was dropped: nothing shown after the reconnect (sync carries no content)' (([int]$c1.shown -eq [int]$c0.shown) -and (Toast-Count) -eq 0) ("shown $($c0.shown)->$($c1.shown), toasts on Windows $(Toast-Count)")
        Say ('phone drop counters now: ' + (Phone-State))

        # --- the PC down: restart the test Kin; notifications posted meanwhile never arrive; the phone reconnects and syncs
        Kin-Post 'a' 'dn1'
        Check 'S1 (before the PC stops) dn1 is on the PC' (Wait-Until { (Toast-Count) -ge 1 } 10)
        Stop-DevKin 15
        Check 'S1 PC stopped: the toasts and the keys are gone' ((Toast-Count) -eq 0 -and (Reg-Exists).Count -eq 0)
        Check 'S1 PC stopped: the phone sees it offline' (Wait-Until { (Phone-State) -match 'online=false' } 40 1500)
        Kin-Post 'b' 'dn2'; Kin-Cancel 'dn2' 2; Kin-Post 'a' 'dn3'
        Start-TestKin
        Check 'S1 PC restarted: the phone reconnects with notif' (Wait-Until { Phone-Online } 90 2000)
        Read-Identity
        Quiet-Settle 8
        Check 'S1 PC restarted: nothing posted while it was down is shown (a stale toast is worse than none)' ((Toast-Count) -eq 0) ("toasts: $(Toast-Count)")
        Check 'S1 PC restarted: the keys are written again, with the same dev identity' ((Reg-Exists).Count -ge 2)
        Phone-Cmd 'MIRROR_CLEAR_ALL'; Quiet-Settle 2
    }

    # ============================================================================================================================ S2
    $nidA = $null
    if (Want 'S2') {
        Say '--- S2 allow list, icon, MessagingStyle, skip rules'
        Flush-PhoneLog
        $st0 = Phone-State
        $c0 = Counts; $tx0 = Net-Tx
        Kin-Post 'a' 'a1'
        $keyA = Wait-Key '^0\|app\.kinrelay\.android\|1\|a1\|' 8
        $nidA = if ($keyA) { Nid-Of $keyA } else { 'x' }
        Check 'S2 an allowed app is mirrored (the conversation toast)' (Wait-Until { [bool](Toast-Of $nidA) } 10)
        Start-Sleep -Seconds 2
        $tx1 = Net-Tx
        $t = Toast-Of $nidA -Xml
        if (-not $script:Short) { $script:Short = ([regex]::Match($t.Group, 'kin-([A-Z2-7]{8})')).Groups[1].Value }
        Check 'S2 the icon reached the PC and is cached (a file named by its hash; written once, by S1 or now)' ((Icon-Files) -ge 1) ("icon files $(Icon-Files)")
        Check 'S2 the toast has the reply box, Send and Mark as read: 2 buttons ("Open" is an activity action: not offered on API 31+)' ($t.Input -and (($script:Sdk -lt 31 -and $t.Buttons -eq 3) -or ($script:Sdk -ge 31 -and $t.Buttons -eq 2))) "buttons=$($t.Buttons) input=$($t.Input) sdk=$($script:Sdk)"
        Check 'S2 the toast shows the app icon (a file of the cache), the last two messages (not the first), and the app and phone name' (($t.Xml -match 'notif-icons') -and ($t.Xml -match 'bring the charger') -and ($t.Xml -match 'Yes, at eight') -and -not ($t.Xml -match 'Are you coming tonight')) ($t.Xml.Length)
        $ic1 = Icon-Files
        Kin-Post 'a' 'a2'
        $keyA2 = Wait-Key '^0\|app\.kinrelay\.android\|1\|a2\|' 8
        $nidA2 = if ($keyA2) { Nid-Of $keyA2 } else { 'x' }
        Check 'S2 a second post from the same app is shown' (Wait-Until { [bool](Toast-Of $nidA2) } 10)
        Start-Sleep -Seconds 2
        $tx2 = Net-Tx
        Say ("phone Wi-Fi bytes sent: first post (with the icon) $($tx1 - $tx0), second post (no icon) $($tx2 - $tx1)")
        Check 'S2 the icon is sent once: the second post of the same app costs far fewer bytes than the first (icon + post)' ((($tx1 - $tx0) - ($tx2 - $tx1)) -gt 2000) ("first $($tx1 - $tx0) B, second $($tx2 - $tx1) B (the whole emulator's Wi-Fi, so only the gap counts)")
        Check 'S2 and no second icon file was stored for it' ((Icon-Files) -eq $ic1) ("icon files $ic1 -> $(Icon-Files)")
        # a shell post: another app, another icon; a MessagingStyle with the shell's text
        Shell-Post 'zqmsg' 'ZQ plain text' 'ZQPlain'
        $keyP = Wait-Key '^0\|com\.android\.shell\|\d+\|zqmsg\|' 8
        $nidP = if ($keyP) { Nid-Of $keyP } else { 'x' }
        Check 'S2 a post of com.android.shell (allowed) is shown with its own icon (a second icon file)' ((Wait-Until { [bool](Toast-Of $nidP) } 10) -and (Icon-Files) -eq ($ic1 + 1)) ("icon files $ic1 -> $(Icon-Files)")

        # not allowed: the Demo Bank app, before it is allowed
        $stN = Phone-State; $cN = Counts
        Bank-Post 'plain' 'ZQBankPre' 'ZQBankPreText'
        Start-Sleep -Seconds 4
        $stN2 = Phone-State
        Check 'S2 a not-allowed app is not mirrored (counted as not_allowed on the phone, nothing on the PC)' (((SDrop $stN2 'not_allowed') -gt (SDrop $stN 'not_allowed')) -and ([int](Counts).shown -eq [int]$cN.shown)) ("not_allowed $(SDrop $stN 'not_allowed') -> $(SDrop $stN2 'not_allowed')")

        # the skip rules, one kind at a time (the Demo Bank app is allowed from here; it is flagged, so a post that passed would be private, S6)
        Phone-Cmd 'MIRROR_ALLOW' "--es pkg $Bank"
        Try-Skip 'call category (Demo Bank)' @('category') { Bank-Post 'call' 'ZQSkip' 'ZQSkipCall' 'sk' 1 } 'test\.bank\.demo\|1\|sk\|'
        Try-Skip 'alarm category (Demo Bank)' @('category') { Bank-Post 'alarm' 'ZQSkip' 'ZQSkipAlarm' 'sk' 2 } 'test\.bank\.demo\|2\|sk\|'
        Try-Skip 'determinate progress bar (Demo Bank)' @('category') { Bank-Post 'progress' 'ZQSkip' 'ZQSkipProgress' 'sk' 3 } 'test\.bank\.demo\|3\|sk\|'
        if ($script:Sdk -ge 34) { Try-Skip 'a real foreground service notification (Demo Bank FgsService, am start-foreground-service)' @('ongoing') { A shell am start-foreground-service -n test.bank.demo/.FgsService | Out-Null; Wait-Key 'test\.bank\.demo\|77\|' 12 | Out-Null } 'test\.bank\.demo\|77\|' } else { Note 'the real foreground-service check needs API 34+ (shortService type): not run here' }
        Try-Skip 'ongoing (Demo Bank)' @('ongoing') { Bank-Post 'ongoing' 'ZQSkip' 'ZQSkipOngoing' 'sk' 5 } 'test\.bank\.demo\|5\|sk\|'
        Try-Skip 'ongoing (Kin''s debug test c)' @('ongoing') { Kin-Post 'c' 'sk' } 'app\.kinrelay\.android\|3\|sk\|'
        Bank-Post 'child' 'ZQChild' 'ZQChildText' 'sk' 10; Start-Sleep -Seconds 2   # a summary with no child loses its summary flag on Android (the system strips it); a real one has children
        Try-Skip 'group summary (Demo Bank, with a child in its group)' @('summary') { Bank-Post 'summary' 'ZQSkip' 'ZQSkipSummary' 'sk' 6 } 'test\.bank\.demo\|6\|sk\|'
        Try-Skip 'MediaStyle (Demo Bank)' @('media', 'ongoing') { Bank-Post 'media' 'ZQSkip' 'ZQSkipMedia' 'sk' 7 } 'test\.bank\.demo\|7\|sk\|'
        Try-Skip 'transport category (Demo Bank)' @('media') { Bank-Post 'transport' 'ZQSkip' 'ZQSkipTransport' 'sk' 8 } 'test\.bank\.demo\|8\|sk\|'
        Try-Skip 'MediaStyle (shell: cmd notification post -S media)' @('media', 'ongoing') { Shell-Post 'zqmedia' 'ZQ media text' 'ZQMedia' '-S media' } 'com\.android\.shell\|\d+\|zqmedia\|'
        Try-Skip 'VISIBILITY_SECRET, not allowed for the app (Demo Bank)' @('secret') { Bank-Post 'secret' 'ZQSkip' 'ZQSkipSecret' 'sk' 9 } 'test\.bank\.demo\|9\|sk\|'
        Try-Skip 'VISIBILITY_SECRET, not allowed for the app (Kin''s debug test e)' @('secret') { Kin-Post 'e' 'sk' } 'app\.kinrelay\.android\|7\|sk\|'
        Try-Skip 'Kin''s own notification (a ping from the PC, non-debug channel)' @('own') { Invoke-Kin --ping | Out-Null }
        # group children go, the low-importance one goes silently
        $cG = Counts
        Kin-Post 'd' 'gr'; Kin-Post 'f' 'lo'
        Check 'S2 group children are mirrored, the summary is not (2 children + the low-importance one = 3 new toasts)' (Wait-Until { ([int](Counts).shown) -ge ([int]$cG.shown + 3) } 12) ("shown $($cG.shown) -> $((Counts).shown)")
        Start-Sleep -Seconds 2
        Check 'S2 and no more than that: exactly 3 new toasts' ((([int](Counts).shown + [int](Counts).replaced) - ([int]$cG.shown + [int]$cG.replaced)) -eq 3) ("shown+replaced $($cG.shown)+$($cG.replaced) -> $((Counts).shown)+$((Counts).replaced)")
        A shell am stopservice -n test.bank.demo/.FgsService | Out-Null
        Clean-All; Quiet-Settle 3
    }

    # ============================================================================================================================ S3
    if (Want 'S3') {
        Say '--- S3 update replaces, removal removes'
        Flush-PhoneLog
        Phone-Cmd 'MIRROR_ALLOW' "--es pkg $Bank"
        Shell-Post 'zqupd' 'ZQ first text' 'ZQUpdate'
        $keyU = Wait-Key '^0\|com\.android\.shell\|\d+\|zqupd\|' 8
        $nidU = if ($keyU) { Nid-Of $keyU } else { 'x' }
        Check 'S3 the first post shows a toast' (Wait-Until { [bool](Toast-Of $nidU) } 10)
        $rp0 = Count-Field 'replaced'
        Start-Sleep -Milliseconds 1300
        Shell-Post 'zqupd' 'ZQ second text' 'ZQUpdate'
        Check 'S3 the update replaces it in place (replaced +1)' (Wait-Until { (Count-Field 'replaced') -gt $rp0 } 10)
        Start-Sleep -Seconds 1
        $mine = @(Get-Toasts -Xml | Where-Object { $_.Tag -eq $nidU })
        Check 'S3 still ONE toast for that notification, with the new text and not the old' (($mine.Count -eq 1) -and ($mine[0].Xml -match 'ZQ second text') -and -not ($mine[0].Xml -match 'ZQ first text')) "toasts with that tag: $($mine.Count)"
        # a conversation that grows: the toast follows
        Shell-Post 'zqchat' 'x' 'ZQChat' "-S messaging --conversation ZQConv --message 'Zed:ZQ one' --message 'Zoe:ZQ two'"
        $keyM = Wait-Key '^0\|com\.android\.shell\|\d+\|zqchat\|' 8
        $nidM = if ($keyM) { Nid-Of $keyM } else { 'x' }
        Check 'S3 a MessagingStyle post (shell) is shown' (Wait-Until { [bool](Toast-Of $nidM) } 10)
        Start-Sleep -Milliseconds 1300
        Shell-Post 'zqchat' 'x' 'ZQChat' "-S messaging --conversation ZQConv --message 'Zed:ZQ one' --message 'Zoe:ZQ two' --message 'Zed:ZQ three'"
        Check 'S3 the next message replaces the toast: still one, now with the last two lines' (Wait-Until { $m2 = @(Get-Toasts -Xml | Where-Object { $_.Tag -eq $nidM }); ($m2.Count -eq 1) -and ($m2[0].Xml -match 'ZQ three') -and ($m2[0].Xml -match 'ZQ two') -and -not ($m2[0].Xml -match 'ZQ one') } 12)
        # removal on the phone
        Kin-Post 'b' 'rm'
        $keyR = Wait-Key '^0\|app\.kinrelay\.android\|2\|rm\|' 8
        $nidR = if ($keyR) { Nid-Of $keyR } else { 'x' }
        Check 'S3 (before removal) the code notification has a toast' (Wait-Until { [bool](Toast-Of $nidR) } 10)
        $rm0 = Count-Field 'removed'
        Kin-Cancel 'rm' 2
        Check 'S3 removed on the phone -> the toast is removed on the PC (removed +1, gone from Windows)' ((Wait-Until { -not (Toast-Of $nidR) } 10) -and (Count-Field 'removed') -gt $rm0)
    }

    # ============================================================================================================================ S4
    if (Want 'S4') {
        Say '--- S4 reply and button from the PC (COM activation, no input injection)'
        Flush-PhoneLog
        Kin-Post 'a' 'rp'
        $keyRp = Wait-Key '^0\|app\.kinrelay\.android\|1\|rp\|' 8
        $nidRp = if ($keyRp) { Nid-Of $keyRp } else { 'x' }
        Check 'S4 the conversation toast is on the PC' (Wait-Until { [bool](Toast-Of $nidRp) } 10)
        $t = Toast-Of $nidRp -Xml
        if (-not $script:Short) { $script:Short = ([regex]::Match($t.Group, 'kin-([A-Z2-7]{8})')).Groups[1].Value }
        $argsReply = Toast-Args $t.Xml 'reply'
        $argsRead = $null
        foreach ($m in [regex]::Matches($t.Xml, 'content="([^"]*)"[^>]*arguments="([^"]+)"')) { if ($m.Groups[1].Value -eq 'Mark as read') { $argsRead = $m.Groups[2].Value.Replace('&amp;', '&') } }
        Check 'S4 the toast holds the reply arguments and the Mark as read arguments' ($argsReply -and $argsRead) "$argsReply | $argsRead"
        $replyText = 'ZQReply from the PC 7'
        $s0 = Phone-State; $act0 = Count-Field 'actions.sent'
        $r = Activate $argsReply @("reply=$replyText")
        Check 'S4 reply: the RemoteInput reached the debug receiver of the phone notification (exact text: its SHA-256 prefix matches)' (Wait-Until { $s = Phone-State; ((SInt $s 'replies') -eq ((SInt $s0 'replies') + 1)) -and ((SVal $s 'lastReplySha') -eq (Sha8 $replyText)) } 12 500) ((Phone-State) -replace '.*(replies=\d+ lastReplySha=\S+).*', '$1')
        Check 'S4 reply: the phone answered ack (lastAction=ack) and the PC counted the action as sent' (((SVal (Phone-State) 'lastAction') -eq 'ack') -and (Count-Field 'actions.sent') -eq ($act0 + 1)) ((SVal (Phone-State) 'lastAction'))
        $m0 = SInt (Phone-State) 'marksRead'
        Activate $argsRead | Out-Null
        Check 'S4 button: "Mark as read" ran the app''s receiver (marksRead +1) and was acked' ((Wait-Until { (SInt (Phone-State) 'marksRead') -eq ($m0 + 1) } 10 500) -and ((SVal (Phone-State) 'lastAction') -eq 'ack'))
        $openOffered = $t.Xml -match 'content="Open"'
        if ($script:Sdk -ge 31) {
            Check 'S4 the activity action "Open" is not offered on API 31+ (it would be silently blocked)' (-not $openOffered)
        } else {
            Check 'S4 (API 29/30, spec point 4) "Open" IS offered: the phone cannot tell an activity PendingIntent there' $openOffered
            $argsOpen = $null
            foreach ($m in [regex]::Matches($t.Xml, 'content="([^"]*)"[^>]*arguments="([^"]+)"')) { if ($m.Groups[1].Value -eq 'Open') { $argsOpen = $m.Groups[2].Value.Replace('&amp;', '&') } }
            if ($argsOpen) {
                A shell input keyevent KEYCODE_HOME | Out-Null
                Start-Sleep -Seconds 2
                $front = { ((A shell dumpsys activity activities | Select-String 'mResumedActivity|topResumedActivity' | Select-Object -First 1) -replace '\s+', ' ' -replace 'ActivityRecord\{\w+ u0 ', '') }
                $frontBefore = & $front
                Flush-PhoneLog
                Activate $argsOpen | Out-Null
                Start-Sleep -Seconds 3
                $s = Phone-State
                $blocked = @(A logcat -d | Where-Object { $_ -match 'Background activity launch blocked|BAL_BLOCK|Abort background activity' }).Count
                Note ("API $($script:Sdk) spec point 4: pressing 'Open' (an activity PendingIntent to Kin's own MainActivity) from the PC: phone lastAction=$(SVal $s 'lastAction'); 'launch blocked' log lines: $blocked; resumed activity before: [$frontBefore] after: [$(& $front)]")
            }
        }        # a stale / unknown click is refused on the PC side, nothing reaches the phone
        $act1 = Count-Field 'actions.sent'
        Activate "kin=1&d=$($script:Short)&n=$('0' * 32)&a=btn&i=1" | Out-Null
        Activate 'not our arguments' | Out-Null
        Start-Sleep -Seconds 2
        Check 'S4 clicks that are not ours (unknown nid, nonsense) are refused: nothing sent to the phone' ((Count-Field 'actions.sent') -eq $act1) "refused=$(Count-Field 'actions.refused')"
    }

    # ============================================================================================================================ S4d
    if (Want 'S4d') {
        Say '--- S4d one-time code detection on the phone, seen on the PC (toast buttons only: nothing is copied)'
        Flush-PhoneLog
        $cases = @(
            @{ Tag = 'zqotp1'; Text = 'Your verification code is 731946'; Copy = '731946'; Why = 'one code next to a keyword -> a Copy button' },
            @{ Tag = 'zqotp2'; Text = 'Verification code 846201 or 517308'; Copy = $null; Why = 'two equally near codes -> no code at all (the design''s own negative: the second one is exactly 10 characters farther)' },
            @{ Tag = 'zqotp3'; Text = 'Use code SAVE20 for 20% off'; Copy = $null; Why = 'a promo code -> none' },
            @{ Tag = 'zqotp4'; Text = 'Your order #483921 has shipped'; Copy = $null; Why = 'a number with no keyword -> none' }
        )
        foreach ($c in $cases) { Shell-Post $c.Tag $c.Text 'ZQOtp'; Start-Sleep -Milliseconds 1300 }
        foreach ($c in $cases) {
            $key = Wait-Key ('^0\|com\.android\.shell\|\d+\|{0}\|' -f $c.Tag) 8
            $nid = if ($key) { Nid-Of $key } else { 'x' }
            Wait-Until { [bool](Toast-Of $nid) } 10 | Out-Null
            $tt = Toast-Of $nid -Xml
            $hasCopy = $tt -and ($tt.Xml -match 'content="Copy ')
            if ($c.Copy) { Check "S4d $($c.Why)" ($tt -and $tt.Xml -match ('Copy ' + $c.Copy)) }
            else { Check "S4d $($c.Why)" ($tt -and -not $hasCopy) $(if ($tt) { "buttons=$($tt.Buttons)" } else { 'no toast' }) }
        }
        Clean-All; Quiet-Settle 2
    }

    # ============================================================================================================================ S4c
    $copyAt = $null
    if ((Want 'S4c') -and -not $NoClipboard) {
        Say '--- S4c Copy code (the real clipboard, saved and restored)'
        Flush-PhoneLog
        Kin-Post 'b' 'cp'
        $keyCp = Wait-Key '^0\|app\.kinrelay\.android\|2\|cp\|' 8
        $nidCp = if ($keyCp) { Nid-Of $keyCp } else { 'x' }
        Check 'S4c the code notification is on the PC with the Copy button (reply none, 1 button)' (Wait-Until { $tc = Toast-Of $nidCp -Xml; $tc -and $tc.Buttons -ge 1 -and ($tc.Xml -match 'Copy 482913') } 10)
        $tc = Toast-Of $nidCp -Xml
        $argsCopy = Toast-Args $tc.Xml 'copy'
        $script:Backup = Save-Clipboard
        $before = (Get-ClipboardFormats | ForEach-Object { $_.Name }) -join ','
        Activate $argsCopy | Out-Null
        $copyAt = Get-Date
        $copied = Wait-Until { try { (Get-Clipboard -Raw -ErrorAction SilentlyContinue) -eq '482913' } catch { $false } } 10
        Check 'S4c Copy through the COM class put the code on the PC clipboard' $copied
        $formats = @(Get-ClipboardFormats | ForEach-Object { $_.Name })
        Check 'S4c with the exclusion marker (ExcludeClipboardContentFromMonitorProcessing: the installed Kin skips it, so it is NOT synced to the real phone), no history, no cloud' (($formats -contains 'ExcludeClipboardContentFromMonitorProcessing') -and ($formats -contains 'CanIncludeInClipboardHistory') -and ($formats -contains 'CanUploadToCloudClipboard')) ($formats -join ',')
        Check 'S4c the code is not in the phone-side log or counted as content (codes_copied counted)' ((Count-Field 'codesCopied') -ge 1)
        # (the 60 s check is at the end of S5, which runs meanwhile)
    }

    # ============================================================================================================================ S5
    if (Want 'S5') {
        Say '--- S5 dismiss sync'
        Flush-PhoneLog
        Kin-Post 'a' 'ds'
        $keyD = Wait-Key '^0\|app\.kinrelay\.android\|1\|ds\|' 8
        $nidD = if ($keyD) { Nid-Of $keyD } else { 'x' }
        Check 'S5 (before) the toast is on the PC and the notification is active on the phone' ((Wait-Until { [bool](Toast-Of $nidD) } 10) -and [bool](Key-Like '\|ds\|'))
        $dm0 = SInt (Phone-State) 'dismissed'; $rm0 = Count-Field 'removed'
        Invoke-Kin --dev-toast dismiss $nidD | Out-Null
        Check 'S5 the X on the PC (UserCanceled path) -> cancelNotification on the phone (dismissed +1, gone from the shade)' ((Wait-Until { (SInt (Phone-State) 'dismissed') -eq ($dm0 + 1) } 12 500) -and (Wait-Until { -not (Key-Like '\|ds\|') } 8 500))
        Start-Sleep -Seconds 4
        Check 'S5 and no echo: the phone does not send a notif.remove back for it (removed unchanged)' ((Count-Field 'removed') -eq $rm0) ("removed $rm0 -> $(Count-Field 'removed')")
        # the phone's setting off
        Phone-Cmd 'MIRROR_DISMISS_PREF' '--ez on false'
        Kin-Post 'a' 'ds2'
        $keyD2 = Wait-Key '^0\|app\.kinrelay\.android\|1\|ds2\|' 8
        $nidD2 = if ($keyD2) { Nid-Of $keyD2 } else { 'x' }
        Wait-Until { [bool](Toast-Of $nidD2) } 10 | Out-Null
        $dm1 = SInt (Phone-State) 'dismissed'; $ds1 = Dismiss-Sent
        Invoke-Kin --dev-toast dismiss $nidD2 | Out-Null
        Start-Sleep -Seconds 4
        Check 'S5 phone setting "Dismiss on the phone too" off: the PC still sends the dismissal' (Wait-Until { (Dismiss-Sent) -gt $ds1 } 12) ("notif.dismiss frames sent (Kin log) $ds1 -> $(Dismiss-Sent)")
        Check 'S5 phone setting off: the phone does not dismiss (dismissed unchanged)' ((SInt (Phone-State) 'dismissed') -eq $dm1) ("dismissed $dm1 -> $(SInt (Phone-State) 'dismissed')")
        Check 'S5 phone setting off: the notification is still in the shade' ([bool](Key-Like '\|ds2\|')) ((Active-Keys | Where-Object { $_ -match 'kinrelay' }) -join ' ; ')
        Phone-Cmd 'MIRROR_DISMISS_PREF' '--ez on true'
        # the PC's setting off
        Invoke-Kin --dev-set mirrorDismissToPhone false | Out-Null
        Kin-Post 'a' 'ds3'
        $keyD3 = Wait-Key '^0\|app\.kinrelay\.android\|1\|ds3\|' 8
        $nidD3 = if ($keyD3) { Nid-Of $keyD3 } else { 'x' }
        Wait-Until { [bool](Toast-Of $nidD3) } 10 | Out-Null
        $dm2 = SInt (Phone-State) 'dismissed'; $ds2 = Dismiss-Sent
        Invoke-Kin --dev-toast dismiss $nidD3 | Out-Null
        Start-Sleep -Seconds 4
        Start-Sleep -Seconds 6   # status.json is rewritten a moment after a change: look again before saying that nothing was sent
        Check 'S5 PC setting "Closing a phone notification here also dismisses it" off: nothing is sent, the phone keeps it' (((Dismiss-Sent) -eq $ds2) -and ((SInt (Phone-State) 'dismissed') -eq $dm2) -and [bool](Key-Like '\|ds3\|')) ("notif.dismiss frames sent (Kin log) $ds2 -> $(Dismiss-Sent)")
        Invoke-Kin --dev-set mirrorDismissToPhone true | Out-Null
        Check 'S5 no notif.remove was echoed back at any point of S5 (removed unchanged since the start)' ((Count-Field 'removed') -eq $rm0) ("removed $rm0 -> $(Count-Field 'removed')")
        Clean-All; Quiet-Settle 2
    }

    # the 60 s clear of S4c (the time passed while S5 ran)
    if ($copyAt) {
        $wait = 63 - ((Get-Date) - $copyAt).TotalSeconds
        if ($wait -gt 0) { Say ("waiting {0:N0} s more for the 60 s clipboard clear" -f $wait); Start-Sleep -Seconds ([int][Math]::Ceiling($wait)) }
        $left = try { Get-Clipboard -Raw -ErrorAction SilentlyContinue } catch { $null }
        Check 'S4c after 60 seconds the code is off the clipboard (unchanged, so cleared)' ($left -ne '482913')
        try { Restore-Clipboard $script:Backup; $script:Backup = $null; Say 'the PC clipboard was restored' } catch { Say "restoring the clipboard failed: $_" }
    }

    # ============================================================================================================================ S6
    if (Want 'S6') {
        Say '--- S6 flagged app, redacted placeholder, hidden-on-lock-screen allowed'
        Flush-PhoneLog
        Phone-Cmd 'MIRROR_ALLOW' "--es pkg $Bank"
        Bank-Post 'plain' 'ZQBankTitle' 'ZQBankText' 'fl' 7
        $keyF = Wait-Key '^0\|test\.bank\.demo\|7\|fl\|' 8
        $nidF = if ($keyF) { Nid-Of $keyF } else { 'x' }
        Check 'S6 a flagged app that was turned on is mirrored' (Wait-Until { [bool](Toast-Of $nidF) } 10)
        $t = Toast-Of $nidF -Xml
        Check 'S6 the PC shows the app name only: the private form, no reply box, no buttons, and neither the title nor the text' ($t.Private -and $t.Buttons -eq 0 -and -not $t.Input -and ($t.Xml -match 'Demo Bank') -and -not ($t.Xml -match 'ZQBankTitle') -and -not ($t.Xml -match 'ZQBankText')) "private=$($t.Private) buttons=$($t.Buttons)"
        Bank-Post 'plain' 'ZQBankCode' 'Your verification code is 482913' 'fl' 8
        $keyF2 = Wait-Key '^0\|test\.bank\.demo\|8\|fl\|' 8
        $nidF2 = if ($keyF2) { Nid-Of $keyF2 } else { 'x' }
        Wait-Until { [bool](Toast-Of $nidF2) } 10 | Out-Null
        $t2 = Toast-Of $nidF2 -Xml
        Check 'S6 an OTP from a flagged app: no Copy button and no code on the PC' ($t2 -and $t2.Private -and $t2.Buttons -eq 0 -and -not ($t2.Xml -match '482913'))
        # hidden on the lock screen, allowed for the app: private too
        Phone-Cmd 'MIRROR_HIDDEN' "--es pkg $Pkg"
        Kin-Post 'e' 'hd'
        $keyH = Wait-Key '^0\|app\.kinrelay\.android\|7\|hd\|' 8
        $nidH = if ($keyH) { Nid-Of $keyH } else { 'x' }
        Wait-Until { [bool](Toast-Of $nidH) } 10 | Out-Null
        $th = Toast-Of $nidH -Xml
        if ($script:Sdk -ge 31) {
            Check 'S6 a notification hidden on the lock screen, allowed for the app, is mirrored as private (no preview)' ($th -and $th.Private -and $th.Buttons -eq 0 -and -not ($th.Xml -match 'hidden on the lock screen'))
        } else { Check 'S6 (API 29) hidden-on-lock-screen allowed: mirrored as private' ($th -and $th.Private -and -not ($th.Xml -match 'hidden on the lock screen')) }
        Phone-Cmd 'MIRROR_HIDDEN' "--es pkg $Pkg --ez on false"
        # the redacted placeholder (Android 15+ puts it in place of a code)
        Kin-Post 'g' 'rd'
        $keyG = Wait-Key '^0\|app\.kinrelay\.android\|9\|rd\|' 8
        $nidG = if ($keyG) { Nid-Of $keyG } else { 'x' }
        Wait-Until { [bool](Toast-Of $nidG) } 10 | Out-Null
        $tg = Toast-Of $nidG -Xml
        if ($script:Sdk -ge 35) {
            Check 'S6 Android''s redacted placeholder -> the PC says "Code hidden by Android" (no reply, no buttons, no code)' ($tg -and ($tg.Xml -match 'Code hidden by Android') -and $tg.Buttons -eq 0) ($tg.Xml.Length)
        } else {
            Check 'S6 (API < 35) the same text is just a text: the matcher is API 35+ only' ($tg -and -not ($tg.Xml -match 'Code hidden by Android'))
        }
        Clean-All; Quiet-Settle 2
    }

    # ============================================================================================================================ S8
    if (Want 'S8') {
        Say '--- S8 flood: 50 posts from one app'
        Flush-PhoneLog
        Say 'waiting 65 s so the phone''s per-package window (20 per 60 s) is empty'
        Start-Sleep -Seconds 65
        Clean-All; Quiet-Settle 3
        $s0 = Phone-State; $c0 = Counts; $logCount0 = ([regex]::Matches((Kin-Log), 'rate_limited')).Count
        $flood = Join-Path $Work 'flood.sh'
        Set-Content -Path $flood -Encoding ASCII -Value "i=1`nwhile [ `$i -le 50 ]; do cmd notification post -t ZQFlood f`$i `"ZQ flood `$i`"; i=`$((i+1)); done`necho flood-done"
        A push $flood /data/local/tmp/flood.sh | Out-Null
        $lat = New-Object System.Collections.Generic.List[double]
        $sw = [Diagnostics.Stopwatch]::StartNew()
        $job = Start-Process $Adb -ArgumentList @('-s', $Serial, 'shell', 'sh', '/data/local/tmp/flood.sh') -PassThru -WindowStyle Hidden -RedirectStandardOutput (Join-Path $Work 'flood-out.txt')
        $endAt = $null
        while ($true) {
            $t0 = [Diagnostics.Stopwatch]::StartNew(); $null = Get-Status; $null = @(Invoke-Probe history $script:Aumid); $lat.Add($t0.Elapsed.TotalMilliseconds)
            if ($job.HasExited -and -not $endAt) { $endAt = (Get-Date).AddSeconds(10); $floodSecs = $sw.Elapsed.TotalSeconds }
            if (($endAt -and (Get-Date) -gt $endAt) -or $sw.Elapsed.TotalSeconds -gt 130) { break }
            Start-Sleep -Milliseconds 250
        }
        $s1 = Phone-State; $c1 = Counts
        $sentD = (SInt $s1 'sent') - (SInt $s0 'sent'); $rateD = (SDrop $s1 'rate') - (SDrop $s0 'rate')
        $shownD = ([int]$c1.shown + [int]$c1.replaced) - ([int]$c0.shown + [int]$c0.replaced)
        Say ("flood: posted 50 in {0:N1} s (the loop on the phone); phone sent $sentD, dropped by its limiter $rateD; PC shown/replaced +$shownD, PC dropped_rate +$([int]$c1.dropped.rate - [int]$c0.dropped.rate), over-cap removed +$([int]$c1.overCapRemoved - [int]$c0.overCapRemoved), toasts on Windows $(Toast-Count); PC probe latency max {1:N0} ms (n=$($lat.Count))" -f $floodSecs, ($lat | Measure-Object -Maximum).Maximum)
        Check 'S8 the phone''s limiter applied: at most 20 of the 50 sent (per package 20 per 60 s), the rest counted as rate drops, sent + dropped accounts for the posts' (($sentD -le 20) -and ($sentD -ge 8) -and ($rateD -ge 25) -and ($sentD + $rateD -ge 45)) "sent $sentD, rate drops $rateD"
        Check 'S8 the PC showed what arrived and never more than its cap of 30 toasts (Windows holds at most that for the identity)' (($shownD -ge 8) -and ((Toast-Count) -le 30)) "shown +$shownD, on Windows $(Toast-Count)"
        Check 'S8 the PC stayed responsive during the flood (status.json + Windows history answered every time, the slowest under 5 s)' (($lat.Count -ge 3) -and (($lat | Measure-Object -Maximum).Maximum -lt 5000)) ("max {0:N0} ms" -f ($lat | Measure-Object -Maximum).Maximum)
        Check 'S8 the test Kin is alive and still connected to the phone' ((@(Get-DevKinProcesses).Count -ge 1) -and ((Get-Status).state -eq 'phone_connected'))
        A shell rm /data/local/tmp/flood.sh | Out-Null
        Clean-All
        Check 'S8 cleanup: the flood toasts are removed from Windows' (Wait-Until { (Toast-Count) -eq 0 } 15) "left: $(Toast-Count)"
    }

    # ============================================================================================================================ S7 (privacy, part 1: while running)
    Say '--- S7 privacy'
        Flush-PhoneLog
    Phone-Cmd 'MIRROR_CLEAR_ALL' '' 1500
    Flush-PhoneLog
    $kinLog = Kin-Log
    $leakLog = @($TestStrings | Where-Object { $kinLog.Contains($_) })
    Check 'S7 the test Kin''s log has no test text, reply or code' ($leakLog.Count -eq 0 -and $kinLog.Length -gt 0) ($leakLog -join ',')
    $phoneKinLog = if (Test-Path $PhoneLog) { Get-Content $PhoneLog -Raw } else { '' }
    $leakPhone = @($TestStrings | Where-Object { $phoneKinLog.Contains($_) })
    Check 'S7 the phone''s Kin log (whole run, every Kin tag line) has no test text, reply or code' ($leakPhone.Count -eq 0 -and $phoneKinLog.Length -gt 0) ("$($leakPhone -join ',') ($($phoneKinLog.Length) chars)")
    $sys = if (Test-Path $SystemHits) { @(Get-Content $SystemHits).Count } else { 0 }
    if ($sys -gt 0) { Note "the phone's system log (other tags than Kin's) holds $sys line(s) with a test string (Android's own services; see $SystemHits)" } else { Say 'the phone''s system log had none either' }
    # the phone's app data (debug build: run-as): one tar of everything, read as bytes
    $tar = Join-Path $Work 'phone-data.tar'
    cmd /c "`"$Adb`" -s $Serial exec-out run-as $Pkg tar cf - . > `"$tar`" 2>nul"
    $tarText = ''
    if (Test-Path $tar) { $tarText = [Text.Encoding]::GetEncoding(28591).GetString([IO.File]::ReadAllBytes($tar)) }
    $leakDb = @($TestStrings | Where-Object { $tarText.Contains($_) })
    Check 'S7 the phone''s app data (databases, DataStore, files, caches: one tar of all of it) has no test text, reply or code' ($tarText.Length -gt 100000 -and $leakDb.Count -eq 0) ("tar $($tarText.Length) bytes; leaks: $($leakDb -join ',')")
    $keyLeaks = @(Active-Keys | ForEach-Object { $_ } | Where-Object { $_ -match 'zq|ZQ' })
    Remove-Item $tar -Force -ErrorAction SilentlyContinue
    $filesWithText = @(Get-ChildItem $Data -Recurse -File -ErrorAction SilentlyContinue | Where-Object { $c = Read-Shared $_.FullName; ($TestStrings | Where-Object { $c.Contains($_) }).Count -gt 0 })
    Check 'S7 no file of the test Kin''s data directory (database, settings, status, log, icon cache, ...) has a test text, reply or code' ($filesWithText.Count -eq 0) (($filesWithText | ForEach-Object { $_.Name }) -join ',')
    $files = @(Get-ChildItem $Data -Recurse -File -ErrorAction SilentlyContinue | Where-Object { $_.FullName -notmatch '\\logs\\' } | ForEach-Object { $_.FullName.Substring($Data.Length + 1) })
    Say ('the test Kin''s data directory holds: ' + ($files -join ', '))
    $nidLeak = @(Get-ChildItem $Data -Recurse -File -ErrorAction SilentlyContinue | Where-Object { (Read-Shared $_.FullName) -match '(?<![0-9a-f])[0-9a-f]{32}(?![0-9a-f])' -and $_.FullName -notmatch '\\notif-icons\\' -and $_.Name -notmatch '^(Kin|kin).*\.(db|sqlite)' } | ForEach-Object { $_.Name })
    Say ('files of the data directory with a 32-hex run (a nid, a hash or an id; the icon files are excluded): ' + ($nidLeak -join ', '))
    $icons = @(Get-ChildItem (Join-Path $Data 'notif-icons') -File -ErrorAction SilentlyContinue)
    Check 'S7 the only notification file on disk is the icon cache, each file named by its hash' (($icons.Count -ge 1) -and (@($icons | Where-Object { $_.Name -notmatch '^[0-9a-f]{32}\.png$' }).Count -eq 0)) ("$($icons.Count) icon file(s)")
}
catch {
    Say "ERROR: $_ ($($_.ScriptStackTrace))"
    $script:Failures.Add("exception: $_")
}
finally {
    if ($script:Backup) { try { Restore-Clipboard $script:Backup; Say 'the PC clipboard was restored' } catch { Say "restoring the clipboard failed: $_" } }
    if ($started.Emulator -and ((A get-state | Out-String) -match 'device')) {
        try { A shell svc wifi enable | Out-Null } catch {}
        try { Phone-Cmd 'MIRROR_CLEAR_ALL' '' 500 } catch {}
        try { A shell cmd notification disallow_listener $Service | Out-Null } catch {}
        try { A uninstall $Bank 2>&1 | Out-Null } catch {}
    }
    if ($started.Kin -and $script:KinProc) { Stop-DevKin 15 }
}
# ---------------------------------------------------------------------------------------------------------------------------------- after quitting
try {
    if ($script:Aumid) {
        Start-Sleep -Seconds 2
        $h = @(Invoke-Probe history $script:Aumid)
        Check 'S7 after quitting: no toast of the dev identity is left on Windows' (($h -match '^0 toast').Count -eq 1) ($h -join ' | ')
        $reg = @(Invoke-Probe registry $script:Aumid $script:Clsid)
        Check 'S7 after quitting: every registry key of the dev identity is gone (CLSID, AppUserModelId, Notifications\Settings, PushNotifications\Backup)' (($reg -match 'EXISTS').Count -eq 0) ($reg -join ' | ')
        cmd /c 'reg query "HKCU\Software\Classes\AppUserModelId\app.kinrelay.desktop.phone" >nul 2>nul'
        Check 'S7 the installed Kin''s phone AUMID (app.kinrelay.desktop.phone) was never written' ($LASTEXITCODE -ne 0)
        if ($LaterCheckSeconds -gt 0) {
            Say "waiting $LaterCheckSeconds s to look at the registry again (Windows may add its PushNotifications\Backup key a while after a toast)"
            Start-Sleep -Seconds $LaterCheckSeconds
            $reg2 = @(Invoke-Probe registry $script:Aumid $script:Clsid)
            if (($reg2 -match 'EXISTS').Count -gt 0) {
                Note ('a key came back: ' + ($reg2 -join ' | ') + ' - removing it (only the dev identity''s)')
                foreach ($k in @("Software\Microsoft\Windows\CurrentVersion\Notifications\Settings\$($script:Aumid)", "Software\Microsoft\Windows\CurrentVersion\PushNotifications\Backup\$($script:Aumid)", "Software\Classes\AppUserModelId\$($script:Aumid)", "Software\Classes\CLSID\$($script:Clsid)")) { cmd /c "reg delete `"HKCU\$k`" /f >nul 2>nul" }
            }
            Check 'S7 a while later no registry key of the dev identity is back' (($reg2 -match 'EXISTS').Count -eq 0) ($reg2 -join ' | ')
        }
    }
    $installedAfter = @(Get-CimInstance Win32_Process -Filter "Name='Kin.exe'" | Where-Object { $_.ExecutablePath -like "$env:ProgramFiles\Kin\*" } | ForEach-Object { $_.ProcessId })
    Check 'the installed Kin was not started, stopped or restarted (same PIDs)' (($installedBefore -join ',') -eq ($installedAfter -join ',')) "$($installedBefore -join ',') -> $($installedAfter -join ',')"
    Check 'no Kin process of this run is left' (@(Get-DevKinProcesses).Count -eq 0)
} catch { Say "ERROR in the after-quit checks: $_"; $script:Failures.Add("exception: $_") }
if (-not $KeepRunning -and $started.Emulator) { Invoke-EmuScript 'stop.ps1' @('-All') 120 | Out-Null }
$env:COMPUTERNAME = $script:OrigComputerName
Remove-DevDataDir $Data
Exit-KinLock
Write-Host ''
$script:Results.GetEnumerator() | ForEach-Object { Write-Host ("{0,-5} {1}" -f $_.Value, $_.Key) }
if ($script:Notes.Count -gt 0) { Write-Host "`nNotes:"; $script:Notes | ForEach-Object { Write-Host " - $_" } }
Say "work folder (logs of the phone): $Work"
if ($script:Failures.Count -gt 0) { Write-Host "`nFAILED: $($script:Failures -join '; ')"; exit 1 }
Write-Host "`nOK: all $($script:Results.Count) checks passed on $Avd."
exit 0

