[CmdletBinding()]
param(
    [string]$OutputDirectory = "",
    [string]$PythonExecutable = ""
)

$ErrorActionPreference = "Stop"
$root = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$frontendRoot = Join-Path $root "tools\hyge-editor-python"
if ($OutputDirectory) {
    $resolvedOutput = Resolve-Path $OutputDirectory -ErrorAction SilentlyContinue
    if ($resolvedOutput) { $output = $resolvedOutput.Path }
    else { $output = Join-Path $root $OutputDirectory }
} else {
    $output = Join-Path $root "target\editor-windows"
}
$venv = Join-Path $frontendRoot ".venv-r104"
$venvPython = Join-Path $venv "Scripts\python.exe"
$venvScripts = Join-Path $venv "Scripts"
$cargo = "C:\Users\estev\.cargo\bin\cargo.exe"

if ($PythonExecutable) {
    $pythonLauncher = (Resolve-Path $PythonExecutable).Path
} elseif (-not (Get-Command py -ErrorAction SilentlyContinue)) {
    throw "Python launcher 'py' is required to create the Python 3.12 build environment."
} else {
    $pythonLauncher = "py"
}
if (-not (Test-Path $venvPython)) {
    if ($PythonExecutable) { & $pythonLauncher -m venv $venv }
    else { & $pythonLauncher -3.12 -m venv $venv }
    if ($LASTEXITCODE -ne 0) { throw "Python 3.12 virtual environment creation failed." }
}
if (-not (Test-Path (Join-Path $venv "Scripts\pyside6-deploy.exe"))) {
    & $venvPython -m pip install --upgrade pip
    & $venvPython -m pip install -e "$frontendRoot[test,build]"
    if ($LASTEXITCODE -ne 0) { throw "Editor frontend dependency installation failed." }
}

$dumpbin = Get-Command dumpbin -ErrorAction SilentlyContinue
if (-not $dumpbin) {
    throw "dumpbin.exe was not found. Run this script from a Visual Studio Developer PowerShell."
}
if (-not (Test-Path $cargo)) {
    throw "Required Rustup cargo shim was not found: $cargo"
}

Push-Location $frontendRoot
$oldPath = $env:PATH
$env:PATH = "$venvScripts;$env:PATH"
$deploymentSpec = Join-Path $frontendRoot "pysidedeploy.spec"
$deploymentSpecSource = [System.IO.File]::ReadAllText($deploymentSpec)
try {
    & (Join-Path $venv "Scripts\pyside6-project.exe") build
    if ($LASTEXITCODE -ne 0) { throw "pyside6-project build failed." }
    & (Join-Path $venv "Scripts\pyside6-deploy.exe") -c pysidedeploy.spec --force
    if ($LASTEXITCODE -ne 0) { throw "pyside6-deploy failed." }
}
finally {
    $env:PATH = $oldPath
    [System.IO.File]::WriteAllText($deploymentSpec, $deploymentSpecSource, [System.Text.UTF8Encoding]::new($false))
    Pop-Location
}

$packageRoot = Join-Path $output "HygeEditor"
if (Test-Path $packageRoot) {
    Remove-Item -Recurse -Force $packageRoot
}

& $cargo build -p hyge-tools --release
if ($LASTEXITCODE -ne 0) { throw "Release hyge-tools build failed." }

$frontendExe = Get-ChildItem -Path (Join-Path $root "target\editor-windows") -Filter "HygeEditor.exe" -File -Recurse |
    Select-Object -First 1
if (-not $frontendExe) { throw "pyside6-deploy did not produce HygeEditor.exe." }

New-Item -ItemType Directory -Force -Path $packageRoot | Out-Null
Get-ChildItem -Path $frontendExe.Directory.FullName -Force | Copy-Item -Destination $packageRoot -Recurse -Force
New-Item -ItemType Directory -Force -Path (Join-Path $packageRoot "bin") | Out-Null
Copy-Item (Join-Path $root "target\release\hyge-tools.exe") (Join-Path $packageRoot "bin\hyge-tools.exe") -Force
Copy-Item (Join-Path $frontendRoot "HygeEditor.cmd") $packageRoot -Force
Copy-Item (Join-Path $frontendRoot "select_project.ps1") $packageRoot -Force
Copy-Item (Join-Path $frontendRoot "README.txt") $packageRoot -Force
New-Item -ItemType Directory -Force -Path (Join-Path $packageRoot "qml") | Out-Null
Copy-Item (Join-Path $frontendRoot "qml\Main.qml") (Join-Path $packageRoot "qml\Main.qml") -Force

