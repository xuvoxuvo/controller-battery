# AGENTS.md

给在本仓库工作的 AI 代理（以及新加入的开发者）的项目说明。面向用户的文档见 [README](README.md)，版本变更见 [CHANGELOG](CHANGELOG.md)。

## 项目是什么

Windows 桌面小卡片，显示手柄电量。Rust + egui，单文件 exe（约 4.2 MB，带图标与 VERSIONINFO），无运行时依赖。

- `crates/controller-battery-core/` —— 协议与设备访问，无 UI，可单测
- `crates/controller-battery-widget/` —— egui 界面 + 采集线程

## 命令

```powershell
cargo test --workspace                     # 单元测试
cargo fmt --all                            # 格式化（CI 会检查）
cargo clippy --workspace --all-targets -- -D warnings
cargo build --release                      # → target/release/controller-battery.exe
```

真机调试用 examples（会访问真实设备）：

```powershell
cargo run -p controller-battery-core --example probe          # HID + XInput 概览
cargo run -p controller-battery-core --example bt_battery     # 蓝牙电池与连接状态
cargo run -p controller-battery-core --example hid_dump -- 2DC8 310B
cargo run -p controller-battery-core --example capture -- 2DC8 - 20
```

## 代码约定

- 面向用户的字符串用中文；注释可以中文，但标识符一律英文。
- 采集线程不能 panic：不要在后台线程用 `unwrap()` 处理设备返回值。
- 每个数据源有自己的「空闲阈值」（`DeviceState::stale_after`），不要共用。
- 新增手柄协议时，先写解码函数 + 真实报文的单元测试，再接采集线程。
- 不要在代码或文档里写死本机路径（如 `C:\App\...`）。

## 文档分工

- **README 只写用户视角**：是什么、支持哪些设备、怎么用、有什么限制、怎么构建、去哪儿看文档。实现细节、协议坑、排查步骤、目录结构不进 README。
- 各类内容各归其位：
  - 构建、真机调试、CI 与发布 → `docs/development.md`；
  - 使用中出问题的排查（托盘图标、电量不刷新、卡片找不到）→ `docs/troubleshooting.md`；
  - 协议报文与实测记录 → `docs/protocol.md`；
  - 架构 → `docs/architecture.md`；
  - 面向 AI/贡献者的约定与踩坑 → 本文件（AGENTS.md）。
- 同一个事实只写在最合适的那一层，其它地方用链接指过去。README 里删掉的「项目结构」不要加回来：它和 `docs/development.md` 的目录重复。
- README 图片先缩放再提交：卡片截图约 460 px 宽（`docs/images/widget.png`），图标网格类预览不超过 500 px 宽；图标预览按 `icon_glyph.rs` 的 32×32 几何重新生成，不要拿旧图缩放凑数。

## 已经踩过的坑（改代码前务必读）

### DualSense 有三种输入报文

| 报告 | 长度 | 电量字节 | 说明 |
| --- | --- | --- | --- |
| `0x01`（USB） | 64 | 53 | 正常 |
| `0x31`（蓝牙全量） | 78 | 54 | 正常，充电状态可信 |
| `0x01`（蓝牙简化） | 78 | 54 | **电量字段不刷新**，标记 `degraded`，不能当实时值 |

- 以**写权限**打开设备（`GENERIC_READ | GENERIC_WRITE`）并发送 `dualsense::noop_output_report()`，手柄会切到 `0x31` 全量报文，且状态保持；不要改回只读打开。
- 电量是 0–10 档位（每档 10%），显示档位中点（`档位 × 10 + 5`），这是硬件上限。
- 手柄空闲会停止上报，Windows 会重复返回最后一条报文；判断「是否还在刷新」只能靠整条报文的指纹（见 `Shared::prune` 与 `stale_after`）。

### XInput

