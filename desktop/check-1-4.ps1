# Milestone 1.4 against the real Kin.exe: the queue (a text and files sent while the phone is away arrive, in order, within seconds of the
# reconnect), the queue across a restart of Kin.exe, expiry (--dev-clock-offset), cancel and retry, the tray's "last seen" line, the
# settings (device name, hotkey with a conflict, start with Windows, folder), the debug numbers and "Export logs". A fake phone
# (examples\fake_phone.rs) plays the phone.
#
#   cd desktop; powershell -ExecutionPolicy Bypass -File check-1-4.ps1 [-Build] [-SkipClipboard]
#
# What it does to your machine: starts Kin.exe with `--dev-bind 127.0.0.1` (loopback only: no firewall prompt, no mDNS) and a throwaway
# profile and a throwaway received-files folder under %LOCALAPPDATA%\Temp, so it never writes to the real Downloads or to %APPDATA%\Kin.
# **"Start with Windows" is a stand-in file in a developer run: the registry is never touched** (the script checks that the Run key has no
# Kin value before and after). The real clipboard is saved first, in every format (clipboard-backup.ps1), and put back at the end (also after
# a failure). A second PowerShell holds one hotkey for a moment, to make a conflict. Everything it started is stopped, and its folders are
# deleted. -SkipClipboard leaves out the checks that put text on the clipboard.
param([switch]$Build, [switch]$SkipClipboard)
# Only one of the Kin scripts (e2e, smoke, capture, check) may run at a time: they share the real clipboard, the Kin.exe slot and the emulator (tools\kin-lock.ps1).
. (Join-Path $PSScriptRoot '..\tools\kin-lock.ps1'); Enter-KinLock 'check-1-4'
. (Join-Path $PSScriptRoot '..\tools\kin-dev.ps1')   # a Kin of our own beside the installed one; every command carries --data-dir (kin-dev.ps1)
$ErrorActionPreference = 'Stop'
$here = $PSScriptRoot
. (Join-Path $here 'clipboard-backup.ps1')
$kin = Join-Path $here 'src-tauri\target\release\Kin.exe'
$phoneExe = Join-Path $here 'src-tauri\target\debug\examples\fake_phone.exe'
$work = Join-Path $env:LOCALAPPDATA ('Temp\kin14-check-' + [guid]::NewGuid().ToString('N').Substring(0, 8))
$data = Join-Path $work 'data'; $recv = Join-Path $work 'received'; $save = Join-Path $work 'phone-saved'; $src = Join-Path $work 'source'
$identity = Join-Path $work 'phone-identity.json'
$dash = [string][char]0x2013
$results = New-Object System.Collections.Generic.List[object]
$phones = New-Object System.Collections.Generic.List[object]
$helpers = New-Object System.Collections.Generic.List[object]
$backup = $null

