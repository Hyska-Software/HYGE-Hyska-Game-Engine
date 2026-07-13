Hyge Editor Windows package
===========================

Start the editor with HygeEditor.cmd. If no project directory is passed, the
launcher opens a folder selector and then starts the Rust backend from bin\.

Examples:
  HygeEditor.cmd
  HygeEditor.cmd C:\path\to\hyge-project
  HygeEditor.cmd C:\path\to\hyge-project --scene main.hyge-world

HygeEditor.exe is the frozen Qt frontend child. It is not the package entry
point and must not be opened directly because it has no backend address or
authentication token until hyge-tools.exe launches it.

A valid project directory must contain at least one .hyge-world scene.