# Nuitka may copy the declarative QML tree without the plugin DLLs referenced
# by qmldir. Copy the validated PySide6 modules explicitly so the package is
# relocatable and never depends on the build environment.
$pysideQmlRoot = Join-Path $venv "Lib\site-packages\PySide6\qml"
$packageQmlRoot = Join-Path $packageRoot "PySide6\qml"
foreach ($qmlModule in @("QtQuick\Controls", "QtQuick\Layouts", "QtQuick\NativeStyle", "QtQuick\Templates", "QtQuick\Window", "QtQuick\Effects", "QtQuick\Controls\Windows")) {
    $sourceModule = Join-Path $pysideQmlRoot $qmlModule
    $destinationModule = Join-Path $packageQmlRoot $qmlModule
    if (-not (Test-Path -LiteralPath $sourceModule)) {
        throw "Required PySide6 QML module is missing: $sourceModule"
    }
    New-Item -ItemType Directory -Force -Path $destinationModule | Out-Null
    Copy-Item -Path (Join-Path $sourceModule '*') -Destination $destinationModule -Recurse -Force
}
foreach ($qtDllPattern in @("Qt6QuickControls2*.dll", "Qt6QuickTemplates2.dll", "Qt6QuickLayouts.dll")) {
    $sourceDlls = Get-ChildItem -Path (Join-Path $venv "Lib\site-packages\PySide6") -Filter $qtDllPattern -File
    if (-not $sourceDlls) {
        throw "Required PySide6 Qt libraries are missing: $qtDllPattern"
    }
    foreach ($sourceDll in $sourceDlls) {
        Copy-Item -LiteralPath $sourceDll.FullName -Destination (Join-Path $packageRoot $sourceDll.Name) -Force
    }
}
foreach ($qtDll in @("Qt6QuickEffects.dll")) {
    $sourceDll = Join-Path $venv "Lib\site-packages\PySide6\$qtDll"
    if (-not (Test-Path -LiteralPath $sourceDll)) {
        throw "Required PySide6 Qt library is missing: $sourceDll"
    }
    Copy-Item -LiteralPath $sourceDll -Destination (Join-Path $packageRoot $qtDll) -Force
}

$manifest = [ordered]@{
    package = "HygeEditor"
    version = "0.1.0"
    python = (& $venvPython --version)
    pyside6 = (& $venvPython -c "import PySide6; print(PySide6.__version__)")
    rust_binary = (& (Join-Path $packageRoot "bin\hyge-tools.exe") --version)
    platform = [System.Runtime.InteropServices.RuntimeInformation]::OSDescription
    architecture = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
    commands = @(
        "python -m venv tools/hyge-editor-python/.venv-r104",
        "pyside6-project build",
        "pyside6-deploy -c pysidedeploy.spec --force",
        "cargo build -p hyge-tools --release"
    )
    files = @{}
}
function Get-Sha256Hex([string]$Path) {
    $sha = [System.Security.Cryptography.SHA256]::Create()
    try {
        $bytes = [System.IO.File]::ReadAllBytes($Path)
        return ([System.BitConverter]::ToString($sha.ComputeHash($bytes))).Replace("-", "").ToLowerInvariant()
    }
    finally { $sha.Dispose() }
}
Get-ChildItem -Path $packageRoot -File -Recurse | Sort-Object FullName | ForEach-Object {
    $relative = $_.FullName.Substring($packageRoot.Length + 1).Replace("\", "/")
    $manifest.files[$relative] = Get-Sha256Hex $_.FullName
}
$manifestPath = Join-Path $packageRoot "package-manifest.json"
$manifest | ConvertTo-Json -Depth 8 | Set-Content -Path $manifestPath -Encoding UTF8
Write-Host "HygeEditor package created at $packageRoot"
