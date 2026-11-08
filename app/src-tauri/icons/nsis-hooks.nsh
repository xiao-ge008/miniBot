!macro NSIS_HOOK_POSTINSTALL
  File "/oname=$INSTDIR\app-icon.ico" "${INSTALLERICON}"
  WriteRegStr SHCTX "${UNINSTKEY}" "DisplayIcon" "$\"$INSTDIR\app-icon.ico$\""
  ${If} ${FileExists} "$SMPROGRAMS\${PRODUCTNAME}.lnk"
    CreateShortcut "$SMPROGRAMS\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe" "" "$INSTDIR\app-icon.ico" 0
  ${EndIf}
  ${If} ${FileExists} "$DESKTOP\${PRODUCTNAME}.lnk"
    CreateShortcut "$DESKTOP\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe" "" "$INSTDIR\app-icon.ico" 0
  ${EndIf}
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0x2000, i 0, i 0)'
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  Delete "$INSTDIR\app-icon.ico"
  RMDir "$INSTDIR"
!macroend
