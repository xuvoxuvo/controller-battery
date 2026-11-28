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
│  │     ├─ model.rs       # BatteryReading 等公共类型与 FNV-1a 指纹
│  │     ├─ dualsense.rs   # DualSense 报文解码
│  │     ├─ dualshock4.rs  # DualShock 4 报文解码
│  │     ├─ switch_pro.rs  # Switch Pro / Joy-Con 报文解码
│  │     ├─ eightbitdo.rs  # 8BitDo DInput 报文（电量/充电）
│  │     ├─ hid.rs         # Windows HID 枚举/读取（SetupAPI + hid.dll）
│  │     ├─ xinput.rs      # Windows XInput 后端
│  │     └─ bluetooth.rs   # Windows BLE Battery Service 后端
│  └─ controller-battery-widget/      # egui 界面 + 采集线程
│     └─ src/
│        ├─ app.rs         # 卡片绘制、语言切换、超椭圆缓存
│        ├─ collector.rs   # 多协议后台采集与共享 HID 枚举缓存
│        ├─ i18n.rs        # 简体中文 / 繁體中文 / English 文案表
│        ├─ icon_glyph.rs  # 托盘与 exe 共用的电池图标几何
│        ├─ autohide.rs    # 空闲自动隐藏的状态机（可单测）
│        ├─ menu.rs        # 卡片右键菜单（原生 Win32）
│        ├─ settings.rs    # 持久化设置与多屏坐标校验
│        ├─ single.rs      # 命名互斥量单实例保护
│        └─ tray.rs        # 托盘图标、非阻塞淡入淡出、主题监听
├─ docs/
│  ├─ protocol.md          # 协议实测记录
│  ├─ architecture.md      # 本文
│  ├─ development.md       # 构建、真机调试与发布
│  ├─ troubleshooting.md   # 用户侧疑难排查
│  └─ legacy/powershell/   # 早期 PowerShell 版本（保留作参考，不再维护）
└─ scripts/build.ps1       # 本地构建与安装脚本
```

## 数据流

```
 HID 线程（DualSense / DS4 / Switch Pro / 8BitDo）──┐
 XInput 线程                                       ──┼─> BatteryReading ──> 共享状态(Mutex) ──> egui 界面（自适应重绘）
 蓝牙线程（BLE Battery Service）                    ──┘
```

* 采集在后台线程中进行，HID 枚举通过短时缓存共享，界面线程只读快照，不阻塞 UI；
* 每个后端输出统一的 `BatteryReading`：设备名、传输方式、百分比、充电状态、精度、新鲜度、备注。

## 后端接口

`controller-battery-core` 目前提供以下协议与系统后端：

| 后端 | 平台 | 说明 |
| --- | --- | --- |
| `dualsense::decode_input_report` | 全平台（纯解析） | DualSense 输入报告 → `DecodedReport` |
| `dualshock4::decode_input_report` | 全平台（纯解析） | DualShock 4 USB/蓝牙输入报告 → `DecodedReport` |
| `switch_pro::decode_input_report` | 全平台（纯解析） | Switch Pro / Joy-Con 输入报告 → `PowerState` |
| `eightbitdo::decode_power_state` | 全平台（纯解析） | 8BitDo DInput 34 字节报告 → 电量/充电 |
| `hid::HidDevice` | Windows | 枚举/打开/读取报告与序列号 |
| `xinput::poll` | Windows | 读取 4 个 XInput 槽位的电量 |
| `bluetooth::enumerate_filtered` | Windows | 按名称前置过滤后读取 BLE Battery Service 电量 |

跨平台计划：

* **Linux**：`/dev/hidraw*` + `evdev`（DualSense 走 hidraw，Xbox 走 evdev 的 `ABS_*`/电池节点）；
* **macOS**：IOHIDManager（DualSense 与 Xbox 均可）；
* 协议解码层（`dualsense.rs`）与平台无关，可直接复用。

## 界面

* `egui`/`eframe` 立即模式 UI，单窗口、无边框、可拖动，窗口层级可在「始终置顶 / 普通窗口 / 始终置底」间切换（通过 `egui::WindowLevel` 与 Win32 `HWND_BOTTOM` 强制维护实现）；
* 位置、窗口层级、语言与自动隐藏开关保存在 `%LOCALAPPDATA%\ControllerBattery\settings.json`；
* 界面文案统一走 `i18n.rs`：`Language::System` 跟随 Windows 界面语言，也可在右键菜单固定为简体中文 / 繁體中文 / English；
* 右键菜单是原生 Win32 菜单（`menu.rs`）：egui 的弹出层被限制在卡片窗口里会被裁剪，原生菜单能超出卡片、在屏幕边缘自动翻转，并跟随系统主题；
* 只依赖系统 DLL，不引入 WebView，因此在 LTSC / 精简系统上也能跑。

### 空闲自动隐藏

`autohide.rs` 是一个不碰 Win32 的状态机，由托盘线程的 1 秒定时器驱动（卡片隐藏后 egui 可能不再重绘，不能放在 UI 线程）：

* 所有手柄空闲（1 秒无上报，XInput 2 秒）→ 卡片切到「未检测到手柄，3 秒后隐藏」并倒计时 → 2 秒非阻塞淡出后 `SW_HIDE`；点击/拖动卡片会立即取消倒计时并中止淡出；托盘显示后有 5 秒缓冲；
* 有手柄重新上报 → 自动 `SW_SHOWNORMAL`，不抢焦点；
* 手动显示/隐藏会清掉自动隐藏的「待命」状态，直到手柄再次活跃，避免卡片在用户眼前自动收起；
* 没有托盘图标（`TRAY_READY` 为假）时绝不隐藏，否则卡片会失联。

### 托盘图标

`controller-battery-widget/src/tray.rs` 用一个**独立线程 + 隐藏窗口**承接 `Shell_NotifyIcon` 的回调（不子类化 eframe 的窗口）：

* 左键切换卡片显示/隐藏，右键弹菜单（显示卡片 / 隐藏卡片 / 退出）；
* 每秒读一次 `collector::snapshot`，电量或状态变化时才重建 32×32 电池图标并 `NIM_MODIFY`；
* 隐藏/显示用 `ShowWindow`，而不是 `ViewportCommand::Visible`：egui 认为窗口一直在，恢复后才会正常重绘；
* 隐藏前要求 `TRAY_READY`（shell 真的持有图标），否则拒绝隐藏；图标丢失或 Explorer 重启（`TaskbarCreated`）时自动把卡片放回来；
* 退出时 `on_exit` → `tray::shutdown()` 发 `NIM_DELETE`，避免通知区留下幽灵图标。

## 发布

```powershell
cargo build --release
# 产物：target\release\controller-battery.exe
```

体积目标：单个 exe < 10 MB，零运行时依赖。

## 测试

* 协议解码有真实报文样本的单元测试（`cargo test -p controller-battery-core`）；
* HID/XInput 后端依赖真实设备，后续用 8BitDo 猎2代等硬件做手动验证。
