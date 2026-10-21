# Saves and restores every format of the real Windows clipboard, for the scripts that have to use it (check-1-3.ps1 and the
# milestone integration scripts). Dot-source it:
#
#   . .\clipboard-backup.ps1
#   $backup = Save-Clipboard          # writes %LOCALAPPDATA%\Temp\kin-clipboard-backup\ (one file per format); see Save-Clipboard for an earlier backup
#   ... use the clipboard ...
#   Restore-Clipboard $backup         # puts every format back, then deletes the backup
#   Get-ClipboardFormats              # what is on it now (id, name, size)
#
# What is saved: every format whose data is a block of global memory (text in all its encodings, `CF_DIB`/`CF_DIBV5`, `PNG`, `CF_HDROP`,
# rich text, HTML, the password-manager markers, ...). The formats that are GDI handles (`CF_BITMAP`, `CF_PALETTE`, the metafiles) can't
# be copied that way, and Windows makes `CF_BITMAP` from `CF_DIB` by itself. A clipboard that holds *only* such handle formats is
# reported and not saved (nothing is then touched by `Restore-Clipboard`). Delayed-rendering formats are rendered by their owner when
# they are read here, which is what any paste would do.
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public static class KinClip {
    [DllImport("user32.dll", SetLastError = true)] static extern bool OpenClipboard(IntPtr hWnd);
    [DllImport("user32.dll", SetLastError = true)] static extern bool CloseClipboard();
    [DllImport("user32.dll", SetLastError = true)] static extern bool EmptyClipboard();
    [DllImport("user32.dll")] static extern uint EnumClipboardFormats(uint format);
    [DllImport("user32.dll", SetLastError = true)] static extern IntPtr GetClipboardData(uint format);
    [DllImport("user32.dll", SetLastError = true)] static extern IntPtr SetClipboardData(uint format, IntPtr hMem);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClipboardFormatName(uint format, StringBuilder name, int max);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern uint RegisterClipboardFormat(string name);
    [DllImport("kernel32.dll")] static extern IntPtr GlobalLock(IntPtr h);
    [DllImport("kernel32.dll")] static extern bool GlobalUnlock(IntPtr h);
    [DllImport("kernel32.dll")] static extern UIntPtr GlobalSize(IntPtr h);
    [DllImport("kernel32.dll")] static extern IntPtr GlobalAlloc(uint flags, UIntPtr bytes);
    [DllImport("kernel32.dll")] static extern IntPtr GlobalFree(IntPtr h);

    // GDI-handle formats, which are not memory blocks.
    static bool IsHandleFormat(uint f) { return f == 2 || f == 3 || f == 9 || f == 14 || (f >= 0x0080 && f <= 0x008E) || (f >= 0x0200 && f <= 0x02FF); }

    static bool Open() {
        for (int i = 0; i < 30; i++) { if (OpenClipboard(IntPtr.Zero)) return true; System.Threading.Thread.Sleep(30); }
        return false;
    }

    public class Entry { public uint Id; public string Name; public byte[] Data; public bool Skipped; }

    static string NameOf(uint f) {
        var names = new Dictionary<uint, string> { {1,"CF_TEXT"},{2,"CF_BITMAP"},{3,"CF_METAFILEPICT"},{4,"CF_SYLK"},{5,"CF_DIF"},{6,"CF_TIFF"},{7,"CF_OEMTEXT"},{8,"CF_DIB"},{9,"CF_PALETTE"},{10,"CF_PENDATA"},{11,"CF_RIFF"},{12,"CF_WAVE"},{13,"CF_UNICODETEXT"},{14,"CF_ENHMETAFILE"},{15,"CF_HDROP"},{16,"CF_LOCALE"},{17,"CF_DIBV5"} };
        string n; if (names.TryGetValue(f, out n)) return n;
        var sb = new StringBuilder(256);
        return GetClipboardFormatName(f, sb, 256) > 0 ? sb.ToString() : ("format " + f);
    }

    public static List<Entry> Read(bool withData) {
        var list = new List<Entry>();
        if (!Open()) throw new Exception("the clipboard is in use by another program");
        try {
            uint f = 0;
            while ((f = EnumClipboardFormats(f)) != 0) {
                var e = new Entry { Id = f, Name = NameOf(f) };
                if (IsHandleFormat(f)) { e.Skipped = true; list.Add(e); continue; }
                IntPtr h = GetClipboardData(f);
                if (h == IntPtr.Zero) { e.Skipped = true; list.Add(e); continue; }
                int size = (int)(ulong)GlobalSize(h);
                if (withData) {
                    IntPtr p = GlobalLock(h);
                    if (p == IntPtr.Zero) { e.Skipped = true; list.Add(e); continue; }
                    e.Data = new byte[size];
                    Marshal.Copy(p, e.Data, 0, size);
                    GlobalUnlock(h);
                } else e.Data = new byte[Math.Max(size, 0)];
                list.Add(e);
            }
        } finally { CloseClipboard(); }
        return list;
    }

    public const string ExcludeFormat = "ExcludeClipboardContentFromMonitorProcessing";
    public const string DevTestFormat = "KinRelayDevTestContent";

    public static void Write(List<Entry> entries) { Write(entries, false); }

    // quiet: also write ExcludeClipboardContentFromMonitorProcessing (if the content does not carry it already). A Kin that is installed on this PC
    // (0.1.0 too) leaves content that has it alone, so the clipboard sync does not send it to a phone, and Windows keeps it out of its history.
    public static void Write(List<Entry> entries, bool quiet) {
        if (!Open()) throw new Exception("the clipboard is in use by another program");
        try {
            EmptyClipboard();
            bool marked = false;
            foreach (var e in entries) {
                if (e.Skipped || e.Data == null) continue;
                if (e.Name == ExcludeFormat) marked = true;
                Put(e.Id >= 0xC000 ? RegisterClipboardFormat(e.Name) : e.Id, e.Data);
            }
            if (quiet && !marked) Put(RegisterClipboardFormat(ExcludeFormat), new byte[] { 1, 0, 0, 0 });
        } finally { CloseClipboard(); }
    }

    static void Put(uint id, byte[] data) {
        IntPtr h = GlobalAlloc(0x0002 /*GMEM_MOVEABLE*/, (UIntPtr)Math.Max(data.Length, 1));
        IntPtr p = GlobalLock(h);
        Marshal.Copy(data, 0, p, data.Length);
        GlobalUnlock(h);
        if (SetClipboardData(id, h) == IntPtr.Zero) GlobalFree(h);
    }

    // Text for a test: CF_UNICODETEXT plus the two marks that a Kin beside the installed one writes itself (clipboard.rs, DEV_TEST_CONTENT).
    // The installed Kin skips it (ExcludeClipboardContentFromMonitorProcessing); a Kin started with its own --data-dir reads it as ordinary text.
    public static void WriteQuietText(string text) {
        if (!Open()) throw new Exception("the clipboard is in use by another program");
        try {
            EmptyClipboard();
            Put(13, Encoding.Unicode.GetBytes(text + "\0"));
            Put(RegisterClipboardFormat(ExcludeFormat), new byte[] { 1, 0, 0, 0 });
            Put(RegisterClipboardFormat(DevTestFormat), new byte[] { 1, 0, 0, 0 });
        } finally { CloseClipboard(); }
    }
}
'@

