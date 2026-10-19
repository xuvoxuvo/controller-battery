# AGENTS.md

给在本仓库工作的 AI 代理（以及新加入的开发者）的项目说明。面向用户的文档见 [README](README.md)，版本变更见 [CHANGELOG](CHANGELOG.md)。

## 项目是什么

Windows 桌面小卡片，显示手柄电量。Rust + egui，单文件 exe（约 3.7 MB），无运行时依赖。

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

- 2.4G：XInput 只报 wired/full；厂商通道（usage page `0xFF7A`，输出报告 `0x81`）只有升级/identify/振动/RF/产品字符串和配置镜像，**没有电量命令**。
- 蓝牙：走 BLE HID + Battery Service，可读精确百分比。

## 界面

- 透明窗口：`clear_color` 必须返回全透明，否则圆角会出现深色方块。
- 窗口高度随设备数变化：需要 `with_resizable(true)`，否则 `ViewportCommand::InnerSize` 会被忽略。
- 圆角是超椭圆（`squircle_points`），不要换回普通圆弧。
- 设备名按**像素宽度**省略（`fit_width`），不要按字符数截断。

## 发布

推送 `v*` 标签会触发 `.github/workflows/release.yml`：构建、打包 zip、用 `softprops/action-gh-release` 发布。CI 在 `.github/workflows/ci.yml`。详见 [docs/development.md](docs/development.md)。
