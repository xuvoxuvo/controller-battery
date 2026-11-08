param(
    [switch]$Install,
    [string]$Destination = (Join-Path $env:LOCALAPPDATA 'Programs\ControllerBattery')
)

# 构建 release 产物。
#
# 依赖：Rust 1.75+（cargo 在 PATH 中即可）。
# 如果工具链装在非标准位置，可以设置环境变量 CARGO_EXE 指向 cargo 可执行文件。
#
# -Install 会把 exe 复制到一个普通目录（默认 %LOCALAPPDATA%\Programs\ControllerBattery）。
# 这一步不只是图方便：如果构建目录带 Low 完整性标签（例如本机沙箱工作区），
# 从里面直接启动的进程也是 Low，托盘图标会被 UIPI 拒绝注册。
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

    if ($Install) {
        New-Item -ItemType Directory -Force -Path $Destination | Out-Null
        $installed = Join-Path $Destination 'controller-battery.exe'
        Copy-Item $exe $installed -Force
        Write-Host "installed: $installed"

        # Copying out of a labeled tree drops the label; if it survived, warn.
        $label = icacls $installed 2>$null | Select-String -SimpleMatch 'Low Mandatory Level'
        if ($label) {
            Write-Warning 'destination still carries a Low integrity label; start the app from a normal folder'
        }
    }
} finally {
    Pop-Location
}