# THE CLIPBOARD SINK (2B.6). A script that loads tools\kin-dev.ps1 runs its test Kins with --dev-clipboard-sink (Kin never opens the real clipboard), and in that
# mode every function below works on the sink's files and NEVER on the real clipboard: Save-Clipboard / Restore-Clipboard do nothing, Set-QuietClipboardText "copies"
# into the sink's input files, Get-ClipboardFormats lists what Kin's last clipboard write carries (the formats Kin set). Only a run that says Set-KinDev -RealClipboard
# (tools\check-real-clipboard.ps1) or a script that does not load kin-dev.ps1 at all (desktop\check-clipboard-backup.ps1, desktop\run-real-clipboard-test.ps1, which
# are parts of that one real check) gets the real clipboard.
function Test-KinSinkMode { [bool](Get-Command Get-KinClipboardSink -ErrorAction SilentlyContinue) -and -not (Test-KinRealClipboard) }

# Puts text on the real clipboard the way the dev Kin does (see KinClip.WriteQuietText): invisible to an installed Kin and to Windows' history.
# **Use this and never Set-Clipboard in a test**: Set-Clipboard text is ordinary, and an installed Kin with its automatic sync on would send it to the user's phone.
# In sink mode it is a copy into the sink (input.txt + input.formats, then input.seq).
function Set-QuietClipboardText([string]$Text) {
    if (Test-KinSinkMode) { Set-KinSinkInput $Text -Formats @([KinClip]::ExcludeFormat, [KinClip]::DevTestFormat); return }
    [KinClip]::WriteQuietText($Text)
}

