# Verifies that the keys entered locally really return live data, and prints the
# numbers Quottyall will render. Reads the DPAPI-sealed key file the app uses
# (%APPDATA%\Quotty\<provider>.key) and never prints, echoes or logs the key.
#
#   powershell -ExecutionPolicy Bypass -File tools\probe-providers.ps1
#   powershell -ExecutionPolicy Bypass -File tools\probe-providers.ps1 -Provider minimax -DumpRaw $env:TEMP\mm.json
#
# -DumpRaw writes the endpoint's JSON body (no credentials in it) so the Rust
# parser tests can be pointed at a real response:
#   $env:QUOTTY_MINIMAX_FIXTURE = "C:\...\mm.json"
#   cargo test minimax -- --nocapture
param(
    [ValidateSet("minimax", "deepseek", "both")]
    [string]$Provider = "both",
    [string]$DumpRaw = ""
)

$ErrorActionPreference = "Stop"

# Windows PowerShell keeps DPAPI in System.Security.dll: load it explicitly so
# the script works without the caller setting anything up.
Add-Type -AssemblyName System.Security

function Decode-Key([string]$name) {
    $base = if ($env:APPDATA) { $env:APPDATA } else { Join-Path $env:USERPROFILE "AppData\Roaming" }
    $path = Join-Path $base "Quottyall\$name.key"
    if (-not (Test-Path $path)) { return $null }
    $raw = [System.IO.File]::ReadAllBytes($path)
    $magic = [System.Text.Encoding]::ASCII.GetBytes("QTMM1")
    if ($raw.Length -le $magic.Length) { throw "Файл ключа $name повреждён." }
    for ($i = 0; $i -lt $magic.Length; $i++) {
        if ($raw[$i] -ne $magic[$i]) { throw "Файл ключа $name повреждён." }
    }
    $blob = New-Object byte[] ($raw.Length - $magic.Length)
    [Array]::Copy($raw, $magic.Length, $blob, 0, $blob.Length)
    $plain = [System.Security.Cryptography.ProtectedData]::Unprotect(
        $blob, $null, [System.Security.Cryptography.DataProtectionScope]::CurrentUser)
    $text = [System.Text.Encoding]::UTF8.GetString($plain)
    [Array]::Clear($plain, 0, $plain.Length)
    return $text.Trim()
}

function Invoke-Api([string]$url, [string]$key) {
    $headers = @{ Authorization = "Bearer $key"; Accept = "application/json" }
    try {
        $resp = Invoke-WebRequest -Uri $url -Headers $headers -Method Get -TimeoutSec 25 -UseBasicParsing
        return @{ Status = [int]$resp.StatusCode; Body = $resp.Content }
    } catch {
Write-Warning ("Request error: {0}: {1}" -f $_.Exception.GetType().Name, $_.Exception.Message)
        $code = 0
        if ($_.Exception.Response) { $code = [int]$_.Exception.Response.StatusCode }
        $body = ""
        try { $body = $_.ErrorDetails.Message } catch { }
        return @{ Status = $code; Body = $body }
    }
}