- 只提供空/低/中/满四档；很多第三方手柄（8BitDo 2.4G、盖世小鸡等）无线连接时也报 `batteryType=wired`。这个值只代表「不上报电量」，**既不等于有线，也不等于 100%**：显示 `--` 与「未报告电量」。
- XInput 不提供设备名，名称来自同厂商 HID 的 product string（`hid::product_name`）。**不要把 Sony 的 HID 名称借给 XInput 槽位**，会导致名称串号。

### 蓝牙（BLE）

- 电量来自 Windows 的 `DEVPKEY_Bluetooth_Battery`（BLE Battery Service），与系统设置显示一致。
- 已配对但断开的设备仍保留 PnP 节点和上次电量，**必须**用 `BluetoothLEDevice.ConnectionStatus` 判断在线状态。
- `FromBluetoothAddressAsync` 的地址要按 48 位**大端**传入（`0xE417D8BC7BA4`），传小端会得到一个不存在的设备、状态恒为 `Disconnected`。
- 同一只手柄在 2.4G 模式下也保持蓝牙配对，`snapshot()` 里会按名称匹配隐藏多余的蓝牙行。

### 8BitDo Ultimate 2

同一只接收器有三种身份，由**开机时按住的键**决定（固件 1.06 / 接收器 1.04 起）：

- XInput（正常开机）：PID `0x310B`，XInput 只报 wired/full，**没有电量**；厂商通道（usage page `0xFF7A`，输出报告 `0x81`）只有升级/identify/振动/RF/产品字符串和配置镜像，也没有电量命令。
- DInput（开机时按住 `B`）：PID `0x6012`，普通 HID 手柄（34 字节报文），**第 14 字节**是电量：bit 7 = 充电中，bit 0–6 = 0–100，读数 100 视为已充满（依据 SDL `SDL_hidapi_8bitdo.c`）。这个模式**不会产生 XInput 槽位**，所以只轮询 XInput 的程序会完全看不到它。
- Switch（开机时按住 `Y`）：PID `0x6012`，电量恒为 55%，不要用来做电量依据。
- 蓝牙：走 BLE HID + Battery Service，可读精确百分比。

切换模式必须**先关机再按住键开机**；从底座上拿起来不会切换。固件 1.02 的 DInput 报文只有 12 字节、没有电量字段，`eightbitdo::decode_power_state` 要求长度 ≥ 34，短报文一律忽略。

## 界面

- 透明窗口：`clear_color` 必须返回全透明，否则圆角会出现深色方块。
- 窗口高度随设备数变化：需要 `with_resizable(true)`，否则 `ViewportCommand::InnerSize` 会被忽略。
- 圆角是超椭圆（`squircle_points`），不要换回普通圆弧。
- 设备名按**像素宽度**省略（`fit_width`），不要按字符数截断。
- **所有用户可见文案走 `src/i18n.rs`**：卡片、右键菜单、托盘菜单/悬停提示、气泡共用 zh-CN / zh-TW / en 三张表，不要在界面或托盘里写死字面量。加一门语言 = 加一张表 + `Language::ALL` + 一个 `#[serde(rename)]`；`i18n` 的测试会检查空字段和枚举覆盖。`Language::System` 读 Windows UI 语言（繁体子语言走 zh-TW）。
- 字体按语言选：zh-TW 先找 `msjh.ttc`，其余先找 `Deng.ttf`；始终保留一个 CJK 回退，因为设备名可能包含中文。切换语言后要重新 `install_fonts`，并让托盘在下一个 1 秒 tick 里刷新提示文本。
- **空闲自动隐藏在 `src/autohide.rs`**（纯状态机，有单测），由托盘线程的 1 秒定时器驱动，**不要**放进 `App::update`：卡片隐藏后 egui 可能不再重绘，只有托盘定时器保证还在跑。手动显示/隐藏必须调 `note_manual_show` / `note_manual_hide`，否则卡片会在用户眼皮底下自动收起；没有托盘图标（`TRAY_READY` 为假）时绝不自动隐藏。

## 图标与版本信息