function Get-ClipboardFormats {
    if (Test-KinSinkMode) {
        $c = Get-KinSinkCurrent
        if ($c) { @($c.formats) | ForEach-Object { [pscustomobject]@{ Id = 0; Name = $_.name; Bytes = [int]$_.bytes; Skipped = $false } } }
        return
    }
    [KinClip]::Read($false) | ForEach-Object { [pscustomobject]@{ Id = $_.Id; Name = $_.Name; Bytes = $_.Data.Length; Skipped = $_.Skipped } }
}

# An earlier backup older than this is not trusted: whatever left it behind (a crash, a failed restore) is long over, and putting it back would
# replace what the user has copied since with a clipboard from hours or days ago.
$script:ClipboardBackupMaxAge = [TimeSpan]::FromHours(1)

# Writes every format of the clipboard as it is now into $dir (one file per format, manifest.json last) and returns what it saved. With
# -SkipIfEmpty an empty clipboard writes nothing and returns $null. (Nothing here writes to the pipeline except that return value: callers take
# the last output of Save-Clipboard.)
function Write-ClipboardSnapshot([string]$dir, [switch]$SkipIfEmpty) {
    # A locked clipboard must stop the function: with the default preference the failed read would go on with $entries empty and write an EMPTY
    # backup, and a later Restore-Clipboard of it would clear what the user has copied (v0.1.1 review). The preference is local to this function.
    $ErrorActionPreference = 'Stop'
    $entries = [KinClip]::Read($true)
    if ($SkipIfEmpty -and $entries.Count -eq 0) { return $null }
    if ([IO.Directory]::Exists($dir)) { [IO.Directory]::Delete($dir, $true) }
    [IO.Directory]::CreateDirectory($dir) | Out-Null
    $manifest = @()
    $i = 0
    foreach ($e in $entries) {
        $file = $null
        if (-not $e.Skipped) { $file = "f$i.bin"; [IO.File]::WriteAllBytes((Join-Path $dir $file), $e.Data) }
        $manifest += [pscustomobject]@{ Id = $e.Id; Name = $e.Name; File = $file; Skipped = $e.Skipped }
        $i++
    }
    ConvertTo-Json -InputObject @($manifest) -Depth 3 | Set-Content (Join-Path $dir "manifest.json") -Encoding utf8
    $saved = @($manifest | Where-Object { -not $_.Skipped })
    Write-Host ("saved the clipboard: {0} format(s) [{1}]{2}" -f $saved.Count, (($saved | ForEach-Object { $_.Name }) -join ', '), $(if (@($manifest | Where-Object Skipped).Count) { "; not copyable: " + ((@($manifest | Where-Object Skipped) | ForEach-Object { $_.Name }) -join ', ') } else { '' }))
    [pscustomobject]@{ Dir = $dir; Formats = $saved.Count; Empty = ($manifest.Count -eq 0) }
}

