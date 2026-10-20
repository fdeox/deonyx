; The installer's look: the same dark colours as the Deonyx settings window,
; a dark title bar, dark buttons and fields, and the art scaled smoothly to
; whatever the screen's size is.

!include LogicLib.nsh
!include WinMessages.nsh

; SetCtlColors reads its colours when the installer is built, so they're
; always written out, never passed in a variable.
!define DEONYX_BG "111116"
!define DEONYX_PANEL "17171E"
!define DEONYX_FIELD "1E1E27"
!define DEONYX_TEXT "ECECF3"
!define DEONYX_FAINT "6C6C80"
; The same colours as Windows wants them (0x00BBGGRR).
!define DEONYX_PANEL_REF 0x001E1717
!define DEONYX_FIELD_REF 0x00271E1E
!define DEONYX_TEXT_REF 0x00F3ECEC
!define DEONYX_ACCENT_REF 0x00F67C8B
!define DEONYX_BORDER_REF 0x00453636

SetFont "Segoe UI" 9

!define MUI_BGCOLOR "${DEONYX_PANEL}"
!define MUI_TEXTCOLOR "${DEONYX_TEXT}"
!define MUI_HEADERIMAGE_RIGHT
!define MUI_FINISHPAGE_LINK_COLOR "A99CFF"
!define MUI_CUSTOMFUNCTION_GUIINIT DeonyxGuiInit
!define MUI_CUSTOMFUNCTION_UNGUIINIT un.DeonyxGuiInit

Var deonyx.hwnd
Var deonyx.image
Var deonyx.w
Var deonyx.h

