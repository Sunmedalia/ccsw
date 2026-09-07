# CCSW

[![CI](https://github.com/Sunmedalia/ccsw/actions/workflows/ci.yml/badge.svg)](https://github.com/Sunmedalia/ccsw/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/Sunmedalia/ccsw)](https://github.com/Sunmedalia/ccsw/releases/latest)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

CCSW 是 Claude Code 的多厂商、多模型配置管理器。它负责维护 Endpoint、凭据、模型映射和本地协议代理，但不会启动 Claude；同步完成后，直接在自己的终端运行 `claude` 即可。

它提供三个核心能力：

- 在 TUI 中管理厂商、模型目录、默认模型、角色别名与 1M 上下文。
- 将所有已启用模型聚合到 Claude 原生 `/model`，并实时同步启用状态。
- 把 Anthropic Messages 请求转发到 Anthropic、OpenAI Chat Completions 或 Responses 兼容网关。

## 安装

### 下载 Release

当前提供 macOS Apple Silicon、Linux x86_64 与 Windows x64 二进制：

```sh
# macOS Apple Silicon
curl -L https://github.com/Sunmedalia/ccsw/releases/latest/download/ccsw-macos-arm64.tar.gz | tar -xz

# Linux x86_64
curl -L https://github.com/Sunmedalia/ccsw/releases/latest/download/ccsw-linux-x86_64.tar.gz | tar -xz

chmod +x ccsw
sudo install ccsw /usr/local/bin/ccsw
```

### Windows（PowerShell）

```powershell
Invoke-WebRequest https://github.com/Sunmedalia/ccsw/releases/latest/download/ccsw-windows-x86_64.zip -OutFile ccsw.zip
Expand-Archive ccsw.zip -DestinationPath "$env:LOCALAPPDATA\Programs\ccsw" -Force
& "$env:LOCALAPPDATA\Programs\ccsw\ccsw.exe"
```

可将 `%LOCALAPPDATA%\Programs\ccsw` 添加到用户 `Path`，之后在 Windows Terminal 中运行 `ccsw`。无需管理员权限。

配置默认位于 `%APPDATA%\ccsw\config.toml`，状态与缓存位于 `%LOCALAPPDATA%\ccsw\state`、`cache`；`CCSW_CONFIG` 和 XDG 路径覆盖仍然有效。Claude 设置默认使用 `%USERPROFILE%\.claude\settings.json`，优先遵循 `CLAUDE_CONFIG_DIR`。

代理面板按 `P` 打开，按 `e` 修改端口；启用登录自启会在当前用户 Startup 目录创建 `CCSW Proxy.lnk`。关闭 TUI 不会停止后台代理，使用面板 Stop 或 `ccsw proxy stop` 停止。移动可执行文件后，需要禁用并重新启用登录自启。更新前先停止旧代理，再覆盖程序文件。

### 从源码安装

需要 Rust 1.88+：

```sh
git clone https://github.com/Sunmedalia/ccsw.git
cd ccsw
cargo install --path .
```

CCSW 支持 macOS、Linux 与 Windows 10/11 x64，需要 Claude Code 2.1.242 或更高版本。

## 快速开始

```sh
ccsw doctor   # 检查 Claude、配置权限与网关连接
ccsw          # 打开 TUI
```

首次使用时：

1. 按 `n` 新建厂商，填写 API Endpoint、认证方式和默认模型。
2. 进入厂商详情，按 `r` 获取 API 模型目录；按 `a` 从候选列表添加模型，也可以手动填写模型 ID。
3. 用 `Space` 启用需要的已配置模型；禁用只暂停模型，不会删除模型。
4. 按 `p` 将全部启用模型同步到 Claude `/model`，退出 CCSW 后运行 `claude`。

首次运行若检测到 `~/.claude/settings.json`，CCSW 会显示脱敏导入预览。也可以手动执行：

```sh
ccsw import
ccsw import --yes
```

## TUI 导航

界面采用统一英文标签，会随终端尺寸调整。100 列及以上并排显示模型与详情；窄窗口改为单面板，并让厂商信息自动换行。最低可用尺寸为 40×12，小于该尺寸时显示调整提示，仍可按 `q` 或 `Ctrl+C` 退出。表单会滚动以保持当前字段和文本光标可见；凭据始终遮蔽显示。任何主页面按 `?` 都会打开当前场景对应的 Help：

- `←/→` 或 `Tab`：切换 Home、All Models、Provider、Forms 分区。
- `↑/↓`：滚动当前帮助内容。
- `1`–`4`：直接打开对应分区。
- `Esc`、`q`、`?` 或 `Enter`：关闭 Help。

状态标记：`●` 已启用、`○` 已禁用、`◆` 默认模型、`◈` 角色依赖模型。

### Home · 厂商首页

首页第一行是 **All Models**，其后是所有厂商。

| 按键 | 操作 |
| --- | --- |
| `↑/↓`、`j/k` | 选择 All Models 或厂商 |
| `Enter`、鼠标单击 | 打开选中项 |
| `Space` | 启用/禁用当前厂商；接入后自动同步 Claude `/model` |
| `n` / `e` / `x` | 新建 / 编辑 / 删除厂商 |
| `r`、`t` | 测试连接并刷新模型目录 |
| `A` | 启用选中厂商中的全部已配置模型；All Models 行不执行此操作 |
| `p` / `P` | 同步全部模型 / 打开代理管理器 |
| `?` / `q` | 帮助 / 退出 |

禁用厂商后，其模型会从 All Models 中移除，代理立即拒绝该厂商的请求；接入后也会自动更新 Claude `/model`。重新启用厂商会完整保留此前的模型启用、禁用状态，包括显式禁用的默认模型。

### All Models · 全部模型

该页面聚合所有已启用厂商的已配置模型。已禁用模型仍会保留在列表中，方便再次启用。

| 按键 | 操作 |
| --- | --- |
| `↑/↓`、`j/k` | 跨厂商选择模型 |
| `PgUp/PgDn`、`Home/End` | 翻页或跳转首尾 |
| `Space` | 启用/禁用模型；接入后自动同步 Claude `/model` |
| `Enter` | 打开已选模型所属厂商 |
| 鼠标单击 | 第一次选中模型，再次单击已选模型时打开所属厂商 |
| `Esc` | 返回首页 |

### Provider · 厂商与模型

厂商详情页展示已配置模型、搜索框和当前模型信息。API 发现结果保存在缓存中，可在添加模型表单中选择；发现模型不会自动添加或启用。

| 按键 | 操作 |
| --- | --- |
| `↑/↓`、`j/k` | 浏览模型；窄窗口用 `Tab` 切换模型与详情面板 |
| `/` | 搜索模型；搜索中按 `Esc` 清空或退出搜索 |
| `Space` | 启用/禁用模型 |
| `d` / `1` | 设为默认模型 / 切换 `[1m]` 上下文 |
| `A` / `C` | 启用筛选结果 / 清空非必要启用项 |
| `a` | 添加自定义模型，并选择是否立即启用 |
| `x` | 删除自定义模型；网关模型不能删除 |
| `E` / `r` / `p` / `P` | 编辑厂商 / 刷新目录 / 同步 / 代理 |

### Forms · 表单

| 按键 | 操作 |
| --- | --- |
| `↑/↓`、`Tab/Shift+Tab` | 切换字段 |
| `Enter` | 确认当前字段并进入下一项；最后一项直接保存 |
| `←/→`、`Home/End` | 移动文本光标 |
| `Backspace/Delete`、`Ctrl+U` | 删除字符 / 清空字段 |
| `Space`、`←/→` | 切换开关或选项 |
| `Ctrl+F`、`Ctrl+R` | 在自定义模型表单中，从厂商 API 刷新可选模型 |
| `Ctrl+S` / `Esc` | 保存 / 取消 |

模型表单中，`Tab`/`Shift+Tab` 会依次遍历字段和 API 搜索面板；窄窗口按焦点显示面板。搜索时按 `Esc` 先清空搜索，再退出搜索，最后关闭表单。`Ctrl+S` 在搜索面板中也能保存。

模型刷新、代理管理和同步在后台执行，等待时仍可导航。重复刷新同一厂商会合并提示；过期请求不会覆盖新表单或已修改的厂商配置。

## 模型状态规则

CCSW 将“模型存在”和“模型启用”分开处理：

- 添加或发现模型会把它放入目录；未启用时模型仍然存在。
- `Space` 只切换启用状态，不删除目录项。
- `x` 是统一的删除快捷键，只删除手动添加的自定义模型，并要求确认。
- `disabled_models` 记录显式禁用项，因此重启后不会被默认模型或角色引用意外重新启用。
- `model-a` 与 `model-a[1m]` 是同一个目录模型；`[1m]` 只表示上下文规格，导入和发现时不会生成重复项。

## Claude `/model` 同步

CCSW 不启动 Claude，也不接管 Claude 的会话参数。同步采用“首次手动接入，之后自动更新”：

- 首次编辑只保存 CCSW 配置。按 `p` 或执行 `ccsw apply --profile <id>` 成功后，建立与当前 Claude settings 文件的接入记录。
- 接入后，在任何页面修改厂商、模型启用状态、默认模型、角色或上下文规格，都会自动同步；连续修改会合并到最新状态。
- 自动同步沿用上次明确选择的默认厂商，不随浏览位置变化。厂商或默认模型不可用时，选择可用项；全部禁用时清空 CCSW 管理的模型，保留接入记录。
- 写入 Claude 前会备份 settings，并保留无关配置。同步失败时，本地修改仍然保存，按 `p` 重试；重新打开 TUI 时会检查未同步修改。
- 如果 Claude 的 Endpoint 或 Token 已被其他工具切换，自动同步暂停，按 `p` 才重新接入。

底部状态为 `Not connected`、`Pending`、`Syncing`、`Synced`、`Failed` 或 `Paused`。接入记录绑定 CCSW 配置路径和 Claude settings 路径，并在重启后保留。

同步完成后，从普通终端运行 `claude`，再使用原生 `/model` 选择 CCSW 管理的模型。在 `/model` 中按 `Enter` 可能写入 Claude 的全局默认模型；只想修改当前会话时按 `s`。

## 命令行

```sh
# 同步全部已启用模型；local 的默认模型作为 Claude 初始默认值
ccsw apply --profile local

ccsw config path
ccsw doctor
ccsw proxy status
```

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
ccsw proxy install     # 安装当前用户登录自启（launchd / systemd / Windows Startup）
ccsw proxy uninstall
```

默认地址是 `127.0.0.1:17321`；指定过自定义监听地址后，停止并重启会保留该地址。`Sync all` 会启动代理，但不会自动安装开机启动项。代理支持流式文本、图片、工具调用、usage、停止原因与 reasoning summary；`/v1/messages/count_tokens` 使用 OpenAI tokenizer 近似估算。

### 修改本地代理端口 / 多系统用户

root 与普通用户使用各自的配置和代理 Token，但同一台机器的监听端口是共用的。可以让 root 使用 `127.0.0.1:17321`，普通用户使用 `127.0.0.1:17322`，更多用户依次选择其他空闲端口。

TUI 中按 `P` 打开 Proxy：

1. 如果当前用户的代理正在运行，按 `x`（Stop）停止。
2. 按 `e` 或点击 `Port (e)`，输入端口，按 `Enter` / `Ctrl+S` 保存；`Esc` 取消。
3. 关闭 Proxy 面板，按主界面的 `p` 启动代理并同步 Claude 到新地址。

命令行也支持：

```sh
# 在需要更改端口的那个用户身份下执行
ccsw proxy stop            # 如果该用户的代理正在运行
ccsw proxy port 17322      # 保存新端口，要求 1–65535 且未被占用
ccsw apply --profile local # 换成自己的厂商 ID；启动代理并更新 Claude 地址
```

已有的 `ccsw proxy start --listen 127.0.0.1:17322` 仍然可用。端口保存在当前用户状态目录的 `proxy.json` 中，重启时保留。这里修改的是本地 Proxy 的 Listen，不是厂商 API 的 Base URL。已经运行的 Claude 需要重新启动以读取更新后的地址。端口冲突时会报错并保留原监听配置，不会停止其他用户的代理。

## 数据与安全

- 配置：`~/.config/ccsw/config.toml`
- 模型缓存：`~/.cache/ccsw/models.json`
- 代理状态与日志：`~/.local/state/ccsw/`
- 同步接入记录：`~/.local/state/ccsw/sync-state.json`（仅包含路径、默认厂商、变更标记和本地代理认证信息，不保存上游凭据）

配置包含明文上游 Token，Unix 下强制使用 `0600` 权限。Token 不会写入缓存或运行日志。配置和缓存使用原子替换并带文件锁。编辑会合并其他实例对独立字段的修改；同字段冲突会要求重新打开编辑器。文件正在被其他实例写入时，TUI 会提示重试，不会一直等待文件锁。

## 开发

```sh
cargo fmt -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets
cargo build --locked --release
```

CI 在 Ubuntu、macOS 与 Windows 上执行相同检查，并生成平台二进制。

TUI 按状态、事件、页面、表单、模型规则、布局和后台任务拆分在 `src/tui/`；CLI 与 TUI 共用 `src/sync.rs` 的同步服务。回归测试包含真实事件序列、延迟本地 API、同步失败恢复、并发编辑及 120×36 到 40×12 的布局检查。
