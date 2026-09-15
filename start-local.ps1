param([string]$Allowlist = "$PSScriptRoot/data/allowed-users.json")
$ErrorActionPreference = 'Stop'
if (-not (Test-Path -LiteralPath $Allowlist -PathType Leaf)) {
    throw 'Create data/allowed-users.json with your email/name entries, using configuration/allowed-users.sample.json as a template, or pass -Allowlist <path>.'
}
$env:APP_AUTH_MODE = 'allowlist'
$env:APP_EMAIL_ALLOWLIST = (Resolve-Path -LiteralPath $Allowlist).Path
$env:APP_COOKIE_SECURE = 'false'
$env:CLUSTERING_WEB_BIND = '127.0.0.1:8080'
Push-Location $PSScriptRoot
try { cargo run --release --locked; if ($LASTEXITCODE -ne 0) { throw "Application exited with code $LASTEXITCODE" } }
finally { Pop-Location }