function Check([string]$name, [bool]$ok, [string]$detail = '') {
    $results.Add([pscustomobject]@{ Name = $name; Ok = $ok })
    Write-Host ("  [{0}] {1}{2}" -f $(if ($ok) { 'ok' } else { 'FAIL' }), $name, $(if ($detail -and -not $ok) { "  ($detail)" } else { '' }))
}
function Say([string]$t) { Write-Host "== $t" }
function Wait-Until([scriptblock]$Cond, [int]$Seconds = 20) {
    $sw = [Diagnostics.Stopwatch]::StartNew()
    while ($sw.Elapsed.TotalSeconds -lt $Seconds) { if (& $Cond) { return $true }; Start-Sleep -Milliseconds 100 }
    return $false
}
function Status { Get-Content -LiteralPath (Join-Path $data 'status.json') -Raw -Encoding UTF8 | ConvertFrom-Json }
function Kin-Log { (Get-Content -LiteralPath (Join-Path $data 'logs\kin.log') -Raw -ErrorAction SilentlyContinue) }
function Start-Kin([string[]]$Extra = @()) {
    $a = @('--data-dir', ('"{0}"' -f $data), '--dev-bind', '127.0.0.1', '--dev-received-dir', ('"{0}"' -f $recv), '--dev-print-pair-url', '--dev-no-windows') + $Extra
    Start-DevKin $a | Out-Null
    if (-not (Wait-Until { Test-Path -LiteralPath (Join-Path $data 'status.json') } 15)) { throw 'Kin did not start' }
}
function Stop-Kin { Stop-DevKin 10 }   # --quit to our data directory; a hung process of ours is stopped by its ID
# A fake phone with a key that stays the same between runs (--identity-file). The first one pairs; with -Paired it just connects.
function New-Phone([string[]]$PhoneArgs, [int]$Seconds = 60, [switch]$Paired) {
    Invoke-Kin --dev-start-pairing
    if (-not (Wait-Until { (Test-Path -LiteralPath (Join-Path $data 'pair-url.txt')) } 10)) { throw 'no pairing URL' }
    Start-Sleep -Milliseconds 400
    $url = (Get-Content -LiteralPath (Join-Path $data 'pair-url.txt')).Trim()
    $log = Join-Path $work ("phone{0}.log" -f $phones.Count)
    $a = @($url, '--host', '127.0.0.1', '--seconds', "$Seconds", '--save-dir', ('"{0}"' -f $save), '--identity-file', ('"{0}"' -f $identity)) + $PhoneArgs
    if ($Paired) { $a += '--skip-pairing' }
    $p = Start-Process $phoneExe -ArgumentList $a -PassThru -RedirectStandardOutput $log -NoNewWindow
    $o = [pscustomobject]@{ Process = $p; Log = $log }
    $phones.Add($o)
    $o
}
function Phone-Log($phone) { (Get-Content -LiteralPath $phone.Log -Raw -ErrorAction SilentlyContinue) }
function Wait-Phone($phone, [string]$Pattern, [int]$Seconds = 30) { Wait-Until { (Phone-Log $phone) -match $Pattern } $Seconds }
function Stop-Phones { $phones | ForEach-Object { if (-not $_.Process.HasExited) { $_.Process.Kill() } }; $phones.Clear() }
function Hash([string]$p) { (Get-FileHash -LiteralPath $p -Algorithm SHA256).Hash.ToLower() }
function Make-File([string]$path, [int]$bytes) { $b = New-Object byte[] $bytes; (New-Object Random 42).NextBytes($b); [IO.File]::WriteAllBytes($path, $b) }
# A view that Kin writes for scripts (--dev-dump-view): asks for it and reads it once it is newer than the request.
function View([string]$name) {
    $file = Join-Path $data "view-$name.json"
    Remove-Item -LiteralPath $file -Force -ErrorAction SilentlyContinue
    Invoke-Kin --dev-dump-view $name
    if (-not (Wait-Until { Test-Path -LiteralPath $file } 10)) { throw "no view-$name.json" }
    Start-Sleep -Milliseconds 100
    Get-Content -LiteralPath $file -Raw -Encoding UTF8 | ConvertFrom-Json
}
function Dev-Result([string]$file, [scriptblock]$Action) {
    $path = Join-Path $data $file
    Remove-Item -LiteralPath $path -Force -ErrorAction SilentlyContinue
    & $Action
    if (-not (Wait-Until { Test-Path -LiteralPath $path } 10)) { throw "no $file" }
    Start-Sleep -Milliseconds 100
    Get-Content -LiteralPath $path -Raw -Encoding UTF8 | ConvertFrom-Json
}
function Dev-Set([string]$key, [string]$value) { Dev-Result 'dev-set-result.json' { Invoke-Kin --dev-set $key $value }.GetNewClosure() }
# The text goes to the clipboard and --send-clipboard reads it when Kin gets to the command (a few tens of ms later), so wait until Kin has
# done it before the clipboard is changed again.
function Send-Clip([string]$text) {
    $t0 = (Status).lastSent.text.at
    Set-QuietClipboardText $text; Start-Sleep -Milliseconds 600; Invoke-Kin --send-clipboard
    Wait-Until { (Status).lastSent.text.at -ne $t0 } 10 | Out-Null
}
function Run-KeyText { (@(Run-Key | ForEach-Object { "$($_.Name)=$($_.Value)" }) -join '|') }
function Run-Key { (Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -ErrorAction SilentlyContinue).PSObject.Properties | Where-Object { $_.Name -match 'Kin' } }

