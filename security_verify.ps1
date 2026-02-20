param(
    [switch]$NoBuild
)

$ErrorActionPreference = "Stop"

Write-Host "=== MTT Security Verification Suite ===" -ForegroundColor Cyan

if (-not $NoBuild) {
    Write-Host "[1/3] cargo test -p mtt-search-service" -ForegroundColor Yellow
    cargo test -p mtt-search-service
}

Write-Host "[2/3] cargo test -p mtt-search-protocol" -ForegroundColor Yellow
cargo test -p mtt-search-protocol

Write-Host "[3/4] cargo test -p mtt-search-service ipc_server::tests" -ForegroundColor Yellow
cargo test -p mtt-search-service ipc_server::tests

Write-Host "[4/4] cargo test -p mtt-search-service security_policy::tests" -ForegroundColor Yellow
cargo test -p mtt-search-service security_policy::tests

Write-Host "Security verification suite finished." -ForegroundColor Green
