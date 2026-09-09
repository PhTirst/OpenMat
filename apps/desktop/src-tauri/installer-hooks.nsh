!macro OPENMAT_REMOVE_OBSOLETE_FILES
  ; OpenMat 0.1.0 shipped the native server as a sidecar. The desktop host now
  ; links it in-process, so remove the orphan when upgrading an existing install.
  Delete /REBOOTOK "$INSTDIR\openmat-server.exe"
  ; These first-party license filenames belonged to earlier local builds.
  ; The current package installs OPENMAT-AGPL-3.0.txt and third-party notices.
  Delete /REBOOTOK "$INSTDIR\resources\licenses\OpenMat-LICENSE-MIT.txt"
  Delete /REBOOTOK "$INSTDIR\resources\licenses\OpenMat-LICENSE-APACHE.txt"
!macroend

!macro NSIS_HOOK_POSTINSTALL
  !insertmacro OPENMAT_REMOVE_OBSOLETE_FILES
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  !insertmacro OPENMAT_REMOVE_OBSOLETE_FILES
!macroend
