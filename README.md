# Controller Battery

[![CI](https://github.com/xuvoxuvo/controller-battery/actions/workflows/ci.yml/badge.svg)](https://github.com/xuvoxuvo/controller-battery/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

一个常驻桌面的小卡片，随时看一眼手柄还剩多少电。单个 exe，不装运行时，Win10/11 直接跑。

![界面](docs/images/widget.png)

## 特性

- **PlayStation DualSense / DualSense Edge**：直接解析 HID 报文，电量、充电状态、USB/蓝牙一目了然；
- **Xbox / XInput 兼容手柄**：读 XInput 电量档位（空/低/中/满），支持 8BitDo 等第三方手柄；
- **8BitDo Ultimate 2 Wireless**：DInput 模式（2.4G 接收器，按住 `B` 开机）下解析 34 字节 HID 报文，精确到 1%；
- **蓝牙手柄**：读取 Windows 的 BLE 电池属性（标准 Battery Service），精确到 1%；
- **多手柄同时显示**：每只手柄一行，窗口高度自动适配；
- **托盘图标**：通知区用电池图标显示电量，悬停看百分比；左键显示/隐藏卡片，右键菜单退出；
- 无边框、置顶、可拖动，位置自动记忆；
- 单文件约 4.2 MB（含图标与版本信息），零运行时依赖。

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
- 卡片上右键 → 「隐藏到托盘」或「退出」；
- 托盘图标：左键切换显示/隐藏卡片，右键菜单里有「显示卡片 / 隐藏卡片 / 退出」，悬停显示手柄名和电量；
- 某一行变灰显示「空闲」，表示手柄暂时停止上报，数字是最后一次读数。

托盘图标会跟着电量和充电状态变色，并自动适配浅色/深色任务栏：

![托盘图标](docs/images/tray-icon.png)

> Windows 11 默认把新图标折叠进通知区的「^」里：点一下「^」就能看到电池图标，也可以右键任务栏 →「任务栏设置」→「其他系统托盘图标」把 Controller Battery 打开，让它常驻。
> 托盘图标需要**从资源管理器双击启动**：如果程序是被沙箱/受限环境（低完整性进程）拉起来的，Windows 不允许它注册托盘图标，菜单里的「隐藏到托盘」会置灰并在悬停时说明原因；程序每 5 秒自动重试，条件满足后会自动可用。
> 排查时看 `%LOCALAPPDATA%\ControllerBattery\tray.log`：里面记录了失败时的错误码、本进程与 shell 的完整性级别、会话号，反馈问题时附上它即可。
> 如果 exe 所在目录带 `Low` 完整性标签（部分沙箱/受限环境会这么做，`icacls <目录>` 可见 `Mandatory Label\Low Mandatory Level`），从该目录启动的进程会被降级运行、注册不了托盘图标——把 exe 复制到普通目录（如 `C:\App\ControllerBattery`）再运行即可。

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
- 部分第三方手柄（盖世小鸡等）在无线连接时不上报电量，XInput 返回的 wired 只表示「无电量信息」，界面显示 -- 与「未报告电量」；这类手柄切到蓝牙通常能读到精确值。
- 8BitDo Ultimate 2 Wireless 的接收器有两种形态：正常开机是 XInput（`2DC8:310B`，没有电量）；按住 `B` 开机是 DInput（`2DC8:6012`，有精确电量）。两种模式都能被本程序识别，想要电量就按住 `B` 开机，需要原生 XInput（不经过 Steam）就正常开机。

更多细节见 [docs/protocol.md](docs/protocol.md)。

## 路线图

- [x] 托盘图标（电量图标、显示/隐藏卡片、退出）
- [ ] 开机自启（注册表 `Run` 项，独立于托盘实现）
- [ ] Linux（hidraw + evdev）与 macOS（IOHIDManager）后端
- [ ] 更多厂商协议（DualShock 4、Switch Pro 等）
- [ ] 低电量提醒

## 项目结构

```
crates/controller-battery-core/     协议解析与设备访问（可单测）
  ├─ dualsense.rs            DualSense 报文解码
  ├─ eightbitdo.rs           8BitDo DInput 报文（电量/充电）
  ├─ hid.rs                  Windows HID（SetupAPI）
  ├─ xinput.rs               XInput 电量
  └─ bluetooth.rs            BLE 电池属性
crates/controller-battery-widget/   egui 界面、采集线程与托盘图标
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
