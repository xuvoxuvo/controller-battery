# Changelog

本项目遵循 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/) 与语义化版本。

## [Unreleased]

### Added

- Rust workspace：`controller-battery-core`（协议与设备层）+ `controller-battery-widget`（egui 悬浮窗）。
- DualSense / DualSense Edge 输入报告解码，含真实报文单元测试。
- Windows HID 枚举与读取（SetupAPI + hid.dll）。
- XInput 电量读取，支持 Xbox 兼容手柄。
- 悬浮窗：置顶、无边框、可拖动，位置持久化。
- 文档：`docs/protocol.md`、`docs/architecture.md`。

### Fixed

- 窗口真正透明，圆角不再出现深色方块；
- 同时连接多个手柄时每个手柄各占一行（此前只显示优先级最高的一个）。

### Added

- 蓝牙手柄电量：读取 Windows 的 `DEVPKEY_Bluetooth_Battery`（BLE Battery Service），8BitDo Ultimate 2 蓝牙模式可显示精确百分比。

### Changed

- 早期 PowerShell 版本移动到 `legacy/powershell`，不再维护。

## [0.1.0] - 计划中

首个可运行版本：DualSense + XInput 支持，单文件发布。
