# 用 C:\App 下的工具链构建 release 产物。
$ErrorActionPreference = 'Stop'

$env:RUSTUP_HOME = 'C:\App\Rust\rustup'
$env:CARGO_HOME = 'C:\App\Rust\cargo'
if (Test-Path 'C:\App\mingw64\bin') {
    $env:PATH = 'C:\App\mingw64\bin;' + $env:PATH
}

$root = Split-Path $PSScriptRoot -Parent
Push-Location $root
try {
    & 'C:\App\Rust\cargo\bin\cargo.exe' build --release -p battery-widget
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed: $LASTEXITCODE" }
    $exe = Join-Path $root 'target\release\controller-battery.exe'
    $size = [math]::Round((Get-Item $exe).Length / 1MB, 1)
    Write-Host "built: $exe ($size MB)"
} finally {
    Pop-Location
}
