# Windows 验证记录

## v0.1.10 原生 CI 补充（2026-09-16）

[原生验证运行](https://github.com/Sunmedalia/ccsw/actions/runs/35046297299) 对应提交 `8700d5ed8edced0d6fb9c02c54bc5ccce98d5169`。Windows Server 2022 的 185 项测试全部通过（153 单元、11 Codex、5 隔离、5 Pi、3 卸载、8 Windows 集成），包括 junction、长路径、实际快捷方式、锁文件和进程树清理；真实 npm `.cmd` shim 也通过。Rust 1.88、macOS/Linux 测试、卸载安全套件及安全审计通过。原生 release 构建、PE 导入检查、ZIP 打包和解压后运行验证也通过。

本次原生验证发现并修复 Windows 强制字节范围锁导致的卸载读取失败，同时升级 rustls 至 0.23.45 以修复 RUSTSEC-2026-0285。Git 推送认证可用，GitHub CLI 可通过同一凭据访问 CI；下方初始记录中的认证和原生 CI 限制已解除。Windows 10/11 桌面交互、真实认证、注销登录自启动和实际 UNC 共享仍需桌面验收。

## 初始交叉构建记录（保留历史）

日期：2026-09-15。版本：0.1.8。基础提交：`afd58f0034f5390bebe53c7664dce1de34e914c5`。
本记录对应包含 Windows 适配的未提交工作区；基础提交本身不能复现此构建。

## 已完成

| 环境 | 结果 |
| --- | --- |
| macOS arm64，Rust 1.95 | fmt、Clippy（警告视为错误）、全部 167 项测试通过；实际 PTY 启动和 Ctrl+C 后终端恢复通过 |
| Linux Docker | fmt、Clippy、全部 167 项测试通过；安全套件 root 33 项通过，普通用户 32 项通过、1 项因需要 root chown 跳过 |
| Windows MSVC 交叉编译 | Rust 1.88 最低版本检查、当前工具链 Clippy、全部测试编译链接和 release 构建通过 |
| Windows PE 检查 | AMD64、控制台子系统、长路径 manifest；静态 CRT，无外部 VC runtime DLL 依赖 |
| Wine 10 / Linux amd64 | 153 项单元测试、11 项 Codex、5 项 Pi、2 项卸载、5 项隔离、7 项 Windows 集成测试通过，共 183 项 |
| ZIP 解压后 Wine 冒烟 | version/help、中文和特殊字符路径、代理启动/停止两轮、重复卸载通过 |
| PowerShell 7 | 三份脚本语法解析通过；原生执行待 CI 验证 |

Wine 回归包括 `.cmd` RPC、参数转义、环境变量、Credential Manager 合成凭据、超时清理子进程树、实际 `.lnk` 启动，以及超过 260 字符的状态路径。测试使用隔离配置和合成数据。
Wine 无法创建测试所需的 junction，故此环境排除该项；原生 Windows CI 保留此测试，没有放宽产品检查。

## 构建产物

`target/dist/ccsw-windows-x86_64.zip` 仅包含 `ccsw.exe`、`README-Windows.md` 和 `LICENSE`。
同目录的 `.zip.sha256` 提供校验值。已检查 ZIP CRC 和解压后 EXE 与 release EXE 的 SHA-256 一致。
此包在 macOS 使用 cargo-xwin 0.23.1、Rust 1.95.0 构建，目标为 `x86_64-pc-windows-msvc`。

## 原生复现

在 Windows PowerShell 7、Visual Studio C++ Build Tools、Rust 和 Node/npm 环境中运行：

```powershell
cargo fmt -- --check
cargo clippy --locked --target x86_64-pc-windows-msvc --all-targets --all-features -- -D warnings
cargo test --locked --target x86_64-pc-windows-msvc --all-targets --all-features
cargo +1.88.0 check --locked --target x86_64-pc-windows-msvc --all-targets --all-features
.\scripts\npm-shim-smoke.ps1
cargo build --locked --release --target x86_64-pc-windows-msvc --bin ccsw
.\scripts\package-windows.ps1
```

CI 已配置 Windows Server 2022 原生测试、最低 Rust 版本检查、真实 npm shim、打包和解压冒烟。当前 GitHub CLI 凭据失效，未触发或确认远程运行，未发布 release。

## 尚待原生验收

Wine 结果是补充证据，不能替代原生 Windows 验证。仍需执行上述 CI，并在 Windows 10/11 桌面确认：

- Windows Terminal / ConPTY 的 TUI 输入、窗口缩放和退出恢复。
- 浏览器认证往返、真实客户端和 Credential Manager 的持久化行为。
- 注销再登录后的 Startup 快捷方式启动，以及关闭终端后代理继续运行。
- 原生 junction 拒绝测试、真实 npm 生成的 shim、文件占用替换和进程树清理。
- 实际 UNC 共享的路径、锁和权限，以及系统长路径策略组合。

以上未完成项目不计入已通过结果。
