# Changelog

本项目遵循 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/) 与语义化版本。

## [Unreleased]

### Added

- Rust workspace：`battery-core`（协议与设备层）+ `battery-widget`（egui 悬浮窗）。
- DualSense / DualSense Edge 输入报告解码，含真实报文单元测试。
- Windows HID 枚举与读取（SetupAPI + hid.dll）。
- XInput 电量读取，支持 Xbox 兼容手柄。
- 悬浮窗：置顶、无边框、可拖动，位置持久化。
- 文档：`docs/protocol.md`、`docs/architecture.md`。

### Changed

- 早期 PowerShell 版本移动到 `legacy/powershell`，不再维护。

## [0.1.0] - 计划中

首个可运行版本：DualSense + XInput 支持，单文件发布。