try {
    if ($Build) {
        Say 'building'
        Push-Location (Join-Path $here 'src-tauri'); cargo build --example fake_phone | Out-Null; Pop-Location
        powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $here 'build.ps1') | Out-Null
    }
    foreach ($f in $kin, $phoneExe) { if (-not (Test-Path -LiteralPath $f)) { throw "$f is missing: build it first (or pass -Build)" } }
    Set-KinDev -Exe $kin -DataDir $data
    # `cargo test --release` replaces target\release\Kin.exe with a dev-server build; `tauri build` writes the installer just after the exe.
    $installer = Get-ChildItem (Join-Path (Split-Path $kin) 'bundle\nsis') -Filter 'Kin_*_x64-setup.exe' -ErrorAction SilentlyContinue | Sort-Object LastWriteTime | Select-Object -Last 1
    if (-not $installer -or (Get-Item $kin).LastWriteTime -gt $installer.LastWriteTime.AddSeconds(30)) { throw 'Kin.exe is not from `tauri build` (`cargo test --release` replaces it with a dev-server build): pass -Build' }
    New-Item -ItemType Directory -Force -Path $work, $src, $save | Out-Null
    if (-not $SkipClipboard) { $backup = Save-Clipboard }
    Say "work folder $work"
    # The installed Kin has its own value in the Run key (start with Windows). A developer run must leave the key exactly as it is.
    $runBefore = Run-KeyText
    Write-Host ('  the Run key has {0} Kin value(s) before the run (the installed Kin''s own, if it has one: only compared at the end)' -f @(Run-Key).Count)

    # ---------------------------------------------------------------- the queue
    Say 'queue: the phone goes away, the PC sends a text, files and a second text, the phone comes back'
    Start-Kin
    Make-File (Join-Path $src 'photo.jpg') 2000000
    Make-File (Join-Path $src 'notes.txt') 5000
    $q = New-Phone @('--offline-after', '4', '--offline-for', '9') 45
    Check 'the phone connected and paired' (Wait-Until { (Status).state -eq 'phone_connected' } 15)
    Check 'the phone went away' (Wait-Phone $q 'going offline' 15)
    Check 'the PC shows the phone as offline' (Wait-Until { (Status).state -eq 'phone_offline' } 10)
    $tray = (Status).tray.status
    Check 'the tray says "Phone offline - last seen <time>"' ($tray -match ('^Phone offline ' + $dash + ' last seen \d\d:\d\d')) $tray
    if (-not $SkipClipboard) {
        Send-Clip 'queued text one'
        Invoke-Kin --send (Join-Path $src 'photo.jpg') (Join-Path $src 'notes.txt')
        Send-Clip 'queued text two'
        Check 'three sends are waiting in the queue' (Wait-Until { (Status).queue.count -eq 3 } 10) "count $((Status).queue.count)"
        Check 'the tray shows how many are waiting' ((Status).tray.status -match '3 queued') (Status).tray.status
        Check 'the automatic clipboard was dropped, not queued (it said phone_offline)' (((Status).lastSent.clipboard.result -eq 'phone_offline') -and ((Status).queue.count -eq 3)) "$((Status).lastSent.clipboard.result) / $((Status).queue.count)"
        $hv = View 'history'
        Check 'the history lists them as queued, newest first' ((($hv.items | Where-Object { $_.status -eq 'queued' }).Count -eq 3) -and ($hv.items[0].kind -eq 'text'))
        Check 'the history has no automatic clipboard sync' (-not (($hv.items | ConvertTo-Json -Depth 5) -match 'stale|auto'))
        Check 'the phone reconnects and gets all three, in order, and downloads the files' (Wait-Phone $q 'downloaded notes.txt' 40)
        $log = Phone-Log $q
        $after = $log.Substring($log.IndexOf('reconnected at='))
        $arrivals = [regex]::Matches($after, 'arrival type=(\w+) id=(\S+) at=(\d+) ms_since_hello=(\d+)')
        $types = ($arrivals | ForEach-Object { $_.Groups[1].Value }) -join ','
        Check 'the order is text, offer, text (the order they were made in)' ($types -match '^text,offer,text') $types
        $first = [int]$arrivals[0].Groups[4].Value
        $last = [int]$arrivals[$arrivals.Count - 1].Groups[4].Value
        Write-Host ("  queue arrival after the phone's hello: first {0} ms, last {1} ms" -f $first, $last)
        Check 'the first queued item arrives within 2 seconds of the hello (done-when of 1.4)' ($first -lt 2000) "$first ms"
        Check 'the text and files are what was sent' (($log -match 'received text id=\S+ isUrl=\w+ sensitive=\w+ text=queued text one') -and ($log -match 'received text id=\S+ isUrl=\w+ sensitive=\w+ text=queued text two')) $log
        Check 'photo.jpg arrived intact' ((Test-Path -LiteralPath (Join-Path $save 'photo.jpg')) -and ((Hash (Join-Path $save 'photo.jpg')) -eq (Hash (Join-Path $src 'photo.jpg'))))
        Check 'the queue is empty again' (Wait-Until { (Status).queue.count -eq 0 } 10)
        $dv = View 'debug'
        Check 'the debug view counts them as not first-try (the phone was away)' (($dv.successRate.firstTry.settled -ge 3) -and ($dv.successRate.firstTry.firstTryOk -lt $dv.successRate.firstTry.settled)) ($dv.successRate | ConvertTo-Json -Depth 4 -Compress)
        $ev = @($dv.connections | ForEach-Object { $_.event })
        Check 'the connection history has the disconnect and the reconnect' ((@($ev | Where-Object { $_ -eq 'disconnected' }).Count -ge 1) -and (@($ev | Where-Object { $_ -eq 'connected' }).Count -ge 2)) ($ev -join ',')
        $nets = @($dv.network.networks)
        Write-Host ("  network profile: " + $(if ($nets.Count) { ($nets | ForEach-Object { "$($_.name) = $($_.category)" }) -join '; ' } else { '(none reported)' }))
        Check 'the debug view has the network profile (Private or Public) of this PC' ($nets.Count -eq 0 -or ($nets | Where-Object { $_.category -in 'Private', 'Public', 'Domain' }))
    }
    Stop-Phones

    # ---------------------------------------------------------------- restart
    if (-not $SkipClipboard) {
        Say 'restart: a queued text survives a restart of Kin.exe'
        Check 'the phone is offline' (Wait-Until { (Status).state -eq 'phone_offline' } 15)
        Send-Clip 'survives the restart'
        Check 'one send is queued' (Wait-Until { (Status).queue.count -eq 1 } 10)
        Stop-Kin
        Start-Kin
        Check 'after the restart the queue still has it' ((Status).queue.count -eq 1) "count $((Status).queue.count)"
        $r = New-Phone @() 25 -Paired
        Check 'the same phone connects and the text arrives' (Wait-Phone $r 'received text id=\S+ isUrl=False sensitive=False text=survives the restart' 20) (Phone-Log $r)
        Check 'the queue is empty' (Wait-Until { (Status).queue.count -eq 0 } 10)
        Stop-Phones
        Check 'the phone is offline again' (Wait-Until { (Status).state -eq 'phone_offline' } 15)

        # ------------------------------------------------------------ cancel
        Say 'cancel: a queued item that is canceled never goes out'
        Send-Clip 'canceled before it left'
        Check 'it is queued' (Wait-Until { (Status).queue.count -eq 1 } 10)
        $id = (View 'history').items[0].id
        $res = Dev-Result 'dev-history-result.json' { Invoke-Kin --dev-history cancel $id }.GetNewClosure()
        Check 'cancel worked' ($res.ok -eq $true) ($res | ConvertTo-Json -Compress)
        Check 'the queue is empty and the history says canceled' (((Status).queue.count -eq 0) -and ((View 'history').items[0].status -eq 'canceled'))
        $c = New-Phone @() 12 -Paired
        Check 'the phone connects' (Wait-Until { (Status).state -eq 'phone_connected' } 15)
        Start-Sleep -Seconds 3
        Check 'the canceled text never arrived' (-not ((Phone-Log $c) -match 'canceled before it left'))
        Stop-Phones
        Check 'the phone is offline again' (Wait-Until { (Status).state -eq 'phone_offline' } 15)

        # ------------------------------------------------------------ expiry
        Say 'expiry: a queued text that is 7 days old fails with "expired" (the clock of Kin is moved with --dev-clock-offset)'
        Send-Clip 'too old to send'
        Check 'it is queued' (Wait-Until { (Status).queue.count -eq 1 } 10)
        Stop-Kin
        Start-Kin @('--dev-clock-offset', '604801')
        Check 'it expired at start' (Wait-Until { (Status).queue.count -eq 0 } 10)
        $hv = View 'history'
        Check 'the history says failed, with the error "expired"' (($hv.items[0].status -eq 'failed') -and ($hv.items[0].error -eq 'expired')) ($hv.items[0] | ConvertTo-Json -Compress)
        Check 'the queue is empty' ((Status).queue.count -eq 0)
        Stop-Kin
        Start-Kin
    }

    # ---------------------------------------------------------------- settings
    Say 'settings: device name, switches, folder, hotkey with a conflict, start with Windows (a stand-in file)'
    $s = New-Phone @() 40 -Paired
    Check 'the phone is connected' (Wait-Until { (Status).state -eq 'phone_connected' } 15)
    $r = Dev-Set 'deviceName' 'Anna Desk'
    Check 'the device name was changed' (($r.ok -eq $true) -and ($r.message -eq 'Anna Desk'))
    Check 'the connected phone got a new hello with the name' (Wait-Phone $s '"name":"Anna Desk"' 10) (Phone-Log $s)
    Check 'status.json has the name' ((Status).pcName -eq 'Anna Desk')
    Check 'settings.json has the name' ((Get-Content -LiteralPath (Join-Path $data 'settings.json') -Raw | ConvertFrom-Json).deviceName -eq 'Anna Desk')
    [void](Dev-Set 'autoClipboardSync' 'false'); [void](Dev-Set 'imagesToClipboard' 'false')
    $set = Get-Content -LiteralPath (Join-Path $data 'settings.json') -Raw | ConvertFrom-Json
    Check 'the switches are saved' (($set.autoClipboardSync -eq $false) -and ($set.imagesToClipboard -eq $false))
    $folder = Join-Path $work 'my received'
    $r = Dev-Set 'receivedFolder' $folder
    Check 'a received folder is checked, made and kept' (($r.ok -eq $true) -and (Test-Path -LiteralPath $folder) -and ((Get-Content -LiteralPath (Join-Path $data 'settings.json') -Raw | ConvertFrom-Json).receivedFolder -eq $folder))
    $r = Dev-Set 'receivedFolder' 'relative\folder'
    Check 'a folder that is no good is refused' ($r.ok -eq $false) ($r | ConvertTo-Json -Compress)

    $r = Dev-Set 'hotkey' 'Ctrl+Alt+Shift+F11'
    Check 'a new hotkey is registered' (($r.ok -eq $true) -and ((Status).hotkey.accelerator -eq 'Ctrl+Alt+Shift+F11') -and ((Status).hotkey.registered -eq $true))
    # Another program holds Ctrl+Alt+Shift+F12: a second PowerShell registers it and keeps it for a while.
    $holderScript = @'
