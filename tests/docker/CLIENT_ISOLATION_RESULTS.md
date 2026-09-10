# 客户端隔离及 Codex 修复验证

日期：2026-09-10。环境：macOS arm64，以及用户现有的 `debian` 容器（Debian 13 / aarch64）；Codex CLI 0.153.4。

## 配置隔离

- v4 配置分别保存 Claude `profiles`、Codex `codex.profiles`、Pi `pi.profiles`。
- v1–v3 共享目录迁移为独立副本，保留禁用状态；新配置不自动共享厂商。
- 验证同 ID 在三套配置中可具有不同名称、模型和地址；TUI 编辑 Codex 不改变 Claude/Pi；新客户端空目录不会读取 Claude 目录。
- Debian 实际 PTY 验证三个不同厂商名称随鼠标标签切换而显示，切换不修改文件。
- Codex 代理路由读取 Codex 范围；Pi 导入/应用/状态读取 Pi 范围；缓存独立，非 Claude 标签不触发 Claude 同步。

## 自定义模型元数据

CCSW 为应用的厂商生成 `model_catalog_json`，登记 `deepseek-v4-flash` 等实际模型 ID。CLI 接受该目录，三种 API 协议的真实 Codex 进程测试均不再出现 `Model metadata for ... not found`。测试包含固定临时文件读取、修改、验证和最终响应。订阅切换及断开操作恢复原目录设置。

参考：[OpenAI 官方配置说明](https://learn.chatgpt.com/docs/config-file/config-reference)。未指定上下文时暂用 128K，显式上下文优先，`[1m]` 为 1M；模型目录默认只声明文本及基本工具能力。

macOS 使用 `workspace-write` 沙箱。Debian 容器因不允许创建嵌套用户命名空间，内层 Codex 沙箱拒绝执行命令；隔离测试中设置 `CCSW_SMOKE_SANDBOX=danger-full-access` 后验证固定的临时文件操作。未修改宿主机或日常 Codex 沙箱设置。

## 本地真实订阅副本

本地 `~/.codex/auth.json` 和 `config.toml` 复制到 Debian 私有临时目录；仅运行导入、激活、账号识别、真实额度查询及断开恢复。

- 导入和激活通过。
- 额度查询成功，缓存时间已写入，账号错误字段为空。
- 原 Codex 配置恢复通过，复制的 live auth 内容未变。
- 源副本哈希未变，测试后容器内凭据副本已删除；未写入宿主机配置。

额度查询使用 `account/read(refreshToken=false)` 读取当前会话，避免每次查询都强制轮换有效令牌。本次只有一份真实账号，未验证两个真实账号之间的在线切换，也未完成一次新的浏览器 OAuth 授权；这些边界不等同于已覆盖。模拟账号回归覆盖双账号切换和登录 RPC。

## 自动化

macOS / Debian：138 项 Rust 测试通过；macOS 格式与 Clippy 全 targets/features 检查通过；Debian 33 项卸载安全测试通过。

复用脚本：`tests/docker/client_tabs_isolation.py`、`tests/docker/codex_current_login.py`、`tests/fixtures/codex_cli_smoke.py`。账号脚本通过 `CCSW_TEST_INPUT` 指定私有副本目录，`CCSW_TEST_BINARY` 指定测试二进制，不打印令牌、邮箱、账号 ID 或额度详情。
