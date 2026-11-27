# DualSense 电量协议笔记

本文记录本项目在 Windows 上实测 DualSense（PID `0x0CE6` / Edge `0x0DF2`）电量的过程与结论，供后续维护和二次开发参考。

## 1. 电量字段

输入报告里的 `status[0]` 字节：

| 位 | 含义 |
| --- | --- |
| 0–3 | 电量档位 `0..=10`（每档 10%） |
| 4–7 | 充电状态 |

充电状态取值（与 Linux `hid-playstation.c` 一致）：

| 值 | 含义 |
| --- | --- |
| `0x0` | 放电中 |
| `0x1` | 充电中 |
| `0x2` | 已充满 |
| `0xA` | 电压或温度超范围（不充电） |
| `0xB` | 温度错误 |
| `0xF` | 充电错误 |

`status[0]` 的位置：

| 传输 | 报文字节数 | 报告 ID | 偏移 |
| --- | --- | --- | --- |
| USB | 64 | `0x01` | 53 |
| 蓝牙 | 78 | `0x31` | 54（蓝牙帧在 ID 后多一个字节） |

> 蓝牙偏移 54 与 Linux 内核 `drivers/hid/hid-playstation.c`（`&data[2]` + `status[0]`）以及第三方实现 `crystal-hidapi/samples/dualsense_battery.cr`（`buf[54]`）一致。

## 2. 蓝牙的两种报文（重要）

同一只手柄、同一个物理状态（未插线），实测读到两种 78 字节报文：

| 报告 ID | `status[0]` | 解读 |
| --- | --- | --- |
| `0x31` | `0x02` | 25%、放电中 ✅ |
| `0x01` | `0x12` | 25%、充电中 ❌ |

`0x01` 是“简化报文”：结构与 `0x31` 相同（触点、陀螺仪、计数器都能对上），但**电池/充电字段不刷新**，会一直停在某个时刻的值——插拔充电线也不会改变它。

复现记录：

* 90 秒内连续采样 1943 条报文，78 个字节完全一致（手柄静止）；
* 同一时间段内 `0x31` 报文的 `status[0]` 为 `0x02`（放电），而 `0x01` 报文为 `0x12`（充电）；
* 充电线拔掉 30 秒后，活跃上报的 `0x01` 报文仍报 `0x12`。

因此解码器把 78 字节的 `0x01` 标记为 `degraded`，UI 不应把它当作实时的充电状态展示。

### 如何让手柄回到全量报文

实测发现：**只要宿主以「写」权限打开设备**（`CreateFile` 请求 `GENERIC_READ | GENERIC_WRITE`），并发送一条 `valid_flag` 全 0 的 `0x31` 输出报文，手柄就会切到 `0x31` 全量报文，且该状态在后续重新打开时保持。

切回前后同一只手柄（充电中）的读数：

```
简化报文 0x01 → 35%（旧值）、充电中（degraded）
全量报文 0x31 → 65%（实时）、充电中
```

本项目因此默认以读写方式打开 DualSense，发现简化报文时补发一次空操作输出报文（不改灯、不振动）。

## 3. 空闲停止上报

蓝牙下手柄并非持续以固定频率上报：手柄静止一段时间后停止发送，Windows 会把最后一条报文原样返回给读取方。判断“还在不在刷新”只能靠报文内容指纹（本项目取整条报文的 FNV-1a 哈希），超过阈值未变化即视为空闲，显示“最后一次读数”。

推论：**插拔充电线不会触发上报**，需要一次按键/摇杆动作才会刷新读数。

## 4. 精度

电量是 11 档（0–10）的粗值，硬件层面没有更细的百分比。本项目取档位中点（`档位 × 10 + 5`）显示，例如档位 2 → 25%，真实区间是 20–30%。

想更准只能等更细的字段（当前固件未提供），或用库仑计等外部手段，软件层面做不到 1% 精度。

## 5. 8BitDo Ultimate 2 Wireless

实测这台手柄（VID `0x2DC8`）在同一只接收器上会以三种身份出现，取决于开机时有没有按着键（固件 1.06 / 接收器 1.04 起支持）：

| 模式 | USB PID | 系统视角 | 电量 |
| --- | --- | --- | --- |
| XInput（正常开机） | `0x310B` | XInput 槽位 + 若干 HID 接口 | 无 |
| DInput（开机时按住 `B`） | `0x6012` | HID 手柄（34 字节报文） | 有，精确百分比 |
| Switch（开机时按住 `Y`） | `0x6012` | HID 手柄（Pro Controller 报文） | 无意义，固定 55% |
| 蓝牙 | `0x6012` | BLE HID + Battery Service | 有，精确百分比 |

> 模式由手柄在开机瞬间决定，接收器只负责转发；想换模式必须**先关机**（长按 Home 3 秒），再按住目标键按 Home 开机。在底座上拿起来不会切换。

