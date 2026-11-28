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
- 设备枚举是权威来源：HID 采集线程枚举不到设备后，经过 `HID_GONE_GRACE`（1 秒）宽限就删行，既不会因一次枚举失败而闪烁，手柄断开后也能很快回到「未检测到手柄」界面。

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

### README 编写规则

- **固定顺序**：标题/徽章/一句话简介/截图 → 特性 → 支持的设备 → 快速开始 → 使用（卡片 → 托盘图标 → 空闲自动隐藏 → 多语言）→ 电量精度 → 已知限制 → 构建 → 文档 → 许可证。
- **短句优先**：一条 bullet 只讲一件事；超过两行的描述拆成二级列表，或用分号断句，禁止大段连续文字。
- **能列表不用段落**：操作步骤用有序列表，支持的设备、电量精度用表格，特性用 3–6 条短 bullet。
- **图片跟着相关小节走**：界面截图放开头，托盘图标预览放托盘小节；不要堆在文末。
- **不在 README 展开**：实现细节、协议字段、排查步骤、目录结构、CI 流程一律链接到 `docs/` 或本文件；排查入口就近放在对应小节（如托盘小节指向 `docs/troubleshooting.md`）。
- 新增用户可见功能时同步更新 README 的对应小节与「已知限制」，保持“用户视角”。

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

### 采集线程架构与多设备约束

- **HID 枚举与序列号缓存**：`collector.rs` 通过 `cached_hid_interfaces()`（450 ms TTL）和按路径缓存的 `SERIAL_CACHE` 共享 SetupAPI 扫描，避免多个采集线程各自频繁 `CreateFileW`。
- **多设备与 USB 优先**：DualSense、DualShock 4、Switch Pro/Joy-Con 均采用「发现线程 + 按路径读线程」结构，经 `dedup_prefer_usb` 按序列号去重并优先保留 USB（`owners` 映射在 USB 接管时让旧蓝牙读线程自动退出）。
- **指纹去重 + 心跳节流 + 权威存在性**：HID 读线程用 `model::report_fingerprint`（FNV-1a）过滤 Windows 静止时重复返回的相同报文，并以 500 ms 心跳节流减少加锁；所有权威数据源（HID 枚举 / XInput / BLE 连接）设 `authoritative = true`，空闲后标「空闲」但**不按 `ENTRY_TTL` 删除**，只由各自的断开检测（HID 经 `HID_GONE_GRACE` 1 秒宽限）移除。
- **XInput 回退名与 `FallbackName`**：`controller_product_names()` 排除 Sony、Nintendo 和 8BitDo DInput 接口，仅在单个 XInput 槽位且唯一匹配到 1 个 HID 名称时借用，其余走 `FallbackName` 在渲染时经 `i18n::device_name()` 本地化，不要在 `core` 或 `collector` 里写死中文设备名。
- **DualShock 4 / Switch Pro**：DS4 蓝牙打开后会读一次 `0x02` Feature Report 切到 `0x11` 全量报文；Switch Pro / Joy-Con USB 会发送 `0x80 0x02` / `0x80 0x04` 握手及 `0x03 0x30` 模式切换，蓝牙在收到 `0x3F` 简化报文时懒发一次 `0x03 0x30`（两个模块均按 Linux/SDL 规范实现，待真机实测验证）。

### XInput

