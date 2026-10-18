# Copilot instructions

本仓库的约定、命令与已知问题集中在根目录的 [AGENTS.md](../AGENTS.md)，请优先阅读。

要点：

- 只做 Windows 后端；协议解析与 UI 分离，改动协议层请补真实报文的单元测试。
- 提交前跑 `cargo fmt --all`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`。
- 不要在代码或文档中写死本机路径。
- DualSense 的蓝牙简化报文（78 字节 `0x01`）电量字段不刷新，不能当作实时值。
