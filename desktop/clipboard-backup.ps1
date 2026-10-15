# Saves and restores every format of the real Windows clipboard, for the scripts that have to use it (check-1-3.ps1 and the
# milestone integration scripts). Dot-source it:
#
#   . .\clipboard-backup.ps1
#   $backup = Save-Clipboard          # writes %LOCALAPPDATA%\Temp\kin-clipboard-backup\ (one file per format)
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

    public static void Write(List<Entry> entries) {
        if (!Open()) throw new Exception("the clipboard is in use by another program");
        try {
            EmptyClipboard();
            foreach (var e in entries) {
                if (e.Skipped || e.Data == null) continue;
                IntPtr h = GlobalAlloc(0x0002 /*GMEM_MOVEABLE*/, (UIntPtr)Math.Max(e.Data.Length, 1));
                IntPtr p = GlobalLock(h);
                Marshal.Copy(e.Data, 0, p, e.Data.Length);
                GlobalUnlock(h);
                uint id = e.Id >= 0xC000 ? RegisterClipboardFormat(e.Name) : e.Id;
                if (SetClipboardData(id, h) == IntPtr.Zero) GlobalFree(h);
            }
        } finally { CloseClipboard(); }
    }
}
'@

function Get-ClipboardFormats {
    [KinClip]::Read($false) | ForEach-Object { [pscustomobject]@{ Id = $_.Id; Name = $_.Name; Bytes = $_.Data.Length; Skipped = $_.Skipped } }
}

function Save-Clipboard {
    # (The long form of the temp folder: Remove-Item can't handle the 8.3 short form that $env:TEMP may have.)
    $dir = Join-Path $env:LOCALAPPDATA 'Temp\kin-clipboard-backup'
    if ([IO.Directory]::Exists($dir)) { [IO.Directory]::Delete($dir, $true) }
    [IO.Directory]::CreateDirectory($dir) | Out-Null
    $entries = [KinClip]::Read($true)
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

function Restore-Clipboard($backup) {
    if ($null -eq $backup) { return $true }
    $manifest = Get-Content (Join-Path $backup.Dir 'manifest.json') -Raw | ConvertFrom-Json
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
    [KinClip]::Write($entries)
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
