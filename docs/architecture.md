# 架构

## 目标

* 一个 Windows 桌面小浮窗，显示手柄电量；
* 支持多种手柄：DualSense（HID 协议）、Xbox / XInput 兼容手柄（含 8BitDo 等）；
* 单一可执行文件，不依赖运行时，能在 Windows 10/11 上直接运行；
* 为将来 Linux/macOS 版本留出后端接口，协议解析层与界面层解耦。

## 目录

```
.
├─ Cargo.toml              # workspace
├─ crates/
│  ├─ controller-battery-core/        # 设备与协议层（可单测，无 UI）
│  │  └─ src/
│  │     ├─ model.rs       # BatteryReading 等公共类型
│  │     ├─ dualsense.rs   # DualSense 报文解码
│  │     ├─ hid.rs         # Windows HID 枚举/读取（SetupAPI + hid.dll）
│  │     └─ xinput.rs      # Windows XInput 后端
│  └─ controller-battery-widget/      # egui 界面 + 采集线程
├─ docs/
│  ├─ protocol.md          # 协议实测记录
│  └─ architecture.md      # 本文
└─ legacy/powershell/      # 早期 PowerShell 版本（保留作参考，不再维护）
```

## 数据流

```
 HID 线程 ──┐
            ├─> BatteryReading ──> 共享状态(Mutex) ──> egui 界面（每 250ms 重绘）
 XInput 线程 ┘
```

* 采集在后台线程中进行，界面线程只读快照，不阻塞 UI；
* 每个后端输出统一的 `BatteryReading`：设备名、传输方式、百分比、充电状态、精度、新鲜度、备注。

## 后端接口

`controller-battery-core` 目前提供两个函数式后端：

| 后端 | 平台 | 说明 |
| --- | --- | --- |
| `dualsense::decode_input_report` | 全平台（纯解析） | 输入报告 → `DecodedReport` |
| `hid::DualSenseDevice` | Windows | 枚举/打开/读取报告 |
| `xinput::poll` | Windows | 读取 4 个 XInput 槽位的电量 |

跨平台计划：

* **Linux**：`/dev/hidraw*` + `evdev`（DualSense 走 hidraw，Xbox 走 evdev 的 `ABS_*`/电池节点）；
* **macOS**：IOHIDManager（DualSense 与 Xbox 均可）；
* 协议解码层（`dualsense.rs`）与平台无关，可直接复用。

## 界面

* `egui`/`eframe` 立即模式 UI，单窗口、无边框、置顶、可拖动；
* 位置保存在 `%LOCALAPPDATA%\ControllerBattery\settings.json`；
* 只依赖系统 DLL，不引入 WebView，因此在 LTSC / 精简系统上也能跑。

## 发布

```powershell
cargo build --release
# 产物：target\release\controller-battery.exe
```

体积目标：单个 exe < 10 MB，零运行时依赖。

## 测试

* 协议解码有真实报文样本的单元测试（`cargo test -p controller-battery-core`）；
* HID/XInput 后端依赖真实设备，后续用 8BitDo 猎2代等硬件做手动验证。
