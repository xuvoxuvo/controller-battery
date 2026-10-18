# Changelog

本项目遵循 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/) 与[语义化版本](https://semver.org/lang/zh-CN/)。

## [Unreleased]

### Added

- **DualSense / DualSense Edge**：HID 报文解码，支持 USB（`0x01`，64 字节）、蓝牙全量（`0x31`，78 字节）与蓝牙简化报文（`0x01`，78 字节）三种格式；解析电量档位与充电状态。
- **Windows HID 后端**：SetupAPI 枚举、读写句柄；以写权限打开并发送空操作输出报文，让 DualSense 从简化报文切回全量报文。
- **XInput 后端**：轮询 4 个槽位，读取电量档位（空/低/中/满）与电池类型（有线/碱性/镍氢）。
- **蓝牙电量**：读取 Windows 的 `DEVPKEY_Bluetooth_Battery`（BLE Battery Service），蓝牙手柄可显示 1% 精度。
- **多手柄界面**：每只手柄一行，窗口高度自适应；最多显示 5 行，超出提示「还有 N 个设备」。
- **界面细节**：超椭圆圆角、真正透明的窗口、无边框置顶、拖动与位置记忆、设备名按像素宽度省略。
- **调试工具**：`probe`、`hid_dump`、`capture`、`bt_battery`、`probe_8bitdo` 示例，用于真机抓包与协议验证。

### Changed

- 项目定名为 **Controller Battery**，可执行文件 `controller-battery.exe`，crates 为 `controller-battery-core` / `controller-battery-widget`。
- 早期 PowerShell 版本移至 `docs/legacy/powershell`，仅作存档，不再维护。
- 发布流程：推送 `v*` 标签时由 GitHub Actions 自动构建、打包并发布 Release。
- README 改写为面向用户；构建、调试与发布细节移至 `docs/development.md`。

### Fixed

- release 构建不再弹出黑色控制台窗口（使用 Windows GUI 子系统）。
- 修复 DualSense 蓝牙简化报文导致的电量/充电状态为旧值的问题。
- 修复设备切换、休眠时窗口尺寸跳变与「未检测到手柄」闪动。
- 修复切到 2.4G 后残留旧蓝牙行的问题（改为每帧过滤）。
- 蓝牙断开判断改用 WinRT BluetoothLEDevice.ConnectionStatus：已配对但断开的设备不再显示，断连约 1 秒内消失。
- 修复设备名被截断、XInput 行错误沿用其他设备名称的问题。
- 修复 BLE 轮询源被误判为「空闲」的问题（每个数据源独立空闲阈值）。

### Known issues

- 8BitDo Ultimate 2 在 2.4G 模式下不提供电量（XInput 只报「有线/满」，厂商 `0x81` 命令集无电量命令）；切到蓝牙即可读取。
- Linux / macOS 后端尚未实现。

## [0.1.0] - 计划中

首个公开版本：Windows 上的 DualSense、XInput 与蓝牙手柄电量浮窗。
