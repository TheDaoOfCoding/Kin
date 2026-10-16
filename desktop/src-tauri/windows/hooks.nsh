; Kin Relay installer hooks (milestone 1.5). Tauri's NSIS template calls these four macros; tauri.conf.json points at this file
; (bundle.windows.nsis.installerHooks). The installer is per-machine, so it runs ELEVATED: it can be another account than the person at
; the keyboard (a standard user who typed an administrator's password), and then $APPDATA and HKCU are the administrator's.
;
; So this file only does what is machine-wide: the Windows Firewall rules. Everything that belongs to the signed-in user (the "Send to"
; shortcut and "Start with Windows") is made by the app itself the first time it runs, as that user (desktop/src-tauri/src/integration.rs),
; and is removed here on uninstall in BOTH contexts: the current one, and the signed-in user's through nsis_tauri_utils::RunAsUser.
;
; The Start Menu shortcut (with the app user model ID app.kinrelay.desktop, which makes toasts say "Kin" with the app's icon) and the
; removal of the Run value are made by Tauri's own template (CreateOrUpdateStartMenuShortcut and the "Autostart" part of Uninstall).
;
; Uninstall keeps %APPDATA%\Kin (identity, pairing, history) unless the box "Delete the application data" on the uninstall page is ticked.
; It is not ticked by default, and an upgrade never deletes anything (the installer runs the old uninstaller first, and the box is
; unticked; with /UPDATE nothing at all is removed).

!macro NSIS_HOOK_PREINSTALL
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ; The firewall rules. The old ones go first (a reinstall, an upgrade, or an install in another folder), so that there is never a second
  ; rule for an old path. "delete rule" with no match exits 1, which is fine here.
  DetailPrint "Adding the Windows Firewall rules for Kin Relay"
  nsExec::Exec 'netsh advfirewall firewall delete rule name="Kin Relay"'
  Pop $0
  nsExec::Exec 'netsh advfirewall firewall delete rule name="Kin Relay mDNS"'
  Pop $0
  ; The phone connects to the app over TCP. Private and Public both (decision D12: Windows 11 marks a new network Public by default, and every
  ; request is authenticated). Since v0.1.1 only from the local subnet (remoteip=LocalSubnet, D12 addendum): the phone is on the same Wi-Fi or
  ; router as the PC, and a rule that answers the whole internet is not needed. If a router puts the phone on another subnet than the PC, edit
  ; or delete this rule's "Remote addresses" in Windows Defender Firewall with Advanced Security.
  nsExec::ExecToLog 'netsh advfirewall firewall add rule name="Kin Relay" dir=in action=allow program="$INSTDIR\${MAINBINARYNAME}.exe" enable=yes profile=private,public protocol=TCP remoteip=LocalSubnet'
  Pop $0
  ${If} $0 != "0"
    DetailPrint "The firewall rule could not be added (netsh said: $0)"
    MessageBox MB_OK|MB_ICONEXCLAMATION "Kin Relay could not add its Windows Firewall rule (netsh said: $0).$\r$\n$\r$\nThe app will install, but a phone may not be able to reach it. Run the installer again as an administrator, or allow Kin in Windows Security > Firewall." /SD IDOK
  ${EndIf}
  ; The phone finds the PC again by its mDNS name when the PC's address changes; those queries come in over UDP port 5353. (Not in the
  ; original plan, which had TCP only: without it the Windows Firewall drops them for this program. See docs/windows.md.)
  nsExec::ExecToLog 'netsh advfirewall firewall add rule name="Kin Relay mDNS" dir=in action=allow program="$INSTDIR\${MAINBINARYNAME}.exe" enable=yes profile=private,public protocol=UDP localport=5353 remoteip=LocalSubnet'
  Pop $0
  ${If} $0 != "0"
    DetailPrint "The mDNS firewall rule could not be added (netsh said: $0)"
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ; **Ask first, remove after** (v0.1.1). Tauri's template asks "Kin is running, close it?" right after this hook, and Cancel there aborts the
  ; uninstall. With the rules and shortcuts removed above that question, a person who cancelled kept an installed Kin with no firewall rule. So
  ; the same check runs here, before anything is removed; if the person cancels, the uninstall aborts with nothing touched (the template's
  ; own check after this hook then finds Kin closed and says nothing). It also runs for /UPDATE, as the template's does.
  !insertmacro CheckIfAppIsRunning "$INSTDIR\${MAINBINARYNAME}.exe" "${PRODUCTNAME}"

  ; /UPDATE is the updater's call: nothing is removed then.
  ${If} $UpdateMode <> 1
    DetailPrint "Removing the Windows Firewall rules for Kin Relay"
    nsExec::Exec 'netsh advfirewall firewall delete rule name="Kin Relay"'
    Pop $0
    nsExec::Exec 'netsh advfirewall firewall delete rule name="Kin Relay mDNS"'
    Pop $0

    ; The "Send to" shortcut, in this process's user context (the same account as the signed-in user in the usual case) ...
    DetailPrint "Removing the Send to shortcut"
    SetShellVarContext current
    Delete "$APPDATA\Microsoft\Windows\SendTo\Kin.lnk"
    SetShellVarContext all
    ; ... and as the signed-in user, in case the installer was started as another account. This runs without elevation, in the user's own
    ; session, and does not wait: it needs nothing of ours. It also removes the Run value, which the template removes for this context only.
    nsis_tauri_utils::RunAsUser "cmd.exe" '/d /c "del /f /q "%APPDATA%\Microsoft\Windows\SendTo\Kin.lnk" & reg delete HKCU\Software\Microsoft\Windows\CurrentVersion\Run /v Kin /f"'
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ; The template's box "Delete the application data" (unticked by default; with /UPDATE it is never used) also means %APPDATA%\Kin:
  ; the template itself removes only %APPDATA%\app.kinrelay.desktop and %LOCALAPPDATA%\app.kinrelay.desktop (the WebView2 cache), and Kin's own
  ; data folder is %APPDATA%\Kin (identity key, pairing, history, settings, logs).
  ${If} $DeleteAppDataCheckboxState = 1
  ${AndIf} $UpdateMode <> 1
    DetailPrint "Deleting the Kin data folder"
    SetShellVarContext current
    RmDir /r "$APPDATA\Kin"
    SetShellVarContext all
    ; The signed-in user's, if that is another account.
    nsis_tauri_utils::RunAsUser "cmd.exe" '/d /c if defined APPDATA rd /s /q "%APPDATA%\Kin"'
  ${EndIf}
!macroend
