# CCSW

[![CI](https://github.com/Sunmedalia/ccsw/actions/workflows/ci.yml/badge.svg)](https://github.com/Sunmedalia/ccsw/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/Sunmedalia/ccsw)](https://github.com/Sunmedalia/ccsw/releases/latest)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

CCSW 是 Claude Code 的多厂商、多模型终端管理器。每次启动的 Claude 进程都使用独立的 Endpoint、凭据和模型映射，因此不同终端不会互相覆盖配置。

它提供三个核心能力：

- 在 TUI 中管理厂商、模型目录、默认模型、角色别名与 1M 上下文。
- 将所有已启用模型聚合到 Claude 原生 `/model`，并实时同步启用状态。
- 把 Anthropic Messages 请求转发到 Anthropic、OpenAI Chat Completions 或 Responses 兼容网关。

## 安装

### 下载 Release

当前提供 macOS Apple Silicon 与 Linux x86_64 二进制：

```sh
# macOS Apple Silicon
curl -L https://github.com/Sunmedalia/ccsw/releases/latest/download/ccsw-macos-arm64.tar.gz | tar -xz

# Linux x86_64
curl -L https://github.com/Sunmedalia/ccsw/releases/latest/download/ccsw-linux-x86_64.tar.gz | tar -xz

chmod +x ccsw
sudo install ccsw /usr/local/bin/ccsw
```

### 从源码安装

需要 Rust 1.88+：

```sh
git clone https://github.com/Sunmedalia/ccsw.git
cd ccsw
cargo install --path .
```

CCSW 支持 macOS 与 Linux，需要 Claude Code 2.1.242 或更高版本。

## 快速开始

```sh
ccsw doctor   # 检查 Claude、配置权限与网关连接
ccsw          # 打开 TUI
```

首次使用时：

1. 按 `n` 新建厂商，填写 API Endpoint、认证方式和默认模型。
2. 进入厂商详情，按 `r` 获取模型目录。
3. 用 `Space` 启用需要的模型；禁用只暂停模型，不会删除模型。
4. 按 `Enter` 直接启动 Claude，或按 `p` 将全部启用模型同步到 Claude `/model`。

首次运行若检测到 `~/.claude/settings.json`，CCSW 会显示脱敏导入预览。也可以手动执行：

```sh
ccsw import
ccsw import --yes
```

## TUI 导航

界面会随终端尺寸调整。宽窗口并排显示模型与详情；窄窗口改为单面板，并让厂商信息自动换行。任何主页面按 `?` 都会打开当前场景对应的 Help：

- `←/→` 或 `Tab`：切换 Home、All Enabled、Provider、Forms 分区。
- `↑/↓`：滚动当前帮助内容。
- `1`–`4`：直接打开对应分区。
- `Esc`、`q`、`?` 或 `Enter`：关闭 Help。

状态标记：`●` 已启用、`○` 已禁用、`◆` 默认模型、`◈` 角色依赖模型。

### Home · 厂商首页

首页第一行是 **All Enabled**，其后是所有厂商。

| 按键 | 操作 |
| --- | --- |
| `↑/↓`、`j/k` | 选择 All Enabled 或厂商 |
| `Enter`、鼠标单击 | 打开选中项 |
| `Space` | 启用/禁用当前厂商，并同步 Claude `/model` |
| `n` / `e` / `d` | 新建 / 编辑 / 删除厂商 |
| `r`、`t` | 测试连接并刷新模型目录 |
| `m` / `R` / `N` | 切换启动模式 / 恢复会话 / 新建会话 |
| `p` / `P` | 同步全部模型 / 打开代理管理器 |
| `?` / `q` | 帮助 / 退出 |

禁用厂商后，它的配置、模型和代理路由立即失效；其模型也会从 All Enabled 与 Claude `/model` 中移除。重新启用厂商会恢复其模型状态。

### All Enabled · 全部模型

该页面聚合所有已启用厂商的已配置模型。已禁用模型仍会保留在列表中，方便再次启用。

| 按键 | 操作 |
| --- | --- |
| `↑/↓`、`j/k` | 跨厂商选择模型 |
| `PgUp/PgDn`、`Home/End` | 翻页或跳转首尾 |
| `Space` | 启用/禁用模型，并实时同步 Claude `/model` |
| `Enter` | 打开模型所属厂商 |
| `Esc` | 返回首页 |

### Provider · 厂商与模型

厂商详情页直接展示完整模型目录、搜索框和当前模型信息，不再需要额外的模型管理弹窗。

| 按键 | 操作 |
| --- | --- |
| `↑/↓`、`j/k` | 浏览模型；窄窗口用 `Tab` 切换模型与详情面板 |
| `/` | 搜索模型；搜索中按 `Esc` 清空或退出搜索 |
| `Space` | 启用/禁用模型 |
| `d` / `1` | 设为默认模型 / 切换 `[1m]` 上下文 |
| `Enter` / `R` / `N` | 运行选中模型 / 恢复会话 / 新建会话 |
| `A` / `C` | 启用筛选结果 / 清空非必要启用项 |
| `a` | 添加自定义模型，并选择是否立即启用 |
| `x`、`Delete` | 删除自定义模型；网关模型不能删除 |
| `E` / `r` / `p` / `P` | 编辑厂商 / 刷新目录 / 同步 / 代理 |

### Forms · 表单

| 按键 | 操作 |
| --- | --- |
| `↑/↓`、`Tab/Shift+Tab` | 切换字段 |
| `Enter` | 确认选项或进入下一字段；最后一项直接保存 |
| `←/→`、`Home/End` | 移动文本光标 |
| `Backspace/Delete`、`Ctrl+U` | 删除字符 / 清空字段 |
| `Space`、`←/→` | 切换开关或选项 |
| `Ctrl+F`、`Ctrl+R` | 在自定义模型表单中，从厂商 API 刷新可选模型 |
| `Ctrl+S` / `Esc` | 保存 / 取消 |

## 模型状态规则

CCSW 将“模型存在”和“模型启用”分开处理：

- 添加或发现模型会把它放入目录；未启用时模型仍然存在。
- `Space` 只切换启用状态，不删除目录项。
- `x` 或 `Delete` 只删除手动添加的自定义模型，并要求确认。
- `disabled_models` 记录显式禁用项，因此重启后不会被默认模型或角色引用意外重新启用。
- `model-a` 与 `model-a[1m]` 是同一个目录模型；`[1m]` 只表示上下文规格，导入和发现时不会生成重复项。

## Claude `/model` 同步

| 操作 | 是否修改 `~/.claude/settings.json` |
| --- | --- |
| TUI 中启动 Claude、`ccsw run` | 否。使用子进程环境、`--model` 和临时 `--settings` |
| 按 `p`、执行 `ccsw apply` | 是。先备份，再聚合所有已启用厂商和模型 |
| 在 Home 切换厂商 | 是。实时移除或恢复该厂商模型 |
| 在 All Enabled 切换模型 | 是。实时更新模型选择器 |

Claude 原生 `/model` 中按 `Enter` 可能写入 Claude 的全局默认模型，但不会改变 CCSW 以 `--model` 启动的实例。只想修改当前会话时，在 Claude 的模型选择器中按 `s`。

## 命令行

```sh
# 直接启动指定厂商和模型
ccsw run --profile local --model claude-sonnet-4-6

# 恢复指定会话，并向 Claude 透传参数
ccsw run --profile local --resume SESSION_UUID -- --permission-mode plan

# 启动 TUI，并向之后启动的 Claude 透传参数
ccsw -- --permission-mode plan --add-dir ../shared

# 同步全部已启用模型；local 的默认模型作为 Claude 初始默认值
ccsw apply --profile local

ccsw config path
ccsw doctor
ccsw proxy status
```

CCSW 自己管理 `--model`、`--resume`、`--continue`、`--session-id` 和 `--fallback-model`，不要把这些选项放在透传参数中。一个外部 `--settings` 可以透传，CCSW 会保留其内容并加入会话跟踪 Hook 与当前模型选择器。

## 配置

默认配置路径为 `~/.config/ccsw/config.toml`，可以用 `CCSW_CONFIG` 或 `XDG_CONFIG_HOME` 覆盖。

```toml
version = 2

[profiles.local]
name = "Local gateway"
enabled = true
base_url = "http://127.0.0.1:18080"
api_format = "anthropic"
default_model = "claude-sonnet-4-6"
subagent_model = "claude-haiku-4-5"
fallback_models = ["claude-haiku-4-5"]
enabled_models = ["claude-sonnet-4-6"]
disabled_models = ["claude-opus-4-7"]

[profiles.local.credential]
kind = "bearer"
value = "replace-me"

[profiles.local.aliases]
opus = "claude-opus-4-7"
sonnet = "claude-sonnet-4-6"
haiku = "claude-haiku-4-5"

[[profiles.local.models]]
id = "claude-sonnet-4-6"
label = "Sonnet 4.6"
description = "Daily coding"
```

`api_format` 支持 `anthropic`、`openai-chat` 和 `openai-responses`。Endpoint 可以填写服务根地址、带 `/v1` 的地址或完整生成端点，CCSW 会规范化路径并保留查询参数。

认证类型：

- `bearer`：`Authorization: Bearer`。
- `x-api-key`：`x-api-key` 请求头。
- `api-key`：字面量 `api-key` 请求头，适用于 Azure 类端点。
- `none`：无认证的本地网关。

版本 1 配置会在内存中自动迁移；旧配置缺少 `enabled` 时默认启用。下次保存后写为版本 2。

## 本地代理

同步到 Claude `/model` 时，CCSW 会启动仅监听 `127.0.0.1` 的后台代理。Claude 只获得本地随机 Token；真实上游凭据保留在 CCSW 配置中。

```sh
ccsw proxy start
ccsw proxy start --listen 127.0.0.1:19021
ccsw proxy status
ccsw proxy stop
ccsw proxy install     # 安装 launchd / systemd user 开机服务
ccsw proxy uninstall
```

默认地址是 `127.0.0.1:17321`。`Sync all` 会启动代理，但不会自动安装开机启动项。代理支持流式文本、图片、工具调用、usage、停止原因与 reasoning summary；`/v1/messages/count_tokens` 使用 OpenAI tokenizer 近似估算。

## 数据与安全

- 配置：`~/.config/ccsw/config.toml`
- 会话状态：`~/.local/state/ccsw/state.json`
- 模型缓存：`~/.cache/ccsw/models.json`

配置包含明文上游 Token，Unix 下强制使用 `0600` 权限。Token 不会写入状态、缓存或运行日志。配置、状态和缓存使用原子替换，配置与状态写入带文件锁，因此多个 CCSW 实例可以并行运行。

## 开发

```sh
cargo fmt -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets
cargo build --locked --release
```

CI 在 Ubuntu 与 macOS 上执行相同检查，并生成平台二进制。
