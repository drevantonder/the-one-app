; The One App's Windows installer, built with NSIS (`makensis`) by `tooling/release` for `one/windows-x86_64`: a
; per-user install, as an app that updates itself is, so it needs no administrator. It puts `one.exe` in
; %LOCALAPPDATA%\Programs, adds that folder to the user's PATH, for `one` in a terminal, puts the app in the Start menu
; and in Settings > Apps, and opens it. From then on the app updates itself, by `one update`, as it does on Linux
; and a Mac.
;
;     makensis -DVERSION=<x.y.z> -DPROGRAM=<one.exe> -DOUTFILE=<installer.exe> clients/app/installer.nsi
!include MUI2.nsh
!include LogicLib.nsh

!define NAME "the One App"
; The app's Tauri identifier, which names its folders and the key Windows lists it by.
!define IDENTIFIER "app.theone.client"
!define UNINSTALL "Software\Microsoft\Windows\CurrentVersion\Uninstall\${IDENTIFIER}"

Name "${NAME}"
OutFile "${OUTFILE}"
Unicode true
ManifestDPIAware true
RequestExecutionLevel user
SetCompressor /SOLID lzma
InstallDir "$LOCALAPPDATA\Programs\${NAME}"
BrandingText "${NAME} ${VERSION}"

VIProductVersion "${VERSION}.0"
VIAddVersionKey ProductName "${NAME}"
VIAddVersionKey CompanyName "${NAME}"
VIAddVersionKey FileDescription "${NAME} installer"
VIAddVersionKey FileVersion "${VERSION}"
VIAddVersionKey ProductVersion "${VERSION}"
VIAddVersionKey LegalCopyright "${COPYRIGHT}"

!define MUI_ICON "../../icons/AppIcon.ico"
!define MUI_UNICON "../../icons/AppIcon.ico"
!define MUI_FINISHPAGE_RUN "$INSTDIR\one.exe"
!define MUI_FINISHPAGE_RUN_TEXT "Open ${NAME}"
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE English

; The task that starts a server this device is as its user signs in (`clients/cli/src/serving.rs`).
!define SERVERS "Get-ScheduledTask -TaskName '${NAME} server *'"

Section
  SetOutPath "$INSTDIR"
  ; The app and a server it keeps running are closed, as an installer closes the app it replaces, and the server is
  ; started again on the new one once it's in place.
  nsExec::Exec `powershell -NoProfile -NonInteractive -Command "${SERVERS} | Stop-ScheduledTask"`
  nsExec::Exec `taskkill /f /t /im one.exe`
  ; One still running, as another user's, can't be written over but can be moved aside, as `one update` does: it
  ; carries on, and the next start is the new one. Each is put aside under a name of its own, as `one update` puts
  ; them (`one.<ms>.old`), and those put aside before that nothing runs any more go.
  Delete "$INSTDIR\one.*.old"
  ${If} ${FileExists} "$INSTDIR\one.exe"
    System::Call 'kernel32::GetTickCount64()l.r0'
    Rename "$INSTDIR\one.exe" "$INSTDIR\one.$0.old"
  ${EndIf}
  File "/oname=one.exe" "${PROGRAM}"
  WriteUninstaller "$INSTDIR\uninstall.exe"

  ; Beside the folder of the app's apps' shortcuts, which the app keeps (`clients/app/src/shortcuts.rs`).
  CreateShortcut "$SMPROGRAMS\${NAME}.lnk" "$INSTDIR\one.exe"

  ; `one` in a terminal, as the user's own: Windows' PATH is the user's after the system's.
  nsExec::Exec `powershell -NoProfile -NonInteractive -Command "$$d = '$INSTDIR'; $$p = [Environment]::GetEnvironmentVariable('Path', 'User'); if (-not (($$p -split ';') -contains $$d)) { [Environment]::SetEnvironmentVariable('Path', (@($$p, $$d) | ? { $$_ }) -join ';', 'User') }"`

  nsExec::Exec `powershell -NoProfile -NonInteractive -Command "${SERVERS} | Start-ScheduledTask"`

  WriteRegStr HKCU "${UNINSTALL}" DisplayName "${NAME}"
  WriteRegStr HKCU "${UNINSTALL}" DisplayIcon "$INSTDIR\one.exe"
  WriteRegStr HKCU "${UNINSTALL}" DisplayVersion "${VERSION}"
  WriteRegStr HKCU "${UNINSTALL}" Publisher "${NAME}"
  WriteRegStr HKCU "${UNINSTALL}" InstallLocation "$INSTDIR"
  WriteRegStr HKCU "${UNINSTALL}" UninstallString `"$INSTDIR\uninstall.exe"`
  WriteRegStr HKCU "${UNINSTALL}" QuietUninstallString `"$INSTDIR\uninstall.exe" /S`
  WriteRegDWORD HKCU "${UNINSTALL}" NoModify 1
  WriteRegDWORD HKCU "${UNINSTALL}" NoRepair 1
SectionEnd

; What's the app's goes, and what's the user's stays: its data, in %LOCALAPPDATA%\app.theone.client, is theirs and
; their network's, as an app's is when it's uninstalled. A server this device is stays one of the network's, off, as
; when it's shut down; it runs again once the app is installed again.
Section Uninstall
  ; The app and a server it keeps running are stopped, and the task that starts the server as its user signs in
  ; (`clients/cli/src/serving.rs`) goes, so nothing runs a program that's gone.
  nsExec::Exec `powershell -NoProfile -NonInteractive -Command "${SERVERS} | % { Stop-ScheduledTask -InputObject $$_; Unregister-ScheduledTask -InputObject $$_ -Confirm:$$false }"`
  nsExec::Exec `taskkill /f /t /im one.exe`
  Delete "$SMPROGRAMS\${NAME}.lnk"
  ; The shortcuts to the network's apps and the `one:` links the app keeps (`clients/app/src/shortcuts.rs`,
  ; `computer.rs`).
  RMDir /r "$SMPROGRAMS\${NAME}"
  RMDir /r "$LOCALAPPDATA\${IDENTIFIER}\shortcuts"
  DeleteRegKey HKCU "Software\Classes\one"
  nsExec::Exec `powershell -NoProfile -NonInteractive -Command "$$d = '$INSTDIR'; $$p = [Environment]::GetEnvironmentVariable('Path', 'User'); [Environment]::SetEnvironmentVariable('Path', (($$p -split ';') | ? { $$_ -and $$_ -ne $$d }) -join ';', 'User')"`
  Delete /REBOOTOK "$INSTDIR\one.exe"
  Delete /REBOOTOK "$INSTDIR\one.*.old"
  Delete "$INSTDIR\uninstall.exe"
  RMDir /REBOOTOK "$INSTDIR"
  DeleteRegKey HKCU "${UNINSTALL}"
SectionEnd