function Save-Clipboard {
    # Sink mode: nothing to save, the real clipboard is not used (returns nothing: callers take $null and Restore-Clipboard of $null is a no-op).
    if (Test-KinSinkMode) { Write-Host 'clipboard sink: the real clipboard is not used by this run, so nothing is saved'; return }
    # A locked clipboard must stop the function: with the default preference the failed read would go on with $entries empty and write an EMPTY
    # backup, and a later Restore-Clipboard of it would clear what the user has copied (v0.1.1 review). The preference is local to this function.
    $ErrorActionPreference = 'Stop'
    # (The long form of the temp folder: Remove-Item can't handle the 8.3 short form that $env:TEMP may have.)
    $dir = Join-Path $env:LOCALAPPDATA 'Temp\kin-clipboard-backup'
    # **An earlier backup is never overwritten, and never costs the user what is on the clipboard now** (v0.1.1, review round 1). A backup that is
    # still there is from a run that did not get to put the clipboard back (it crashed, was killed, or its restore failed). The manifest is written
    # last, so a folder without one is a save that was cut short before the clipboard was touched: not a backup, and replaced. With a manifest:
    #  - unreadable: moved aside (kin-clipboard-backup-unreadable-<time>), this run backs up the clipboard normally.
    #  - older than $script:ClipboardBackupMaxAge: it is stale (the clipboard has been the user's again since). It is moved aside
    #    (kin-clipboard-backup-stale-<time>), never restored, and this run backs up the clipboard normally.
    #  - fresh: the clipboard now is most likely what that run left on it, so its backup is kept and this run's Restore-Clipboard puts it back.
    #    But the clipboard as it is right now may hold something the user copied after that run died, so it is FIRST saved, every format, to
    #    kin-clipboard-backup-superseded-<time> next to the backup. That folder is never restored automatically (Restore-Clipboard only takes
    #    the object this function returns); the warning prints its path.
    $manifestFile = Join-Path $dir 'manifest.json'
    if ([IO.File]::Exists($manifestFile)) {
        $stamp = (Get-Date).ToString('yyyyMMdd-HHmmss')
        $savedAt = [IO.File]::GetLastWriteTime($manifestFile)
        $earlier = $null
        try {
            $parsed = Get-Content -LiteralPath $manifestFile -Raw | ConvertFrom-Json
            $earlier = @($parsed | Where-Object { $null -ne $_ })   # (a plain @($parsed) would nest the array in Windows PowerShell 5.1)
        } catch {
            # Not readable: it is no use as a backup, but it is not deleted either.
            $aside = $dir + '-unreadable-' + $stamp
            Write-Warning "the earlier clipboard backup in $dir could not be read ($_): moved to $aside"
            [IO.Directory]::Move($dir, $aside)
        }
        if ($null -ne $earlier) {
            $age = (Get-Date) - $savedAt
            if ($age -ge $script:ClipboardBackupMaxAge) {
                $aside = $dir + '-stale-' + $stamp
                Write-Warning ("an earlier clipboard backup from {0} is {1:N0} minutes old (over {2:N0}): not trusted and never restored, moved to {3}; backing up the clipboard as it is now" -f $savedAt.ToString('yyyy-MM-dd HH:mm'), $age.TotalMinutes, $script:ClipboardBackupMaxAge.TotalMinutes, $aside)
                [IO.Directory]::Move($dir, $aside)
            } else {
                $side = $dir + '-superseded-' + $stamp
                $current = Write-ClipboardSnapshot $side -SkipIfEmpty
                $earlierSaved = @($earlier | Where-Object { -not $_.Skipped })
                $where = if ($current) { "the clipboard as it is now ($($current.Formats) format(s)) was saved to $side and is not restored automatically" } else { 'the clipboard is empty now, so nothing was saved beside it' }
                Write-Warning ("an earlier clipboard backup from {0} is still in {1} (a run that did not finish): keeping it and putting it back at the end of this run; {2}" -f $savedAt.ToString('yyyy-MM-dd HH:mm'), $dir, $where)
                return [pscustomobject]@{ Dir = $dir; Formats = $earlierSaved.Count; Empty = ($earlier.Count -eq 0) }
            }
        }
    }
    Write-ClipboardSnapshot $dir
}

# -NoMark: put the formats back exactly, with nothing added. By default the restored content also gets ExcludeClipboardContentFromMonitorProcessing (one
# invisible extra format, until the next copy): a restore is a clipboard change like any other, and an installed Kin would otherwise send the user's own
# text to the phone a second time.
function Restore-Clipboard($backup, [switch]$NoMark) {
    if ($null -eq $backup) { return $true }
    # Sink mode never writes the real clipboard, not even to put a backup back (a backup can only come from an earlier real-clipboard run).
    if (Test-KinSinkMode) { Write-Host "clipboard sink: the real clipboard is not touched; the backup in $($backup.Dir) is left where it is"; return $true }
    # A restore that cannot finish (a file of the backup is gone, the clipboard stays locked) leaves the clipboard as it is and the backup where
    # it is, and says so. The backup is not kept for ever: a later Save-Clipboard moves one older than an hour aside and never restores it.
    try { Restore-ClipboardFrom $backup $NoMark } catch {
        Write-Host "the clipboard was NOT restored ($_); the backup is in $($backup.Dir) (a later run ignores it once it is over an hour old)"
        $false
    }
}