Add-Type -Namespace K -Name H -MemberDefinition '[System.Runtime.InteropServices.DllImport("user32.dll")] public static extern bool RegisterHotKey(System.IntPtr h, int id, uint mod, uint vk);'
$ok = [K.H]::RegisterHotKey([System.IntPtr]::Zero, 1, 7, 0x7B)
Set-Content -LiteralPath '__FILE__' -Value $ok
Start-Sleep -Seconds 25
'@ -replace '__FILE__', (Join-Path $work 'holder.txt')
    $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($holderScript))
    $holder = Start-Process powershell -ArgumentList '-NoProfile', '-EncodedCommand', $encoded -PassThru -WindowStyle Hidden
    $helpers.Add($holder)
    Check 'the other program holds the key' (Wait-Until { (Test-Path -LiteralPath (Join-Path $work 'holder.txt')) -and (([string](Get-Content -LiteralPath (Join-Path $work 'holder.txt') -Raw)).Trim() -eq 'True') } 15)
    $r = Dev-Set 'hotkey' 'Ctrl+Alt+Shift+F12'
    Check 'a hotkey that another program has is reported as a conflict' (($r.ok -eq $false) -and ($r.message -match 'another program')) ($r | ConvertTo-Json -Compress)
    Check 'the old hotkey stays registered' (((Status).hotkey.accelerator -eq 'Ctrl+Alt+Shift+F11') -and ((Status).hotkey.registered -eq $true))
    # (A function key alone is allowed now, so it is not tried here: it would really take that key from every program for a moment.)
    $r = Dev-Set 'hotkey' 'K'
    Check 'a bare letter is refused, with the reason' (($r.ok -eq $false) -and ($r.message -match 'needs Ctrl, Alt or Win'))  ($r | ConvertTo-Json -Compress)
    $r = Dev-Set 'hotkey' 'Shift+K'
    Check 'Shift and a letter is refused (it is typing)' (($r.ok -eq $false) -and ($r.message -match 'needs Ctrl, Alt or Win')) ($r | ConvertTo-Json -Compress)
    $r = Dev-Set 'hotkey' 'Ctrl+C'
    Check 'a common shortcut (Ctrl+C) is refused, and says what it is' (($r.ok -eq $false) -and ($r.message -match 'Copy') -and ($r.message -match "can't be the hotkey")) ($r | ConvertTo-Json -Compress)
    Check 'the old hotkey is still the one after the refusals' (((Status).hotkey.accelerator -eq 'Ctrl+Alt+Shift+F11') -and ((Status).hotkey.registered -eq $true))

    $r = Dev-Set 'startWithWindows' 'true'
    Check 'start with Windows is switched on in the stand-in file' (($r.ok -eq $true) -and ((Get-Content -LiteralPath (Join-Path $data 'autostart-mock.json') -Raw | ConvertFrom-Json).enabled -eq $true))
    Check 'the registry Run key is as it was (a developer run never touches it)' ((Run-KeyText) -eq $runBefore)
    Check 'the settings view says the switch is not the real one' ((View 'settings').startWithWindowsIsReal -eq $false)
    [void](Dev-Set 'startWithWindows' 'false')
    Stop-Phones

    # ---------------------------------------------------------------- export
    Say 'export logs: a zip with the logs and a redacted status.json'
    $zip = Join-Path $work 'export.zip'
    Invoke-Kin --dev-export-logs $zip
    Check 'the zip was written' (Wait-Until { Test-Path -LiteralPath $zip } 10)
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $z = [IO.Compression.ZipFile]::OpenRead($zip)
    try {
        $names = @($z.Entries | ForEach-Object { $_.FullName })
        Check 'it has the logs, status.json, debug.json and a README' (($names -contains 'status.json') -and ($names -contains 'debug.json') -and ($names -contains 'README.txt') -and ($names -contains 'logs/kin.log')) ($names -join ', ')
        $all = ($z.Entries | ForEach-Object { $rd = New-Object IO.StreamReader($_.Open()); $t = $rd.ReadToEnd(); $rd.Dispose(); $t }) -join "`n"
        Check 'no IP address is left in it' (-not ($all -match '\b\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}\b'))
        Check 'neither the user name nor the phone''s name is in it' ((-not ($all -match [regex]::Escape($env:USERNAME))) -and (-not ($all -match 'Fake Phone')))
        # (1.4 review) the 8.3 short form of the profile folder, any other C:\Users\<name>, and the names of Wi-Fi networks (only the category is exported)
        $short = try { (New-Object -ComObject Scripting.FileSystemObject).GetFolder($env:USERPROFILE).ShortPath } catch { '' }
        $shortOk = (-not $short) -or ($short -ieq $env:USERPROFILE) -or (-not $all.ToLower().Contains($short.ToLower()) -and -not $all.ToLower().Contains($short.ToLower().Replace('\', '\\')))
        Check 'the 8.3 short profile path is not in it' $shortOk "short form: $short"
        $left = [regex]::Matches($all, '(?i)\b[a-z]:\\{1,2}users\\{1,2}(?!<user>)[^\\\s"]+') | ForEach-Object { $_.Value }
        Check 'no C:\Users\<name> path is left in it' (@($left).Count -eq 0) ($left -join '; ')
        $nets = @(try { Get-NetConnectionProfile -ErrorAction Stop | ForEach-Object { $_.Name } } catch { @() }) | Where-Object { $_ -and $_.Length -ge 3 }
        Check 'no Wi-Fi network name is in it' (@($nets | Where-Object { $all.Contains($_) }).Count -eq 0) ("names tested: $($nets.Count)")
        Check 'no .part path (a file name with .part after it) is in it' (-not ($all -match '[^\s"]\.part\b'))
        $st = $z.GetEntry('status.json'); $rd = New-Object IO.StreamReader($st.Open()); $stj = $rd.ReadToEnd() | ConvertFrom-Json; $rd.Dispose()
        Check 'the device ID is cut to 8 characters' ($stj.deviceId.Length -eq 8)
    } finally { $z.Dispose() }
    Check 'the Run key is still as it was at the end' ((Run-KeyText) -eq $runBefore)
}
finally {
    Stop-Phones
    $helpers | ForEach-Object { if (-not $_.HasExited) { $_.Kill() } }
    Stop-Kin
    if ($backup) { Restore-Clipboard $backup }
    Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
}
$failed = @($results | Where-Object { -not $_.Ok })
Write-Host ("`n{0} checks, {1} failed" -f $results.Count, $failed.Count)
if ($failed.Count) { $failed | ForEach-Object { Write-Host "  FAILED: $($_.Name)" }; exit 1 }
