# Stores a provider API key for Quottyall, encrypted with Windows DPAPI under
# the current user. The key is typed into a masked prompt: it is never echoed,
# never passed as an argument (so it cannot land in shell history or a process
# listing) and never written anywhere in plain text.
#
#   powershell -ExecutionPolicy Bypass -File tools\set-key.ps1 -Provider minimax
#   powershell -ExecutionPolicy Bypass -File tools\set-key.ps1 -Provider deepseek
#   powershell -ExecutionPolicy Bypass -File tools\set-key.ps1 -Provider minimax -Remove
#
# This is an alternative to the app's own "Ключи доступа" settings section: both
# write the same %APPDATA%\Quotty\<provider>.key file, so either can be used.
param(
    [Parameter(Mandatory = $true)]
    [ValidateSet("minimax", "deepseek")]
    [string]$Provider,
    [switch]$Remove
)

$ErrorActionPreference = "Stop"

# Windows PowerShell keeps DPAPI in System.Security.dll: load it explicitly so
# the script works without the caller setting anything up.
Add-Type -AssemblyName System.Security

# Same layout the Rust side reads: 5-byte magic + DPAPI blob.
$Magic = [System.Text.Encoding]::ASCII.GetBytes("QTMM1")

function Get-KeyPath([string]$name) {
    $base = if ($env:APPDATA) { $env:APPDATA } else { Join-Path $env:USERPROFILE "AppData\Roaming" }
    return Join-Path $base "Quottyall\$name.key"
}

$path = Get-KeyPath $Provider

if ($Remove) {
    if (Test-Path $path) {
        Remove-Item $path -Force
        Write-Output "Ключ $Provider удалён."
    } else {
        Write-Output "Ключа $Provider не было."
    }
    exit 0
}

$secure = Read-Host -Prompt "Вставьте ключ $Provider (ввод не отображается)" -AsSecureString

# Turn the SecureString into bytes without ever creating a plaintext string or
# leaving the value in a PowerShell variable that outlives this block.
$bstr = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($secure)
try {
    $plain = [Runtime.InteropServices.Marshal]::PtrToStringBSTR($bstr)
    $bytes = [System.Text.Encoding]::UTF8.GetBytes($plain.Trim())
} finally {
    [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($bstr)
    $plain = $null
}

if ($bytes.Length -eq 0) { throw "Пустой ключ — ничего не сохранено." }
if ($bytes.Length -gt 4096) { throw "Ключ слишком длинный — ничего не сохранено." }

$sealed = [System.Security.Cryptography.ProtectedData]::Protect(
    $bytes, $null, [System.Security.Cryptography.DataProtectionScope]::CurrentUser)
[Array]::Clear($bytes, 0, $bytes.Length)

$dir = Split-Path -Parent $path
New-Item -ItemType Directory -Force -Path $dir | Out-Null

$out = New-Object byte[] ($Magic.Length + $sealed.Length)
[Array]::Copy($Magic, 0, $out, 0, $Magic.Length)
[Array]::Copy($sealed, 0, $out, $Magic.Length, $sealed.Length)

$tmp = "$path.tmp"
[System.IO.File]::WriteAllBytes($tmp, $out)
Move-Item -Force $tmp $path

Write-Output "Ключ $Provider сохранён в $path (зашифрован Windows DPAPI, только для текущего пользователя)."
Write-Output "Проверить данные: powershell -ExecutionPolicy Bypass -File tools\probe-providers.ps1 -Provider $Provider"
