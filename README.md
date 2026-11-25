# Controller Battery

[![CI](https://github.com/xuvoxuvo/controller-battery/actions/workflows/ci.yml/badge.svg)](https://github.com/xuvoxuvo/controller-battery/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

一个常驻桌面的小卡片，随时看一眼手柄还剩多少电。单个 exe，不装运行时，Win10/11 直接跑。

![界面](docs/images/widget.png)

## 特性

- **PlayStation DualSense / DualSense Edge**：电量、充电状态、USB/蓝牙一目了然；
- **Xbox / XInput 兼容手柄**：读 XInput 电量档位（空/低/中/满），支持 8BitDo 等第三方手柄；
- **8BitDo Ultimate 2 Wireless**：按住 `B` 开机的 DInput 模式下可读精确电量；
- **蓝牙手柄**：读取 Windows 的 BLE 电池属性，精确到 1%；
- **多手柄同时显示**：每只手柄一行，窗口高度自动适配；
- **托盘图标**：通知区显示电量，悬停看百分比，左键显示/隐藏卡片，右键退出；
- **多语言**：默认跟随系统，也可手动切换简体中文 / 繁體中文 / English；
- **空闲自动隐藏**：可选，手柄闲置或断开时卡片自动收起，手柄重新上报时自动出现；
- 无边框、置顶、可拖动，位置自动记忆；
- 单文件约 4.2 MB，零运行时依赖。

## 支持的设备

| 设备 | 连接 | 电量来源 | 精度 |
| --- | --- | --- | --- |
| DualSense（`054C:0CE6`） | USB / 蓝牙 | HID `status[0]` | 10% |
| DualSense Edge（`054C:0DF2`） | USB / 蓝牙 | HID `status[0]` | 10% |
| Xbox 手柄 | USB / 无线适配器 | XInput | 四档 |
| 8BitDo 等 XInput 兼容手柄 | 2.4G 接收器 | XInput | 四档（部分型号不提供） |
| 8BitDo Ultimate 2 Wireless（`2DC8:6012`，DInput） | 2.4G 接收器 / USB 线 | HID 报文第 14 字节 | 1% |
| 8BitDo 等 BLE 手柄 | 蓝牙 | BLE Battery Service | 1% |

## 使用

从 [Releases](https://github.com/xuvoxuvo/controller-battery/releases) 下载 `controller-battery.exe`，双击即可。

- 左键拖动换位置，位置记在 `%LOCALAPPDATA%\ControllerBattery\settings.json`；
- 卡片上右键：勾选「空闲时自动隐藏」、切换语言（跟随系统 / 简体中文 / 繁體中文 / English）、「隐藏到托盘」或「退出」；
- 勾选「空闲时自动隐藏」后，手柄闲置或断开约 8 秒卡片自动收起；手柄重新上报时自动出现。手动显示或隐藏过卡片后，不会在下次空闲时立刻弹回；
- 托盘图标：左键切换显示/隐藏卡片，右键菜单里有「显示卡片 / 隐藏卡片 / 退出」，悬停显示手柄名和电量；
- 某一行变灰显示「空闲」，表示手柄暂时停止上报，数字是最后一次读数；
- 托盘图标找不到，或「隐藏到托盘」置灰：见 [疑难排查](docs/troubleshooting.md)。

托盘图标会跟着电量和充电状态变色，并自动适配浅色/深色任务栏（上排深色、下排浅色）：

![托盘图标](docs/images/tray-icon.png)

## 电量精度

DualSense 上报的是 0–10 的档位（每档 10%），没有更细的百分比，所以显示档位中点（档位 2 → 25%，真实区间 20–30%）；XInput 只有空/低/中/满四档；BLE 手柄则是真实的 1% 百分比。这是硬件限制，不是软件取整。

## 已知限制

- 手柄空闲会停止上报，插拔充电线本身不触发刷新，按一下键或推一下摇杆才会更新；
- 蓝牙下的 DualSense 偶尔会发「简化报文」，其中电量不刷新；程序会自动请求切回全量报文，失败时显示 `≈` 与「状态未知」；
- 部分第三方手柄（盖世小鸡等）无线连接时不上报电量，界面显示 `--` 与「未报告电量」，切到蓝牙通常能读到精确值；
- 8BitDo Ultimate 2 Wireless 只在按住 `B` 开机（DInput 模式）时上报电量；正常开机走原生 XInput，读不到电量。

协议细节与实测记录见 [docs/protocol.md](docs/protocol.md)。

## 路线图

- [x] 托盘图标（电量图标、显示/隐藏卡片、退出）
- [ ] 开机自启（注册表 `Run` 项，独立于托盘实现）
- [ ] Linux（hidraw + evdev）与 macOS（IOHIDManager）后端
- [ ] 更多厂商协议（DualShock 4、Switch Pro 等）
- [ ] 低电量提醒

## 构建

需要 Rust 1.75+：

```powershell
cargo build --release
# 产物：target\release\controller-battery.exe
```

构建脚本、真机调试、CI 与发布流程见 [docs/development.md](docs/development.md)。

## 文档

- [开发与发布](docs/development.md)
- [疑难排查](docs/troubleshooting.md)
- [协议笔记](docs/protocol.md)
- [架构说明](docs/architecture.md)
- [AI 协作者说明](AGENTS.md)

## 许可证

[MIT](LICENSE)