; Shows a bitmap in a picture control, scaled to fit with halftone
; (LoadImage's own scaling is blocky). The art is drawn 2.5 times the size
; of a 100% screen, so it stays sharp on big ones.
!macro DEONYX_FIT UN HWND IMAGE W H
  StrCpy $deonyx.hwnd ${HWND}
  StrCpy $deonyx.image "${IMAGE}"
  StrCpy $deonyx.w ${W}
  StrCpy $deonyx.h ${H}
  Call ${UN}DeonyxFit
!macroend

!macro DEONYX_FUNCTIONS UN

Function ${UN}DeonyxFit
  Push $0
  Push $1
  Push $2
  Push $3
  Push $4
  Push $5
  Push $6
  System::Call "*(i,i,i,i) p.r0"
  System::Call "user32::GetClientRect(p $deonyx.hwnd, p r0)"
  System::Call "*$0(i,i,i.r2,i.r3)"
  System::Free $0
  ${If} $2 > 0
  ${AndIf} $3 > 0
    ; LR_LOADFROMFILE | LR_CREATEDIBSECTION
    System::Call "user32::LoadImageW(p 0, w '$deonyx.image', i 0, i 0, i 0, i 0x2010) p.r4"
    ${If} $4 P<> 0
      System::Call "user32::GetDC(p 0) p.r0"
      System::Call "gdi32::CreateCompatibleDC(p r0) p.r5"
      System::Call "gdi32::CreateCompatibleDC(p r0) p.r6"
      System::Call "gdi32::CreateCompatibleBitmap(p r0, i r2, i r3) p.r1"
      System::Call "user32::ReleaseDC(p 0, p r0)"
      System::Call "gdi32::SelectObject(p r5, p r4)"
      System::Call "gdi32::SelectObject(p r6, p r1)"
      ; HALFTONE
      System::Call "gdi32::SetStretchBltMode(p r6, i 4)"
      System::Call "gdi32::SetBrushOrgEx(p r6, i 0, i 0, p 0)"
      System::Call "gdi32::StretchBlt(p r6, i 0, i 0, i r2, i r3, p r5, i 0, i 0, i $deonyx.w, i $deonyx.h, i 0x00CC0020)"
      ; Deleting the DCs lets go of the bitmaps they hold.
      System::Call "gdi32::DeleteDC(p r5)"
      System::Call "gdi32::DeleteDC(p r6)"
      System::Call "gdi32::DeleteObject(p r4)"
      ; STM_SETIMAGE, IMAGE_BITMAP. The old picture is MUI's, it frees it.
      SendMessage $deonyx.hwnd 0x172 0 $1
    ${EndIf}
  ${EndIf}
  Pop $6
  Pop $5
  Pop $4
  Pop $3
  Pop $2
  Pop $1
  Pop $0
FunctionEnd

; The frame around the pages. Page changes show MUI's lines again, so this
; runs on every page.
Function ${UN}DeonyxFrame
  Push $0
  Push $1
  ; MUI's light etched lines (under the header, above the buttons).
  StrCpy $0 0
  next:
    FindWindow $0 "Static" "" $HWNDPARENT $0
    StrCmp $0 0 done
    System::Call "user32::GetWindowLongW(p r0, i -16) i .r1"
    IntOp $1 $1 & 0x1F
    ${If} $1 >= 0x10
    ${AndIf} $1 <= 0x12
      ShowWindow $0 ${SW_HIDE}
    ${EndIf}
    Goto next
  done:
  ; The version text is drawn twice for a raised look; once is enough.
  GetDlgItem $0 $HWNDPARENT 1028
  ShowWindow $0 ${SW_HIDE}
  GetDlgItem $0 $HWNDPARENT 1256
  SetCtlColors $0 ${DEONYX_FAINT} ${DEONYX_BG}
  Pop $1
  Pop $0
FunctionEnd

Function ${UN}DeonyxGuiInit
  Push $0
  ; Dark title bar; on Windows 11 also its colour and a quiet border.
  System::Call 'dwmapi::DwmSetWindowAttribute(p $HWNDPARENT, i 20, *i 1, i 4)'
  System::Call 'dwmapi::DwmSetWindowAttribute(p $HWNDPARENT, i 35, *i ${DEONYX_PANEL_REF}, i 4)'
  System::Call 'dwmapi::DwmSetWindowAttribute(p $HWNDPARENT, i 34, *i ${DEONYX_BORDER_REF}, i 4)'
  SetCtlColors $HWNDPARENT ${DEONYX_TEXT} ${DEONYX_BG}
  GetDlgItem $0 $HWNDPARENT 1
  System::Call 'uxtheme::SetWindowTheme(p r0, w "DarkMode_Explorer", p 0)'
  GetDlgItem $0 $HWNDPARENT 2
  System::Call 'uxtheme::SetWindowTheme(p r0, w "DarkMode_Explorer", p 0)'
  GetDlgItem $0 $HWNDPARENT 3
  System::Call 'uxtheme::SetWindowTheme(p r0, w "DarkMode_Explorer", p 0)'
  GetDlgItem $0 $HWNDPARENT 1046
  !insertmacro DEONYX_FIT "${UN}" $0 "$PLUGINSDIR\modern-header.bmp" 375 142
  Call ${UN}DeonyxFrame
  Pop $0
FunctionEnd

!macroend

; Colours a page and everything on it, on the given background.
!macro DEONYX_COLORIZE UN NAME BG
Function ${UN}DeonyxColorize${NAME}
  Push $2
  Push $3
  Push $4
  Push $5
  SetCtlColors $deonyx.hwnd ${DEONYX_TEXT} ${BG}
  StrCpy $2 0
  next:
    FindWindow $2 "" "" $deonyx.hwnd $2
    StrCmp $2 0 done
    System::Call "user32::GetClassNameW(p r2, w .r3, i 64) i"
    System::Call "user32::GetWindowLongW(p r2, i -16) i .r4"
    ${If} $3 == "Button"
      IntOp $5 $4 & 0xF
      ${If} $5 = 0
      ${OrIf} $5 = 1
        System::Call 'uxtheme::SetWindowTheme(p r2, w "DarkMode_Explorer", p 0)'
      ${ElseIf} $5 = 7
        ; Group boxes draw a white frame; the page's title says it all.
        ShowWindow $2 ${SW_HIDE}
      ${Else}
        ; Themed check boxes and radio buttons ignore the text colour,
        ; so they get the plain look.
        System::Call 'uxtheme::SetWindowTheme(p r2, w " ", w " ")'
        SetCtlColors $2 ${DEONYX_TEXT} ${BG}
      ${EndIf}
    ${ElseIf} $3 == "Static"
      IntOp $5 $4 & 0x1F
      ${If} $5 = 0x10
      ${OrIf} $5 = 0x11
      ${OrIf} $5 = 0x12
        ShowWindow $2 ${SW_HIDE}
      ${ElseIf} $5 <> 0x3
      ${AndIf} $5 <> 0xE
        SetCtlColors $2 ${DEONYX_TEXT} ${BG}
      ${EndIf}
    ${ElseIf} $3 == "Edit"
      System::Call 'uxtheme::SetWindowTheme(p r2, w "DarkMode_CFD", p 0)'
      SetCtlColors $2 ${DEONYX_TEXT} ${DEONYX_FIELD}
    ${ElseIf} $3 == "msctls_progress32"
      System::Call 'uxtheme::SetWindowTheme(p r2, w " ", w " ")'
      SendMessage $2 0x409 0 ${DEONYX_ACCENT_REF}
      SendMessage $2 0x2001 0 ${DEONYX_FIELD_REF}
    ${ElseIf} $3 == "SysListView32"
      System::Call 'uxtheme::SetWindowTheme(p r2, w "DarkMode_Explorer", p 0)'
      SendMessage $2 0x1001 0 ${DEONYX_FIELD_REF}
      SendMessage $2 0x1026 0 ${DEONYX_FIELD_REF}
      SendMessage $2 0x1024 0 ${DEONYX_TEXT_REF}
    ${EndIf}
    Goto next
  done:
  Pop $5
  Pop $4
  Pop $3
  Pop $2
FunctionEnd
!macroend

!insertmacro DEONYX_FUNCTIONS ""
!insertmacro DEONYX_FUNCTIONS "un."
!insertmacro DEONYX_COLORIZE "" Page ${DEONYX_BG}
!insertmacro DEONYX_COLORIZE "un." Page ${DEONYX_BG}
!insertmacro DEONYX_COLORIZE "" Full ${DEONYX_PANEL}

; Show function for the ordinary pages.
!macro DEONYX_PAGE UN
Function ${UN}DeonyxPage
  Push $0
  Call ${UN}DeonyxFrame
  FindWindow $0 "#32770" "" $HWNDPARENT
  StrCpy $deonyx.hwnd $0
  Call ${UN}DeonyxColorizePage
  Pop $0
FunctionEnd
!macroend
!insertmacro DEONYX_PAGE ""
!insertmacro DEONYX_PAGE "un."

; For the welcome and finish pages, which fill the window.
!macro DEONYX_FULL_PAGE PAGE IMAGE
  Call DeonyxFrame
  !insertmacro DEONYX_FIT "" ${IMAGE} "$PLUGINSDIR\modern-wizard.bmp" 410 785
  StrCpy $deonyx.hwnd ${PAGE}
  Call DeonyxColorizeFull
!macroend
