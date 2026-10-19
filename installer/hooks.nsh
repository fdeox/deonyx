; Extra steps for the NSIS installer Tauri builds.

!macro NSIS_HOOK_PREUNINSTALL
  ; Close Deonyx first, so its dark layers go away with it.
  nsExec::Exec 'taskkill /F /IM deonyx.exe'
  ; "Start with Windows" without administrator rights lives in the Run key.
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Deonyx"
  ; "Run as administrator" is a scheduled task; removing it needs the rights
  ; it was made with, so Windows asks, but only if the task exists.
  nsExec::Exec 'schtasks /Query /TN Deonyx'
  Pop $0
  ${If} $0 == 0
    ExecShellWait "runas" "schtasks" "/Delete /TN Deonyx /F" SW_HIDE
  ${EndIf}
!macroend