function Show-MiniMax($key, [string]$dump) {
    Write-Output "`n=== MiniMax M Plan: GET https://www.minimax.io/v1/token_plan/remains ==="
    $r = Invoke-Api "https://www.minimax.io/v1/token_plan/remains" $key
    if ($r.Status -ne 200) {
        Write-Output ("HTTP {0} — ключ не принят или сервис недоступен" -f $r.Status)
        if ($r.Body) { Write-Output ("body: {0}" -f $r.Body) }
        return
    }
    $j = $r.Body | ConvertFrom-Json
    if ($null -ne $j.base_resp -and [int]$j.base_resp.status_code -ne 0) {
        Write-Output ("base_resp.status_code={0} status_msg={1}" -f $j.base_resp.status_code, $j.base_resp.status_msg)
        return
    }
    if ($dump) {
        # UTF-8 without BOM: the Rust fixture reader treats the file as the raw
        # HTTP body, and a BOM would not be part of a real response.
        $rawEnc = New-Object System.Text.UTF8Encoding($false)
        [System.IO.File]::WriteAllText($dump, $r.Body, $rawEnc)
        Write-Output ("raw JSON saved: {0}" -f $dump)
    }
    $rows = @($j.model_remains)
    Write-Output ("model_remains rows: {0} ({1})" -f $rows.Count, (($rows | ForEach-Object { $_.model_name }) -join ", "))
    $g = $rows | Where-Object { $_.model_name -eq "general" } | Select-Object -First 1
    if ($null -eq $g) { Write-Output "нет строки general"; return }

    function Show-Window($label, $pct, $endMs) {
        $used = 100 - [double]$pct
        $reset = if ($endMs) {
            [DateTimeOffset]::FromUnixTimeMilliseconds([int64]$endMs).ToLocalTime().ToString("yyyy-MM-dd HH:mm:ss zzz")
        } else { "—" }
        Write-Output ("{0,-10} remaining {1,3}%  ->  used {2,5:F1}%   reset {3}" -f $label, $pct, $used, $reset)
    }
    Show-Window "5 Hours" $g.current_interval_remaining_percent $g.end_time
    Show-Window "Weekly" $g.current_weekly_remaining_percent $g.weekly_end_time
    Write-Output ("status: interval={0} weekly={1}" -f $g.current_interval_status, $g.current_weekly_status)
    Write-Output "plan tier: not in the response (the app labels it 'MiniMax Go'); M Plan FAQ is the source."
}

function Show-DeepSeek($key, [string]$dump) {
    Write-Output "`n=== DeepSeek: GET https://api.deepseek.com/user/balance ==="
    $r = Invoke-Api "https://api.deepseek.com/user/balance" $key
    if ($r.Status -ne 200) {
        $hint = switch ($r.Status) {
            401 { "ключ не принят" }
            402 { "на балансе нет средств (это не ошибка ключа)" }
            429 { "слишком часто" }
            default { "сервис недоступен" }
        }
        Write-Output ("HTTP {0} — {1}" -f $r.Status, $hint)
        if ($r.Body) { Write-Output ("body: {0}" -f $r.Body) }
        return
    }
    if ($dump) {
        # UTF-8 without BOM: the Rust fixture reader treats the file as the raw
        # HTTP body, and a BOM would not be part of a real response.
        $rawEnc = New-Object System.Text.UTF8Encoding($false)
        [System.IO.File]::WriteAllText($dump, $r.Body, $rawEnc)
        Write-Output ("raw JSON saved: {0}" -f $dump)
    }
    $j = $r.Body | ConvertFrom-Json
    Write-Output ("is_available (enough funds for API calls): {0}" -f $j.is_available)
    foreach ($b in @($j.balance_infos)) {
        $sym = switch ($b.currency) { "CNY" { "¥" } "USD" { "$" } default { $b.currency } }
        Write-Output ("{0} total {1}{2}  = topped-up {3}{4} + granted {5}{6}" -f `
            $b.currency, $sym, $b.total_balance, $sym, $b.topped_up_balance, $sym, $b.granted_balance)
    }
    Write-Output "DeepSeek bills money per token: no 5-hour or weekly window exists."
}

$targets = if ($Provider -eq "both") { @("minimax", "deepseek") } else { @($Provider) }
if (-not $DumpRaw) { $DumpRaw = "" }

foreach ($name in $targets) {
    $key = Decode-Key $name
    if (-not $key) {
        Write-Output "`n[$name] ключ не сохранён — пропускаю. Введите его: tools\set-key.ps1 -Provider $name"
        continue
    }
    Write-Output "`n[$name] key loaded from the DPAPI store (not printed)."
    $dump = if ($DumpRaw -and $targets.Count -eq 1) { $DumpRaw } else { "" }
    if ($name -eq "minimax") { Show-MiniMax $key $dump } else { Show-DeepSeek $key $dump }
    $key = $null
}
