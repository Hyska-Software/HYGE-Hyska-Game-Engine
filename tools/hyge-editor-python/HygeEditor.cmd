@echo off
setlocal
set "PACKAGE_DIR=%~dp0"
set "HYGE_EDITOR_PACKAGE_DIR=%PACKAGE_DIR%"
set "BACKEND=%PACKAGE_DIR%bin\hyge-tools.exe"
set "FRONTEND=%PACKAGE_DIR%HygeEditor.exe"

if not exist "%BACKEND%" (
    >&2 echo HygeEditor package is incomplete: missing bin\hyge-tools.exe
    exit /b 2
)
if not exist "%FRONTEND%" (
    >&2 echo HygeEditor package is incomplete: missing HygeEditor.exe
    exit /b 2
)
set "PROJECT=%~1"
if not "%~1"=="" shift
set "PORT=0"
set "SCENE="
set "EVIDENCE="
set "EXTERNAL="

if not defined PROJECT (
    if not exist "%PACKAGE_DIR%select_project.ps1" (
        >&2 echo HygeEditor package is incomplete: missing select_project.ps1
        exit /b 2
    )
    for /f "usebackq delims=" %%P in (`"%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -STA -ExecutionPolicy Bypass -File "%PACKAGE_DIR%select_project.ps1"`) do set "PROJECT=%%P"
    if not defined PROJECT (
        >&2 echo No project was selected. HygeEditor was not started.
        exit /b 2
    )
)
if not exist "%PROJECT%\" (
    >&2 echo Invalid Hyge project directory: "%PROJECT%"
    exit /b 2
)
dir /b "%PROJECT%\*.hyge-world" >nul 2>&1
if errorlevel 1 (
    >&2 echo Invalid Hyge project: no .hyge-world scene was found in "%PROJECT%"
    exit /b 2
)

:parse_args
if "%~1"=="" goto run
if /i "%~1"=="--port" (
    set "PORT=%~2"
    shift
    shift
    goto parse_args
)
if /i "%~1"=="--scene" (
    set "SCENE=%~2"
    shift
    shift
    goto parse_args
)
if /i "%~1"=="--evidence-dir" (
    set "EVIDENCE=%~2"
    shift
    shift
    goto parse_args
)
if /i "%~1"=="--external-scene" (
    set "EXTERNAL=%~2"
    shift
    shift
    goto parse_args
)
>&2 echo Unknown HygeEditor option: %~1
exit /b 2

:run
set "ARGS=editor "%PROJECT%" --port %PORT% --frontend "%FRONTEND%""
if defined SCENE set "ARGS=%ARGS% --scene "%SCENE%""
if defined EVIDENCE set "ARGS=%ARGS% --evidence-dir "%EVIDENCE%""
if defined EXTERNAL set "ARGS=%ARGS% --external-scene "%EXTERNAL%""
call "%BACKEND%" %ARGS%
exit /b %ERRORLEVEL%
