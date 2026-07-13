[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$PackageDirectory,
    [Parameter(Mandatory = $true)][string]$FixtureDirectory,
    [string]$EvidenceDirectory = ""
)

$ErrorActionPreference = "Stop"
$package = (Resolve-Path $PackageDirectory).Path
$fixture = (Resolve-Path $FixtureDirectory).Path
$temp = Join-Path ([System.IO.Path]::GetTempPath()) ("hyge-r104-smoke-" + [guid]::NewGuid().ToString("N"))
$project = Join-Path $temp "project"
if ($EvidenceDirectory) {
    $resolvedEvidence = Resolve-Path $EvidenceDirectory -ErrorAction SilentlyContinue
    if ($resolvedEvidence) { $evidence = $resolvedEvidence.Path }
    else { $evidence = Join-Path (Get-Location) $EvidenceDirectory }
} else {
    $evidence = Join-Path $temp "evidence"
}
$backend = Join-Path $package "bin\hyge-tools.exe"
$launcher = Join-Path $package "HygeEditor.cmd"
if (-not (Test-Path $backend)) { throw "Package is missing bin\hyge-tools.exe" }
if (-not (Test-Path (Join-Path $package "HygeEditor.exe"))) { throw "Package is missing HygeEditor.exe" }

New-Item -ItemType Directory -Force -Path $temp, $evidence | Out-Null
Copy-Item $fixture $project -Recurse
$source = Join-Path $project "assets\source\triangle.gltf"
$cook = Join-Path $project "assets\cook"
& $backend import $source --out $cook
if ($LASTEXITCODE -ne 0) { throw "Packaged hyge-tools asset import failed." }
Copy-Item (Join-Path $cook ".hyge.db") (Join-Path $project ".hyge.db") -Force

$oldPath = $env:PATH
$env:PATH = $env:SystemRoot + "\System32"
$env:QT_QPA_PLATFORM = "offscreen"
$env:QT_QUICK_BACKEND = "software"
$env:HYGE_EDITOR_BOOT_LOG = Join-Path $evidence "boot.log"
try {
    $argumentString = '"{0}" --port 0 --scene "main.hyge-world" --external-scene "external.hyge-world" --evidence-dir "{1}"' -f $project, $evidence
    $process = Start-Process -FilePath $launcher -ArgumentList $argumentString -PassThru -WindowStyle Hidden
    if (-not $process.WaitForExit(120000)) {
        & taskkill.exe /PID $process.Id /T /F | Out-Null
        throw "Packaged editor smoke test timed out."
    }
    if ($process.ExitCode -ne 0) { throw "Packaged editor exited with code $($process.ExitCode)." }
}
finally {
    $env:PATH = $oldPath
    Remove-Item Env:QT_QPA_PLATFORM -ErrorAction SilentlyContinue
    Remove-Item Env:QT_QUICK_BACKEND -ErrorAction SilentlyContinue
    Remove-Item Env:HYGE_EDITOR_BOOT_LOG -ErrorAction SilentlyContinue
}

$workflowPath = Join-Path $evidence "workflow.json"
$manifestPath = Join-Path $evidence "manifest.json"
$tracePath = Join-Path $evidence "protocol.jsonl"
foreach ($required in @("editor.png", "viewport.png", "saved.hyge-world", "workflow.json", "manifest.json", "protocol.jsonl")) {
    if (-not (Test-Path (Join-Path $evidence $required))) { throw "Smoke evidence is missing $required" }
}
$workflow = Get-Content -Raw $workflowPath | ConvertFrom-Json
$manifest = Get-Content -Raw $manifestPath | ConvertFrom-Json
if (-not $workflow.success) { throw "Packaged workflow did not report success." }
$translation = @($workflow.reload_translation | ForEach-Object { [double]$_ })
if ($translation.Count -ne 3 -or $translation[0] -ne 3 -or $translation[1] -ne 1 -or $translation[2] -ne 0) {
    throw "PersistOnReload evidence is invalid."
}
if ((Get-Item $tracePath).Length -le 0) { throw "Protocol trace is empty." }
$result = [ordered]@{
    status = "complete"
    package = $package
    evidence = $evidence
    workflow_success = $workflow.success
    editor_png = $manifest.editor_png
    viewport_png = $manifest.viewport_png
    saved_scene = $manifest.saved_scene
    path_independent = $true
}
$result | ConvertTo-Json | Set-Content (Join-Path $evidence "r104-smoke.json") -Encoding UTF8
Write-Host ($result | ConvertTo-Json -Compress)