DInput 模式的外观和 Steam 里显示的「USB 8Bitdo Ultimate 2 Wireless Controller」一致：它只是一个普通 HID 手柄，**不会出现 XInput 槽位**。仅轮询 XInput 的程序因此完全看不到它——这正是「Steam 能识别、电量卡片空白」的原因。

### DInput 报文的电量字节

34 字节输入报告（报告 ID `0x01`），第 14 字节：

| 位 | 含义 |
| --- | --- |
| 0–6 | 电量百分比 `0..=100` |
| 7 | 充电中标志 |

读数等于 100 时按「已充满」处理（不再区分充电标志）。这份布局与 SDL 的 [`SDL_hidapi_8bitdo.c`](https://github.com/libsdl-org/SDL/blob/main/src/joystick/hidapi/SDL_hidapi_8bitdo.c)（`HIDAPI_Driver8BitDo_HandleStatePacket`，`data[14] >> 7` 与 `data[14] & 0x7f`）一致；第 15 字节起是加速度计/陀螺仪的 16 位小端数据。

实测报文（手柄静止、放电中，`0x62` → 98%）：

```
01 0F 7F 7F 7F 7F 00 00 00 00 00 00 00 00 62 43 FE F8 FF C6 0F FE FF 0F 00 EB FF ...
                                          ^^ 电量字节
```

固件 1.02 只发 12 字节报文，没有电量字段；本项目要求报文长度 ≥ 34 才解码，短报文直接忽略，不做猜测。

### XInput 模式为什么没有电量

* XInput 只返回 `batteryType=wired`、`batteryLevel=full`，没有真实电量；
* 厂商接口（usage page `0xFF7A`）只有在发送 `0x81` 请求后才会回 `0x02` 响应；
* 根据公开的逆向资料（[ascendedent/8bitdo-Linux-Software](https://github.com/ascendedent/8bitdo-Linux-Software)），`0x81` 的命令集只有升级、identify、振动、RF 地址读写和产品字符串选择；Ultimate 2 额外有一段 1592 字节的配置镜像读写（`81 04 ...`），内容是人形映射、死区等设置，**没有电量命令**。

所以想在 2.4G 下看电量，就按住 `B` 开机走 DInput；想要原生 XInput（不依赖 Steam Input）则正常开机，此时只能显示「未报告电量」。蓝牙模式下的电量来自 Windows 的 `DEVPKEY_Bluetooth_Battery`（BLE Battery Service），与本项目读到的数字一致。

## 6. DualShock 4（PS4，待真机验证）

支持 PID `0x05C4`（初代）、`0x09CC`（Slim / Pro）和 `0x0BA0`（官方无线接收器），按 Linux `drivers/hid/hid-playstation.c` 实现：

* **USB**（报告 ID `0x01`，64 字节）：状态字节在偏移 **30**；
* **蓝牙全量**（报告 ID `0x11`，78+ 字节）：状态字节在偏移 **32**；蓝牙连接后读取一次 `0x02` Feature Report 可从简化 `0x01` 切到全量 `0x11`。
* 状态字节：`bit 4` 表示接入 USB 线；低 4 位在未插线时为 `0..=10`（放电），插线时 `0..=9` 为充电中、`10` 为 100% 充电中、`11` 为已充满。

## 7. Nintendo Switch Pro Controller / Joy-Con（待真机验证）

支持任天堂（VID `0x057E`）Pro Controller（`0x2009`）与 Joy-Con L/R（`0x2006` / `0x2007`），按 Linux `drivers/hid/hid-nintendo.c` 实现：

* **模式初始化**：蓝牙默认发送简化报文 `0x3F`（不含电量），收到 `0x3F` 时需以写权限发送 `0x01` 输出报告（中性振动字节 + 子命令 `0x03 0x30`）切换到标准全量模式 `0x30`；USB 连接需先发送 `0x80 0x02`（握手）和 `0x80 0x04`（禁用超时并开启 HID 上报），再发 `0x03 0x30`。若设备被其他程序占用导致无写权限，则回退只读监听（依赖对方保持 `0x30` 模式）。
* 标准输入报告（报告 ID `0x21`、`0x30`、`0x31`，长度 ≥ 13 字节）的第 2 字节高 4 位（`report[2] >> 4`）包含电量；
* `bit 0`（`report[2] & 0x10`）为充电标志，`bits 1..=3`（`(report[2] >> 5) & 0x07`）为 `0..=4` 五档电量（空 / 低 / 中 / 高 / 满）。

## 8. 参考

* Linux 内核 `drivers/hid/hid-playstation.c`（DualSense / DualShock 4 驱动）
* Linux 内核 `drivers/hid/hid-nintendo.c`（Switch Pro / Joy-Con 驱动）
* `crystal-hidapi` 的 `dualsense_battery.cr` 示例（`buf[54]`）
* [ascendedent/8bitdo-Linux-Software](https://github.com/ascendedent/8bitdo-Linux-Software)（8BitDo 协议逆向笔记）
* 本项目 `crates/controller-battery-core/src/` 各协议模块实现与单元测试
