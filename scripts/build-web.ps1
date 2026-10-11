# Builds the browser client into dist/. Requires: rustup target wasm32-unknown-unknown,
# wasm-bindgen-cli 0.2.127 (cargo install wasm-bindgen-cli --version 0.2.127 --locked).
# Optional: wasm-opt (binaryen) on PATH for -O3.
param([string]$Profile = 'web', [switch]$WebGL2)
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)
$env:CARGO_TARGET_DIR = 'target-wasm'
$features = if ($WebGL2) { 'web,web-webgl2' } else { 'web' }
cargo build --profile $Profile --target wasm32-unknown-unknown -p skate-game --no-default-features --features $features --bin skate3rust
if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }
$dir = if ($Profile -eq 'dev') { 'debug' } else { $Profile }
$wasm = "target-wasm/wasm32-unknown-unknown/$dir/skate3rust.wasm"
if (Test-Path dist) { Remove-Item dist -Recurse -Force }
New-Item -ItemType Directory dist/pkg, dist/maps | Out-Null
wasm-bindgen --target web --no-typescript --out-dir dist/pkg --out-name skate3rust $wasm
if ($LASTEXITCODE -ne 0) { throw 'wasm-bindgen failed' }
if (Get-Command wasm-opt -ErrorAction SilentlyContinue) {
    wasm-opt -O3 --enable-bulk-memory --enable-sign-ext --enable-nontrapping-float-to-int --enable-reference-types --enable-mutable-globals -o dist/pkg/skate3rust_bg.wasm dist/pkg/skate3rust_bg.wasm
} else {
    Write-Warning 'wasm-opt not found; shipping unoptimised wasm'
}
Copy-Item web/index.html, web/main.js, web/host.js, web/net.js, web/frontend.js, web/pack.js, web/touch.js, web/sound-picker.html dist/
Copy-Item maps/format-demo.skate dist/maps/
# Stamp every script URL with this build's hash, so browsers and the CDN never
# pair files from different builds (an old wasm under a new skate3rust.js fails
# with "wasm.__wasm_bindgen_func_elem_... is not a function").
$hash = (Get-ChildItem dist/*.js, dist/pkg/* | Get-FileHash -Algorithm SHA256 | ForEach-Object Hash) -join ''
$build = [BitConverter]::ToString([Security.Cryptography.SHA256]::Create().ComputeHash([Text.Encoding]::ASCII.GetBytes($hash))).Replace('-', '').Substring(0, 12).ToLower()
foreach ($file in Get-ChildItem dist/*.js, dist/index.html) {
    $text = [IO.File]::ReadAllText($file.FullName)
    $text = [Text.RegularExpressions.Regex]::Replace($text, "(from\s+'\./[\w-]+\.js)'", "`$1?v=$build'")
    $text = $text.Replace('src="./main.js"', "src=`"./main.js?v=$build`"")
    [IO.File]::WriteAllText($file.FullName, $text, (New-Object Text.UTF8Encoding $false))
}
# The wasm for the game data's host (deploy/upload.py), named by build: the page
# reads it from there and its own server is only the fallback (web/host.js).
if (Test-Path deploy/engine) { Remove-Item deploy/engine/*.wasm -Force }
else { New-Item -ItemType Directory deploy/engine | Out-Null }
Copy-Item dist/pkg/skate3rust_bg.wasm "deploy/engine/skate3rust_bg-$build.wasm"
"Build $build"
Get-ChildItem dist/pkg | ForEach-Object { '{0}  {1:N1} MB' -f $_.Name, ($_.Length / 1MB) }
