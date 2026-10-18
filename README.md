# Controller Battery

[![CI](https://github.com/xuvoxuvo/controller-battery/actions/workflows/ci.yml/badge.svg)](https://github.com/xuvoxuvo/controller-battery/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

一个常驻桌面的小卡片，随时看一眼手柄还剩多少电。单个 exe，不装运行时，Win10/11 直接跑。

![界面](docs/images/widget.png)

## 特性

- **PlayStation DualSense / DualSense Edge**：直接解析 HID 报文，电量、充电状态、USB/蓝牙一目了然；
- **Xbox / XInput 兼容手柄**：读 XInput 电量档位（空/低/中/满），支持 8BitDo 等第三方手柄；
- **蓝牙手柄**：读取 Windows 的 BLE 电池属性（标准 Battery Service），精确到 1%；
- **多手柄同时显示**：每只手柄一行，窗口高度自动适配；
- 无边框、置顶、可拖动，位置自动记忆；
- 单文件约 3.7 MB，零运行时依赖。

## 支持的设备

| 设备 | 连接 | 电量来源 | 精度 |
| --- | --- | --- | --- |
| DualSense（`054C:0CE6`） | USB / 蓝牙 | HID `status[0]` | 10% |
| DualSense Edge（`054C:0DF2`） | USB / 蓝牙 | HID `status[0]` | 10% |
| Xbox 手柄 | USB / 无线适配器 | XInput | 四档 |
| 8BitDo 等 XInput 兼容手柄 | 2.4G 接收器 | XInput | 四档（部分型号不提供） |
| 8BitDo 等 BLE 手柄 | 蓝牙 | BLE Battery Service | 1% |

## 使用

从 [Releases](https://github.com/xuvoxuvo/controller-battery/releases) 下载 `controller-battery.exe`，双击即可。

- 左键拖动换位置，位置记在 `%LOCALAPPDATA%\ControllerBattery\settings.json`；
- 右键菜单 → 退出；
- 某一行变灰显示「空闲」，表示手柄暂时停止上报，数字是最后一次读数。

## 构建

需要 Rust 1.75+：

```powershell
cargo build --release
# 产物：target\release\controller-battery.exe
```

或使用脚本（`cargo` 在 PATH 中，或用 `CARGO_EXE` 指定）：

```powershell
.\scripts\build.ps1
```

运行测试：

```powershell
cargo test --workspace
```

开发、调试与发布流程见 [docs/development.md](docs/development.md)；AI 协作者的说明见 [AGENTS.md](AGENTS.md)。

## 电量精度

DualSense 上报的是 **0–10 的档位**（每档 10%），没有更细的百分比，所以本项目显示档位中点（档位 2 → 25%，真实区间 20–30%）。这是硬件限制，不是软件取整。

XInput 更粗，只有空/低/中/满四档；BLE 手柄则是真实的 1% 百分比。

## 已知限制

- 手柄空闲时会停止上报，插拔充电线本身不触发刷新，需要按一下键或推一下摇杆；
- 蓝牙下的 DualSense 有时会发「简化报文」，其中电量字段不刷新；程序会自动请求切回全量报文，失败时显示 `≈` 和「状态未知」；
- 8BitDo Ultimate 2 在 2.4G 模式下不提供电量（XInput 只报「有线/满」，厂商协议里也没有电量命令），切到蓝牙即可看到精确值。

更多细节见 [docs/protocol.md](docs/protocol.md)。

## 路线图

- [ ] Linux（hidraw + evdev）与 macOS（IOHIDManager）后端
- [ ] 托盘图标 / 开机自启
- [ ] 更多厂商协议（DualShock 4、Switch Pro 等）
- [ ] 低电量提醒

## 项目结构

```
crates/controller-battery-core/     协议解析与设备访问（可单测）
  ├─ dualsense.rs            DualSense 报文解码
  ├─ hid.rs                  Windows HID（SetupAPI）
  ├─ xinput.rs               XInput 电量
  └─ bluetooth.rs            BLE 电池属性
crates/controller-battery-widget/   egui 界面与采集线程
docs/protocol.md             报文实测记录
docs/architecture.md         架构说明
docs/legacy/powershell/      早期 PowerShell 版本（仅存档）
```

调试真机时可以跑探针：

```powershell
cargo run -p controller-battery-core --example probe          # HID + XInput 概览
cargo run -p controller-battery-core --example bt_battery     # BLE 电池
cargo run -p controller-battery-core --example probe_8bitdo   # 8BitDo 厂商通道
```

## 许可证

[MIT](LICENSE)