function Restore-ClipboardFrom($backup, [bool]$NoMark) {
    $manifest = Get-Content -LiteralPath (Join-Path $backup.Dir 'manifest.json') -Raw | ConvertFrom-Json
    if ($backup.Empty) {
        # It was empty: leave it empty.
        Add-Type -AssemblyName System.Windows.Forms
        [Windows.Forms.Clipboard]::Clear()
        [IO.Directory]::Delete($backup.Dir, $true)
        return $true
    }
    $entries = New-Object 'System.Collections.Generic.List[KinClip+Entry]'
    foreach ($m in @($manifest)) {
        $e = New-Object KinClip+Entry
        $e.Id = [uint32]$m.Id; $e.Name = [string]$m.Name; $e.Skipped = [bool]$m.Skipped
        if (-not $e.Skipped) { $e.Data = [IO.File]::ReadAllBytes((Join-Path $backup.Dir $m.File)) }
        $entries.Add($e)
    }
    [KinClip]::Write($entries, (-not $NoMark))
    Start-Sleep -Milliseconds 300
    # Check it: every saved format is back with the same bytes.
    $now = [KinClip]::Read($true)
    $ok = $true
    foreach ($m in @($manifest | Where-Object { -not $_.Skipped })) {
        $have = $now | Where-Object { $_.Name -eq $m.Name } | Select-Object -First 1
        $want = [IO.File]::ReadAllBytes((Join-Path $backup.Dir $m.File))
        if (-not $have -or $have.Data.Length -ne $want.Length) { $ok = $false; Write-Host "  not restored: $($m.Name)" }
    }
    if ($ok) { [IO.Directory]::Delete($backup.Dir, $true); Write-Host 'the clipboard is back as it was' } else { Write-Host "the clipboard was NOT fully restored; the backup is in $($backup.Dir)" }
    $ok
}

# The restore at the end of a long test run, made safe for a PC in daily use (2B.5 integration): the user may have copied something while the run was going, and putting the backup back would
# replace that. The backup is put back only if the clipboard still holds what the RUN can leave there: nothing, what it was at the start ($StartSignature = the text of `Clip-Signature`-style
# "name:bytes,..." taken right after Save-Clipboard), or content that a dev Kin of the run wrote (see Test-KinOwnClipboardContent). Anything else is taken to be the user's own newer copy:
# it is left alone and **the backup folder is KEPT** (its path is printed): a backup that was not put back is never deleted. Returns $true when the backup was put back.

# Pure decision (no clipboard access, so it can be self-tested): is this list of clipboard format names something a dev Kin of the run left there? Every write of a quiet / -RealClipboard
# dev Kin (text, screenshots, the restore of the test text) carries KinRelayDevTestContent (clipboard.rs DEV_TEST_CONTENT, images.rs image_marks(quiet, ..)), so that format alone says
# "ours". A screenshot Kin wrote without it still has the unmistakable shape CF_DIBV5 + PNG + CF_HDROP together with the opt-out marker and no text or HTML.
function Test-KinOwnClipboardContent([string[]]$Names) {
    $n = @($Names)
    if ($n -contains 'KinRelayDevTestContent') { return $true }
    return [bool](($n -contains 'CF_DIBV5') -and ($n -contains 'PNG') -and ($n -contains 'CF_HDROP') -and ($n -contains 'ExcludeClipboardContentFromMonitorProcessing') -and ($n -notcontains 'CF_UNICODETEXT') -and ($n -notcontains 'HTML Format'))
}

# Pure: 'restore' when the backup may be put back (empty clipboard, the start signature, or content of the run), else 'keep'.
function Get-ClipboardRestoreDecision([string[]]$Names, [string]$Signature, [string]$StartSignature) {
    $n = @($Names)
    if ($n.Count -eq 0 -or (Test-KinOwnClipboardContent $n) -or ($Signature -eq $StartSignature)) { return 'restore' }
    return 'keep'
}

function Restore-ClipboardIfNotUsers($backup, [string]$StartSignature) {
    if ($null -eq $backup) { return $true }
    $formats = @(Get-ClipboardFormats)
    $now = @($formats | ForEach-Object { $_.Name })
    $sig = (($formats | ForEach-Object { "$($_.Name):$($_.Bytes)" }) -join ',')
    if ((Get-ClipboardRestoreDecision $now $sig $StartSignature) -eq 'restore') { return (Restore-Clipboard $backup) }
    Write-Host "the clipboard now holds something the user may have copied during the run (not what this run left): NOT replaced by the backup. The backup is KEPT in $($backup.Dir) (formats now: $($now -join ', '))"
    return $false
}
