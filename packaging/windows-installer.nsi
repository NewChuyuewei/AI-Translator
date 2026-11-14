; Per-user Windows x64 installer for the GNU cross-compiled portable build.
; Linux: LC_ALL=C.UTF-8 makensis -INPUTCHARSET UTF8 -DINPUT_DIR=/path/to/payload \
;   -DOUTPUT_FILE=/path/to/Gemini-Live-Translator-Setup.exe packaging/windows-installer.nsi
Unicode true
RequestExecutionLevel user

!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "x64.nsh"

!ifndef INPUT_DIR
  !error "Define INPUT_DIR containing the app exe and WebView2Loader.dll"
!endif
!ifndef OUTPUT_FILE
  !error "Define OUTPUT_FILE for the installer exe"
!endif

!define PRODUCT_NAME "Gemini Live Translator"
!define PRODUCT_VERSION "0.1.6"
!define PRODUCT_PUBLISHER "chuyuewei"
!define PRODUCT_COPYRIGHT "Copyright © 2026 chuyuewei. All rights reserved."
!define APP_EXE "Gemini-Live-Translator.exe"
!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\GeminiLiveTranslator"

Name "${PRODUCT_NAME}"
OutFile "${OUTPUT_FILE}"
VIProductVersion "${PRODUCT_VERSION}.0"
VIAddVersionKey /LANG=2052 "ProductName" "${PRODUCT_NAME}"
VIAddVersionKey /LANG=2052 "ProductVersion" "${PRODUCT_VERSION}"
VIAddVersionKey /LANG=2052 "CompanyName" "${PRODUCT_PUBLISHER}"
VIAddVersionKey /LANG=2052 "LegalCopyright" "${PRODUCT_COPYRIGHT}"
InstallDir "$LOCALAPPDATA\Programs\Gemini Live Translator"
InstallDirRegKey HKCU "Software\GeminiLiveTranslator" "InstallDir"
ShowInstDetails show
ShowUninstDetails show

!define MUI_ABORTWARNING
!define MUI_WELCOMEPAGE_TEXT "此安装程序将为当前 Windows 用户安装 Gemini Live Translator。$\r$\n$\r$\n应用需要 Microsoft Edge WebView2 Runtime；如果电脑没有安装，请从 Microsoft 官网安装。$\r$\n$\r$\n这是未经 Windows 真机验证的开发原型。"
!define MUI_FINISHPAGE_RUN "$INSTDIR\${APP_EXE}"
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "SimpChinese"

Function .onInit
  ${IfNot} ${RunningX64}
    MessageBox MB_OK|MB_ICONSTOP "此版本需要 64 位 Windows。"
    Abort
  ${EndIf}
FunctionEnd

Section "Install"
  SetShellVarContext current
  SetOutPath "$INSTDIR"
  File "${INPUT_DIR}/${APP_EXE}"
  File "${INPUT_DIR}/WebView2Loader.dll"
  File /oname=COPYRIGHT.txt "${__FILEDIR__}/../COPYRIGHT"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  CreateDirectory "$SMPROGRAMS\${PRODUCT_NAME}"
  CreateShortCut "$SMPROGRAMS\${PRODUCT_NAME}\${PRODUCT_NAME}.lnk" "$INSTDIR\${APP_EXE}"
  CreateShortCut "$SMPROGRAMS\${PRODUCT_NAME}\卸载.lnk" "$INSTDIR\Uninstall.exe"
  WriteRegStr HKCU "Software\GeminiLiveTranslator" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayName" "${PRODUCT_NAME}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayVersion" "${PRODUCT_VERSION}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "Publisher" "${PRODUCT_PUBLISHER}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\${APP_EXE}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoRepair" 1
SectionEnd

Section "Uninstall"
  SetShellVarContext current
  Delete "$SMPROGRAMS\${PRODUCT_NAME}\${PRODUCT_NAME}.lnk"
  Delete "$SMPROGRAMS\${PRODUCT_NAME}\卸载.lnk"
  RMDir "$SMPROGRAMS\${PRODUCT_NAME}"
  Delete "$INSTDIR\${APP_EXE}"
  Delete "$INSTDIR\WebView2Loader.dll"
  Delete "$INSTDIR\COPYRIGHT.txt"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
  DeleteRegKey HKCU "Software\GeminiLiveTranslator"
  DeleteRegKey HKCU "${UNINSTALL_KEY}"
SectionEnd
