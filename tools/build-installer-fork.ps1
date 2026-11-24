# Builds the release binary and wraps it in the Quottyall installer.
#   powershell -ExecutionPolicy Bypass -File tools\build-installer-fork.ps1
# Output: dist\Quottyall-Setup-<version>.exe.
#
# This is the fork's own script: it uses installer\quottyall.iss, which has a
# different AppId and install directory from the original Quotty, so running it
# never replaces an installed original Quotty.
param([string]$Iscc = "")

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

# rustup installs cargo per user and only adds it to new shells' PATH; a build
# started from another host (or the IDE) can miss it.
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    $cargoBin = Join-Path $env:USERPROFILE ".cargo\bin"
    if (Test-Path (Join-Path $cargoBin "cargo.exe")) { $env:PATH = "$cargoBin;$env:PATH" }
}
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    throw "cargo не найден. Установите Rust (rustup) или добавьте %USERPROFILE%\.cargo\bin в PATH."
}

# Single source of truth for the version: Cargo.toml.
$version = (Select-String -Path "Cargo.toml" -Pattern '^version\s*=\s*"(.+)"' |
            Select-Object -First 1).Matches[0].Groups[1].Value
Write-Output "Quottyall $version"

if (-not $Iscc) {
    $candidates = @(
        "D:\Programs\Inno Setup 6\ISCC.exe",
        (Join-Path ${env:ProgramFiles(x86)} "Inno Setup 6\ISCC.exe"),
        (Join-Path $env:ProgramFiles "Inno Setup 6\ISCC.exe"),
        (Join-Path $env:LOCALAPPDATA "Programs\Inno Setup 6\ISCC.exe")
    )
    $Iscc = $candidates | Where-Object { Test-Path $_ } | Select-Object -First 1
}
if (-not $Iscc -or -not (Test-Path $Iscc)) {
    throw "Inno Setup 6 не найден. Укажите -Iscc <путь к ISCC.exe> или установите Inno Setup 6."
}

# The cargo wrapper also prunes target\ of what older builds left behind.
& "$PSScriptRoot\cargo.ps1" build --release
if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

New-Item -ItemType Directory -Force -Path dist | Out-Null
& $Iscc "/DAppVersion=$version" "installer\quottyall.iss"
if ($LASTEXITCODE -ne 0) { throw "ISCC failed" }

$setup = "dist\Quottyall-Setup-$version.exe"
"{0}  {1:N2} MB" -f $setup, ((Get-Item $setup).Length / 1MB)
