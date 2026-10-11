# Converts your own Skate 3 disc into the assets the web client imports.
# Usage: powershell -ExecutionPolicy Bypass -File scripts\prepare-web-assets.ps1 -Source "D:\path\skate3.iso"
param(
    [Parameter(Mandatory = $true)][string]$Source,
    [string]$Base = 'data'
)
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)
$exe = 'target-native/release/skate3rust.exe'
if (-not (Test-Path -LiteralPath $exe)) {
    Write-Host 'Building the native engine (needed to validate each converted map)...'
    cargo build --release -p skate-game --bin skate3rust --no-default-features --target-dir target-native
    if ($LASTEXITCODE -ne 0) { throw 'Native build failed' }
}
foreach ($module in 'numpy', 'PIL') {
    python -c "import $module" 2>$null
    if ($LASTEXITCODE -ne 0) { throw "Missing Python module $module. Run: python -m pip install -r tools/requirements-setup.txt" }
}
# ISO extraction downloads the extract-xiso tool from its GitHub release; the
# disc itself is read locally and never uploaded.
python tools/prepare_iso.py $Source --base $Base --game-exe $exe
if ($LASTEXITCODE -ne 0) { throw 'Conversion failed; see the output above' }
Write-Host ''
Write-Host 'Next: serve the web client together with the INSTALLED folder above:'
Write-Host '  SERVE.bat'