- `src/icon_glyph.rs` 是托盘图标与 exe 图标的**唯一来源**（32×32 参考网格，按尺寸缩放）。`build.rs` 用 `mod glyph { include!("src/icon_glyph.rs") }` 复用它，所以这个文件里不能出现 Win32 或 crate 内部依赖。
- `build.rs` 生成多尺寸 `.ico`（16/32/48/256）和一份**纯 ASCII** 的 `.rc`（VERSIONINFO），再调用资源编译器：MSVC 用 `rc.exe`（先 PATH，再 `Windows Kits\10\bin\*\x64\rc.exe`），GNU 用 `windres`。找不到只发 `cargo:warning`，不会让构建失败。
- `.rc` 故意只用 ASCII 字符串：rc.exe 与 windres 的代码页处理不一致，中文很容易变乱码；中文文案留在界面里。
- exe 图标在 Explorer 里可能遇到浅色或深色背景，所以用中性灰描边（`OUTLINE_NEUTRAL`）；托盘图标才按系统主题切换描边。

## 托盘图标

用户侧的排查流程写在 `docs/troubleshooting.md`；下面这些是实现约束，改代码前先看。

- `tray.rs` 自己起线程 + 隐藏窗口跑消息循环；**不要**把 eframe 的窗口拿去做 `Shell_NotifyIcon` 宿主（winit 的 wndproc 不能改）。
- 显示/隐藏用 `ShowWindow`，**不要**用 `ViewportCommand::Visible`：egui 一旦认为窗口不可见，`ShowWindow` 之后可能不再重绘。已验证隐藏→显示后卡片内容仍在。
- 图标必须自适应主题（读 `HKCU\...\Themes\Personalize\SystemUsesLightTheme`），白色描边在浅色任务栏上完全看不见。
- `Shell_NotifyIconW` 返回 `ERROR_ACCESS_DENIED (5)` 时先看进程完整性级别：低完整性/沙箱进程发消息给 Explorer 会被 UIPI 拦掉，这不是代码问题。
- **隐藏必须有兜底**：`TRAY_READY` 记录 shell 是否真的持有图标，`tray::hide()` 在未就绪时拒绝隐藏（菜单项也会置灰）；`NIM_MODIFY` 失败或 Explorer 重启（`TaskbarCreated`）导致图标丢失时会自动 `ShowWindow` 把卡片放回来。否则用户会把卡片藏进一个不存在的托盘，只能杀进程。
- 注册失败时每 5 秒重试一次（`RETRY_EVERY_TICKS`），提示分两种：`tray::running_low_integrity()` 为真时是低权限/沙箱环境（要从资源管理器启动），否则是 shell 还没准备好。
- 排查托盘问题先看进程完整性：`tray::running_low_integrity()` 以及 `NIM_ADD` 日志里的 `low_integrity=`。资源管理器是 Medium(0x2000)，低完整性进程一定会被 UIPI 拒。
- **进程完整性可能是被 exe 所在目录的标签决定的**：如果构建目录带 `Mandatory Label\Low Mandatory Level`（`icacls <dir>` 可查，DSH 工作区就是），从那里启动的 exe 一律是 Low，跟用什么方式启动无关；复制到普通目录（副本不带标签）再运行即恢复 Medium。`scripts/build.ps1 -Install` 就是干这个的。
- 失败诊断写在 `%LOCALAPPDATA%\ControllerBattery\tray.log`（错误码、进程/shell 完整性、会话号）；沙箱内写这个文件会被拒，所以普通用户环境才有。
- 第一次隐藏会弹一次气泡「已隐藏到托盘」（`NIF_INFO`，每个进程只弹一次），提示去哪儿找。
- Windows 11 默认把新托盘图标折叠进「^」，这是系统行为，程序无法强制它出现在任务栏上。
- 退出走 `App::on_exit` → `tray::shutdown()`（`NIM_DELETE`），否则通知区会留下幽灵图标。

## 发布

推送 `v*` 标签会触发 `.github/workflows/release.yml`：构建、打包 zip、用 `softprops/action-gh-release` 发布。CI 在 `.github/workflows/ci.yml`。详见 [docs/development.md](docs/development.md)。