- 只提供空/低/中/满四档；很多第三方手柄（8BitDo 2.4G、盖世小鸡等）无线连接时也报 `batteryType=wired`。这个值只代表「不上报电量」，**既不等于有线，也不等于 100%**：显示 `--` 与「未报告电量」。
- XInput 不提供设备名，名称来自同厂商 HID 的 product string（`hid::product_name`）。**不要把 Sony、Nintendo 或 8BitDo DInput 的 HID 名称借给 XInput 槽位**，会导致名称串号。

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
- **卡片右键菜单用原生 Win32 菜单**（`src/menu.rs`），不要换回 `response.context_menu`：egui 的弹出层被限制在卡片那 230×68 的窗口里，菜单会被裁掉，语言子菜单更是放不下。原生菜单自己处理屏幕边缘翻转和浅色/深色主题；菜单项结果以 `menu::Command` 返回，在帧末统一应用（隐藏、退出、窗口层级、语言、自动隐藏、开机自启动）。顺序是「隐藏到托盘 / 空闲时自动隐藏 / 开机自启动 / 窗口层级 / 语言 / 退出」；`can_hide` 为假时前两项一起置灰——自动隐藏也要靠托盘图标才能恢复，「开机自启动」与「窗口层级」始终可用。托盘右键菜单同样提供「开机自启动」「窗口层级」，方便卡片被置底遮挡时直接从托盘切回。
- **窗口层级与 `winit` 的 `ALWAYS_ON_BOTTOM` 坑**：`WindowLevel`（`AlwaysOnTop` / `Normal` / `AlwaysOnBottom`）记入 `settings.json`，默认置顶。`winit 0.30` 只在属性切换时调一次 `SetWindowPos(HWND_BOTTOM)`，并不会拦截 `WM_WINDOWPOSCHANGING`；点击、拖动、弹出原生菜单（`SetForegroundWindow`）或 `SW_SHOWNORMAL` 都会把卡片抬到普通窗口之上。因此置底模式下必须由 `tray::enforce_bottom_if_needed()`、显示/恢复使用 `SW_SHOWNOACTIVATE` 以及托盘 1 秒定时器在非前台时补一次 `HWND_BOTTOM` 来维持（不子类化 winit 的 wndproc）。
- **开机自启动在 `src/autostart.rs`**：只写 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` 的 `ControllerBattery` 值（命令为带引号的 `current_exe()`），不要用 HKLM 或计划任务；卡片与托盘菜单共用 `autostart::is_enabled()` / `set_enabled()`，读取失败按未启用处理。登录启动与手动启动一致，不传额外参数。
- **所有用户可见文案走 `src/i18n.rs`**：卡片、右键菜单、托盘菜单/悬停提示、气泡共用 zh-CN / zh-TW / en 三张表，不要在界面或托盘里写死字面量。加一门语言 = 加一张表 + `Language::ALL` + 一个 `#[serde(rename)]`；`i18n` 的测试会检查空字段和枚举覆盖。`Language::System` 读 Windows UI 语言（繁体子语言走 zh-TW）。
- 字体按语言选：zh-TW 先找 `msjh.ttc`，其余先找 `Deng.ttf`；始终保留一个 CJK 回退，因为设备名可能包含中文。切换语言后要重新 `install_fonts`，并让托盘在下一个 1 秒 tick 里刷新提示文本。
- **空闲自动隐藏在 `src/autohide.rs`**（纯状态机，有单测），由托盘线程的 1 秒定时器驱动，**不要**放进 `App::update`：卡片隐藏后 egui 可能不再重绘，只有托盘定时器保证还在跑。状态机**不缓存 hidden**，`tick` 用 `IsWindowVisible` 的真实可见性判断隐藏/显示；延迟是 `IDLE_BEFORE_HIDE`（3 s，叠加 1 s 空闲判定，XInput 例外为 2 s）。倒计时由 `tray::countdown()` 从共享的 deadline 与界面自己的时钟算出（托盘线程被菜单等阻塞时也不会漏掉 3-2-1 倒计时），期间卡片统一显示「未检测到手柄，N 秒后隐藏」。用户点击/拖动卡片时 App 调 `tray::note_user_interaction()`：立即取消倒计时，并置 `ABORT_FADE` 让已经开始的淡出回退到不透明，而不是等下一次 1 秒 tick 才检查焦点。手动显示后有 `MANUAL_SHOW_HOLD`（5 s）缓冲，手动隐藏会禁止自动回显（`auto_show_allowed`）。`FADING` 期间忽略托盘点击，避免动画中途翻转目标状态。第一次通过卡片右键菜单或自动隐藏时会在淡出完成后弹一次 `NIF_INFO` 气泡（`HINT_SHOWN`），提示图标可能折叠在「^」里、左键恢复、右键退出（从托盘本身点隐藏时不弹，用户已看到托盘图标）——改动提示文案时保持这三点。
- 显示/隐藏统一走 `tray.rs` 的非阻塞定时器淡入淡出（`start_fade_in` / `start_fade_out` + `FADE_TIMER_ID` 50 ms 步进）：把卡片改成 `WS_EX_LAYERED` 再用 `SetLayeredWindowAttributes` 做淡入淡出（淡出 40 步 × 50 ms = 2 s，淡入 6 步 = 300 ms），DWM 会整体合成，圆角/文字一起渐变；不在 UI 线程或 WndProc 里 `sleep`，避免冻结界面或消息循环，`SetTimer` 失败时立即完成并复位 `FADING`；由于 `winit` 的 `WindowFlags::apply_diff`（例如切换 `WindowLevel`）会重写 `GWL_EXSTYLE` 并清掉 `WS_EX_LAYERED`，`ensure_fade_ready` 必须检查实时 `GWL_EXSTYLE` 位而不能用一次性初始化标志，`set_alpha` 也要在失败时自动补回样式；卡片已可见时 `start_fade_in` 不把 alpha 归零（避免单实例唤醒闪烁）；图标丢失时的兜底 `restore_if_hidden` 必须强行取消进行中的淡出并 `ShowWindow`，绝不能被 `FADING` 拦截。不要再用裸 `ShowWindow` 替代常规动画，也不要清掉 `WS_EX_LAYERED`；淡出结束后 alpha 会恢复成 255，保证下次显示不透明。

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
- **管理员运行（高完整性）时**：Explorer 在 Medium，UIPI 会静默丢弃它发来的托盘回调，表现为图标在、点击和菜单（包括「退出」）全无反应。托盘窗口创建后必须 `ChangeWindowMessageFilterEx(hwnd, WM_TRAY, MSGFLT_ALLOW, None)`，并同样放行 `WM_SHOW_REQUEST` 和 `TaskbarCreated`。
- 退出走 `App::on_exit` → `tray::shutdown()`：在退出线程里**同步** `NIM_DELETE`，再通知托盘线程收尾；只靠托盘线程删除会和进程退出赛跑，通知区容易留下幽灵图标（强杀进程时无解，那是 Explorer 的行为）。**托盘菜单的「退出」不能只发 `WM_CLOSE`**：隐藏的 eframe 窗口可能永远不确认 CloseRequested（它需要一次重绘才生效），所以走 `remove_icon()` + `std::process::exit(0)`，先删图标再退进程。
- **单实例**：`single.rs` 用命名互斥量 `Local\ControllerBattery.SingleInstance` 判断是否已有实例；第二次启动通过 `FindWindowW("ControllerBatteryTrayWindow")` 找到托盘窗口、发 `WM_SHOW_REQUEST` 后退出。互斥量句柄要在 `main` 里活到进程结束，否则保护失效。

## 发布

推送 `v*` 标签会触发 `.github/workflows/release.yml`：构建、打包 zip、用 `softprops/action-gh-release` 发布。CI 在 `.github/workflows/ci.yml`。详见 [docs/development.md](docs/development.md)。
