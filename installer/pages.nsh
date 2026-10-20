; Texts and page helpers that need the pages and languages to exist first.

LangString deonyxWelcomeTitle ${LANG_ENGLISH} "Welcome to Deonyx"
LangString deonyxWelcomeText ${LANG_ENGLISH} "Deonyx gives any Windows app a dark mode, even the ones that never had one.$\r$\n$\r$\nIt waits quietly in the tray. Pick the apps you want dark, or let it darken every light window.$\r$\n$\r$\nClick Next to continue."
LangString deonyxFinishTitle ${LANG_ENGLISH} "Deonyx is ready"
LangString deonyxFinishText ${LANG_ENGLISH} "You'll find Deonyx in the tray, next to the clock.$\r$\n$\r$\nTip: Ctrl+Alt+F darkens the app in front of you, Ctrl+Alt+D turns darkening on and off."

LangString deonyxWelcomeTitle ${LANG_TURKISH} "Deonyx'e hoş geldin"
LangString deonyxWelcomeText ${LANG_TURKISH} "Deonyx, karanlık modu hiç olmayanlar dahil her Windows uygulamasını karartır.$\r$\n$\r$\nSistem tepsisinde sessizce bekler. Karartmak istediğin uygulamaları seçersin ya da açık renkli her pencereyi karartır.$\r$\n$\r$\nDevam etmek için İleri'ye tıkla."
LangString deonyxFinishTitle ${LANG_TURKISH} "Deonyx hazır"
LangString deonyxFinishText ${LANG_TURKISH} "Deonyx'i saatin yanındaki sistem tepsisinde bulabilirsin.$\r$\n$\r$\nİpucu: Ctrl+Alt+F öndeki uygulamayı karartır, Ctrl+Alt+D karartmayı açıp kapatır."

Function DeonyxWelcomeShow
  !insertmacro DEONYX_FULL_PAGE $mui.WelcomePage $mui.WelcomePage.Image
FunctionEnd

Function DeonyxFinishShow
  !insertmacro DEONYX_FULL_PAGE $mui.FinishPage $mui.FinishPage.Image
FunctionEnd
