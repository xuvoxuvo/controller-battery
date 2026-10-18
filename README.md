# Controller Battery

一个常驻桌面的小卡片，用来查看手柄电量。

- **PlayStation DualSense / DualSense Edge**：走 HID 协议直接读，精度 10%；
- **Xbox / XInput 兼容手柄**（含 8BitDo 等）：走 XInput，精度是「空 / 低 / 中 / 满」四档；
- 单个 exe，不需要安装运行时，Win10/11 直接运行。

> 项目早期版本是一个 PowerShell 脚本，保留在 [`legacy/powershell`](legacy/powershell) 里作参考，不再维护。

## 状态

`0.1.0` 开发中：核心协议层、Windows HID / XInput 后端、悬浮窗已完成；正在做真机联调。

## 构建

需要 Rust（本机装在 `C:\App\Rust`）：

```powershell
cargo build --release
# 产物：target\release\controller-battery.exe
```

运行测试：

```powershell
cargo test
```

## 使用

双击 `controller-battery.exe`，桌面出现浮窗：

- 左键拖动移动位置，位置会记住；
- 右键菜单 → 退出；
- 手柄空闲时显示灰色的「空闲」，表示手柄暂时不再上报，数字是最后一次读数。

## 支持的设备

| 设备 | 后端 | 电量精度 | 充电状态 |
| --- | --- | --- | --- |
| DualSense（PID `0x0CE6`） | HID | 10%（11 档） | 有 |
| DualSense Edge（PID `0x0DF2`） | HID | 10%（11 档） | 有 |
| Xbox 官方手柄 | XInput | 四档 | 有线/无线 |
| 8BitDo 等 Xbox 兼容手柄 | XInput | 四档 | 视固件而定 |

目前只实现了 Windows 后端；协议解析层（`battery-core::dualsense`）与平台无关，Linux/macOS 后端见「路线图」。

## 电量精度

**DualSense 只能做到 10% 一档，这是硬件限制，不是软件取整。**

手柄上报的是 0–10 的档位（每档 10%），没有更细的百分比字段。本项目显示档位中点，例如档位 2 → 25%，真实区间是 20%–30%。想再精确只能靠外部库仑计。

XInput 手柄更粗：只有空 / 低 / 中 / 满四档，映射成 5% / 25% / 60% / 100% 仅用于展示。

## 已知限制

- 蓝牙下 DualSense 有时会发「简化报文」，其中的电量字段不刷新，界面会显示 `≈` 和「状态未知」；
- 手柄空闲会停止上报，插拔充电线本身不触发刷新，需要按一下键或推一下摇杆；
- 8BitDo 猎2代等手柄目前走 XInput 通用接口，电量只有四档，后续可针对其 HID 协议做精确读取。

## 路线图

- [ ] 8BitDo 猎2代真机联调，评估其私有 HID 电量协议
- [ ] 托盘图标 / 开机自启
- [ ] Linux（hidraw + evdev）与 macOS（IOHIDManager）后端
- [ ] 多手柄切换（同时连接多个手柄时显示哪一个）
- [ ] GitHub Actions 自动构建与 release 附件

## 开发

```
Cargo.toml                  workspace
crates/battery-core/        协议与设备访问（可单测）
crates/battery-widget/      egui 界面
docs/protocol.md            DualSense 报文实测记录
docs/architecture.md        架构说明
legacy/powershell/          早期 PowerShell 版本
```

调试真机时可以直接跑探针：

```powershell
cargo run -p battery-core --example probe
```

它会列出系统里的 HID 设备、DualSense 的报文解码结果，以及 XInput 槽位。

## 许可证

MIT，见 [LICENSE](LICENSE)。
