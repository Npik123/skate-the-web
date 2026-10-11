# Starts the local server for the browser build.
#
#   SERVE.bat                -> client + game data on http://127.0.0.1
#   SERVE.bat -Local         -> this machine only (http://127.0.0.1)
#   SERVE.bat -Assets <dir>  -> a specific converted folder on this machine
#   SERVE.bat -Pack <url|zip> -> a game-data pack instead; without this, a pack
#                               is only used when no converted folder exists
#   SERVE.bat -Port 9000     -> another port
#   SERVE.bat -DownloadLimit 40 -> cap game-data downloads at 40 Mbit/s so a
#                               friend loading in cannot starve multiplayer
#
# One process serves both the client in dist/ and the game data you converted
# from your own disc; the client streams files from it on demand and stores
# nothing, so there is no second server and no browser storage to fill up. It
# listens on every interface, so phones and other machines on your network can
# open it too. Node runs in this window — close it (or press Ctrl+C) to stop.
param(
    [string]$Assets,
    [string]$Pack,
    [int]$Port = 80,
    [switch]$Local,
    [int]$DownloadLimit = 0,
    [switch]$DryRun
)
$ErrorActionPreference = 'Stop'
# Port 80 is the browser default, so the address to share needs no port.
$PortSuffix = if ($Port -eq 80) { '' } else { ":$Port" }
$Root = Split-Path $PSScriptRoot -Parent
Set-Location $Root

if (-not (Get-Command node -ErrorAction SilentlyContinue)) {
    throw 'Node.js was not found on PATH. Install it from https://nodejs.org and run this again.'
}
if (-not (Test-Path 'dist/index.html')) {
    throw 'dist/ is missing. Build the web client first: powershell -File scripts\build-web.ps1'
}

# What tools/prepare_iso.py printed as INSTALLED: assets/, maps/ and settings/.
function Find-Install {
    if ($Assets) {
        $dir = (Resolve-Path $Assets).Path
        if (-not (Test-Path (Join-Path $dir 'assets/private/game.json'))) {
            throw "$dir does not look like a converted install (no assets/private/game.json)."
        }
        return $dir
    }
    $candidates = @()
    if (Test-Path 'data/installations') {
        $candidates += Get-ChildItem 'data/installations' -Directory -ErrorAction SilentlyContinue |
            Where-Object { Test-Path (Join-Path $_.FullName 'assets/private/game.json') }
    }
    if (Test-Path 'assets/private/game.json') { $candidates += Get-Item $Root }
    if ($candidates.Count -eq 0) { return $null }
    return ($candidates | Sort-Object LastWriteTime -Descending | Select-Object -First 1).FullName
}

$install = Find-Install
if ($Local) {
    Write-Host ''
    Write-Host "Open http://127.0.0.1$PortSuffix" -ForegroundColor Cyan
} else {
    $ip = (Get-NetIPAddress -AddressFamily IPv4 -ErrorAction SilentlyContinue |
        Where-Object { $_.IPAddress -notlike '127.*' -and $_.IPAddress -notlike '169.254.*' } |
        Select-Object -First 1 -ExpandProperty IPAddress)
    Write-Host ''
    Write-Host "Open this on any machine on your network: http://$(if ($ip) { $ip } else { '127.0.0.1' })$PortSuffix" -ForegroundColor Cyan
    Write-Host 'That address hands out the game data you converted from your own disc.' -ForegroundColor DarkGray
    Write-Host 'Keep it on a network you control.' -ForegroundColor DarkGray
}

if ($install) {
    Write-Host "Game data : $install"
} elseif (-not $Pack) {
    Write-Host 'Game data : none on this machine — the pack below is downloaded once instead.' -ForegroundColor Yellow
    Write-Host '            (or convert your own disc: scripts\prepare-web-assets.ps1 -Source "D:\path\skate3.iso")' -ForegroundColor DarkGray
}
Write-Host ''

$args = @('scripts\serve-web.js', 'dist', $Port)
if ($install) { $args += $install }
if ($Pack) { $args += "--pack=$Pack" }
if ($Local) { $args += '--local' }
if ($DownloadLimit -gt 0) { $args += "--download-limit=$DownloadLimit" }
if ($DryRun) {
    Write-Host "[dry run] node $($args -join ' ')"
    return
}
& node @args
