# 开发与发布

面向修改本项目的人；普通用户请看 [README](../README.md)。

AI 代理请先读根目录的 [AGENTS.md](../AGENTS.md)（架构、命令、约定与已知坑）。

## 环境

- Windows 10 1809+ / Windows 11
- Rust 1.75+（[rustup](https://rustup.rs)）
- MSVC 或 GNU 工具链均可；GNU 工具链需要 MinGW-w64 提供 `dlltool`/`ld`

当前只实现了 Windows 后端，Linux/macOS 需要补 `hidraw` / `IOHIDManager` 后端，见 `docs/architecture.md`。

## 常用命令

```powershell
cargo build                      # 调试构建
cargo build --release            # 发布构建 → target/release/controller-battery.exe
cargo test --workspace           # 单元测试（协议解码）
cargo fmt --all                  # 格式化
cargo clippy --workspace --all-targets -- -D warnings
```

或直接运行 `.\scripts\build.ps1`（要求 `cargo` 在 PATH 中，或用 `CARGO_EXE` 指定路径）。

## 真机调试

```powershell
cargo run -p controller-battery-core --example probe         # HID + XInput 概览
cargo run -p controller-battery-core --example bt_battery    # 蓝牙电池属性
cargo run -p controller-battery-core --example hid_dump -- <vid_hex> <pid_hex>
cargo run -p controller-battery-core --example capture -- <vid_hex> - 20
cargo run -p controller-battery-core --example probe_8bitdo  # 8BitDo 厂商通道
```

`capture` 会打印 20 秒内每个字节的变化次数，适合定位未知手柄的电量字段。

## 目录

```
crates/controller-battery-core/     协议与设备访问（无 UI，可单测）
  ├─ dualsense.rs                   DualSense 报文解码
  ├─ hid.rs                         Windows HID（SetupAPI）
  ├─ xinput.rs                      XInput 电量
  └─ bluetooth.rs                   BLE 电池属性
crates/controller-battery-widget/   egui 界面与采集线程
docs/protocol.md                    报文实测记录
docs/architecture.md                架构说明
docs/legacy/powershell/             早期 PowerShell 版本（仅存档）
```

## 发布流程

仓库带两个 GitHub Actions 工作流：

**`.github/workflows/ci.yml`** —— 每次 push / PR：

1. `cargo fmt --check`
2. `cargo clippy --workspace --all-targets -- -D warnings`
3. `cargo test --workspace`
4. `cargo build --workspace --release`
5. 上传 `controller-battery.exe` 作为 artifact

**`.github/workflows/release.yml`** —— 推送 `v*` 标签时：

1. 跑测试并构建 release；
2. 打包 `controller-battery-<version>-windows-x64.zip`；
3. 用 `softprops/action-gh-release` 创建 GitHub Release，附上 exe 与 zip，并自动生成 release notes。

发布步骤：

```powershell
# 1. 更新 CHANGELOG.md 与 Cargo.toml 里的版本号
# 2. 提交
git commit -am "release: v0.1.0"
# 3. 打标签并推送
git tag v0.1.0
git push origin main --tags
```

标签推送后，Actions 会自动产出 Release 附件。

## 新增一种手柄

1. 在 `controller-battery-core` 里加一个后端模块，输出 `BatteryReading`；
2. 在 `controller-battery-widget/src/collector.rs` 里起一个采集线程（参考 `spawn_bluetooth`）；
3. 用 `capture` 示例抓包，确认电量字段后再写解码；
4. 给解码补单元测试（真实报文样本）。
