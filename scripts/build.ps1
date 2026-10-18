# 构建 release 产物。
#
# 依赖：Rust 1.75+（cargo 在 PATH 中即可）。
# 如果工具链装在非标准位置，可以设置环境变量 CARGO_EXE 指向 cargo 可执行文件。
$ErrorActionPreference = 'Stop'

$cargo = $env:CARGO_EXE
if (-not $cargo) {
    $found = Get-Command cargo -ErrorAction SilentlyContinue
    if ($found) { $cargo = $found.Source }
}
if (-not $cargo) {
    throw 'cargo not found: install Rust (https://rustup.rs) or set CARGO_EXE'
}

$root = Split-Path $PSScriptRoot -Parent
Push-Location $root
try {
    & $cargo build --release -p controller-battery-widget
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed: $LASTEXITCODE" }
    $exe = Join-Path $root 'target\release\controller-battery.exe'
    $size = [math]::Round((Get-Item $exe).Length / 1MB, 1)
    Write-Host "built: $exe ($size MB)"
} finally {
    Pop-Location
}
