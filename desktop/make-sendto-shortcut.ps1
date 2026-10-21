# Creates (or removes) the Explorer "Send to" shortcut by hand: the same file (Kin.lnk) that an installed Kin makes for itself at every start
# (src-tauri\src\integration.rs). This is the way to try "Send to" on a development PC, whose Kin never makes it (a developer run, or a run
# from the build folder, doesn't). **Nothing here runs by itself**: the folder `%APPDATA%\Microsoft\Windows\SendTo` is the user's, so the
# default is only used when you ask for it.
#
#   powershell -File make-sendto-shortcut.ps1 -Folder "$env:TEMP\sendto-test"      # a test folder: look at the shortcut, then delete the folder
#   powershell -File make-sendto-shortcut.ps1                                      # the real SendTo folder of the current user
#   powershell -File make-sendto-shortcut.ps1 -Remove                              # takes it out again
#
# The shortcut runs `Kin.exe --send` and Explorer adds the selected files to the end of its command line. If Kin is running it passes the
# files to the running instance (the single-instance rule) and exits, and the files are offered to the phone. On Windows 11 the entry is
# under "Show more options" (the modern menu entry comes in Phase 2).
param(
    [string]$Kin = (Join-Path $PSScriptRoot 'src-tauri\target\release\Kin.exe'),
    [string]$Folder = (Join-Path $env:APPDATA 'Microsoft\Windows\SendTo'),
    [string]$Name = 'Kin.lnk',
    [switch]$Remove
)
$ErrorActionPreference = 'Stop'
$path = Join-Path $Folder $Name
if ($Remove) {
    if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path; "removed $path" } else { "there is no $path" }
    return
}
if (-not (Test-Path -LiteralPath $Kin)) { throw "Kin.exe is not at $Kin (build it first, or pass -Kin)" }
New-Item -ItemType Directory -Force -Path $Folder | Out-Null
$shell = New-Object -ComObject WScript.Shell
$link = $shell.CreateShortcut($path)
$link.TargetPath = (Resolve-Path -LiteralPath $Kin).Path
$link.Arguments = '--send'
$link.WorkingDirectory = Split-Path (Resolve-Path -LiteralPath $Kin).Path
$link.Description = 'Send the selected files to your phone with Kin Relay'
$link.Save()
"created $path -> $($link.TargetPath) $($link.Arguments)"
