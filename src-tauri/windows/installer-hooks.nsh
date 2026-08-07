!include "WinVer.nsh"

!macro NSIS_HOOK_PREINSTALL
  ${IfNot} ${AtLeastBuild} 19045
    MessageBox MB_ICONSTOP|MB_OK "GPTEasy 需要 Windows 10 22H2（内部版本 19045）或更高版本。"
    Abort
  ${EndIf}
!macroend
