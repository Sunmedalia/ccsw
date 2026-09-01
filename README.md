# CCSW

CCSW 是一个面向 Claude Code 的多网关、多模型 TUI。它让每个 Claude 进程使用独立的 Base URL、凭据和模型映射，不需要在多个终端之间反复覆盖 `~/.claude/settings.json`。

## 它解决什么

- 同时运行配置 A 和 B，两个 Claude 进程不会互相覆盖连接信息。
- 在 Claude 内使用原生 `/model`，只显示当前配置档可用的模型。
- 退出 Claude 后返回 CCSW，换一个网关或模型并恢复同一段会话。
- 用 TOML 和 TUI 共同管理默认模型、角色别名、子代理模型和回退链。
- 从 Anthropic 兼容网关的 `/v1/models` 自动发现模型，失败时保留缓存和手工模型。
- 将网关模型目录与启用列表分开；只有你打开的模型才会进入 CCSW 和 Claude `/model`。
- 一键把当前路由和启用模型同步到 Claude 全局设置，之后直接运行 `claude` 也能使用。
- 把 Claude Code 的 Anthropic Messages 请求转换到 OpenAI-compatible Chat Completions 或 Responses 端点。

普通的 `ccsw run` 和 TUI Launch 不会修改 `~/.claude/settings.json`，它们通过每个子进程独立的环境、`--model` 和临时 `--settings` 完成路由。只有显式点击 `Sync all` 或执行 `ccsw apply` 才会修改并备份全局设置。

## 要求与安装

- macOS 或 Linux
- Rust 1.88+（从源码安装时）
- Claude Code 2.1.242+

```sh
cargo install --path .
ccsw doctor
ccsw
```

当前也可以直接开发运行：

```sh
cargo run
```

## 快捷键

| 按键 | 动作 |
| --- | --- |
| `↑/↓`、`j/k` | 移动选择 |
| `Tab`、`←/→` | 切换 Routes、Enabled models 与 Route details |
| `Enter` | 在 Routes 进入模型选择；在 Models 启动 Claude；在 Route details 管理模型 |
| `N` | 使用当前路由新建会话 |
| `m`、`Space` | 切换 Resume/New 会话模式 |
| `/` | 打开模型管理并立即搜索 |
| `1` | 为当前模型切换 `[1m]` 上下文 |
| `p` | 保存并应用到 Claude 全局配置 |
| `n/e/d` | 新建路由、管理模型、删除路由 |
| `E` | 编辑 API Endpoint、API Key/Token 和角色映射 |
| `a/x` | 手动添加、删除模型 |
| `r` 或 `t` | 自动获取提供商模型并测试连接 |
| `A` | 在首页启用当前 Profile 的全部可用模型 |
| `p` | 在首页将所有 Profile 的已启用模型同步到 Claude `/model` |
| `?` | 帮助 |
| `q` | 退出 |
| 鼠标左键 | 选择配置/模型，点击底部开关和操作按钮 |
| 鼠标滚轮/触控板 | 滚动鼠标所在的配置或模型列表 |
| 滚动条 | 点击轨道跳转；按住左键拖动滚动 |

底部操作栏提供 Launch、Sync all、Resume/New、Test、Help 和 Quit 按钮；窄窗口还会显示 Routes/Models/Details 面板开关。宽窗口采用 Routes / Enabled models / Route details 三栏布局，模型列表占据最大的工作区域；窄窗口使用页签和上下分区，避免端点、模型名和操作按钮互相挤压。

Route details 底部直接提供 `[Enable all]`、`[Sync all → Claude]`、`[Manage models]` 和 `[Edit route]`。`Enable all` 会打开当前 Profile 的全部已发现和手动模型；`Sync all → Claude` 会把所有 Profile 当前启用的模型同步到 Claude。点击 `[Edit route]`，或者在主界面按 `E`，即可修改 API Format、API Endpoint、认证类型及 API Key/Token。API Format 和认证类型使用可循环选择控件，按 `Enter`、`Space` 或左右方向键切换，也可以鼠标点击。

在 Route details 上按 `Enter`，或者按 `/`、`e`，会打开同页模型管理器。Provider 和 Credential 只读显示；输入 `/` 搜索模型，使用 `↑/↓` 选择，`Enter`/`Space` 打开或关闭模型，`1` 切换 1M 上下文，`d` 设为默认模型，`Ctrl+S` 保存。`◆` 表示默认或角色映射所需模型，不能直接关闭；`●` 表示已启用，`○` 表示未启用。长列表右侧显示滚动条，支持滚轮和触控板。

模型管理器底部将操作分成两行，提供 `Enable`、`1M`、`Default`、`Fetch`（自动获取）、`Add`（手动添加）、`Save`、`Sync all`、`Edit route` 和 `Cancel`。`Sync all` 会先保存当前勾选，再把所有 Profile 的已启用模型合并写入 `~/.claude/settings.json`。Claude 连接统一的 CCSW 本地聚合代理；代理根据模型名把请求发往对应的 Anthropic、OpenAI Chat 或 OpenAI Responses API。其他 Claude 设置会保留，旧文件备份为 `settings.json.ccsw-backup`。

同步后可以退出 CCSW，直接运行 `claude`。原生 `/model` 会同时显示不同 API 的模型，例如 `ark::deepseek-v4-flash[1m]` 和 `openai::gpt-5`；显示标签包含 Profile 名称。`profile::model` 仅用于 CCSW 本地路由，代理转发时会恢复上游真实模型 ID。当前选中的 Profile 决定首次同步后的默认模型，但不会限制 `/model` 中可选择的其他提供商。

1M 开关通过模型 ID 的 `[1m]` 后缀持久化，例如 `glm-5.3-flash` 会变成 `glm-5.3-flash[1m]`。只有网关支持该上下文规格时才应开启。

手动添加模型时，表单也包含 `1M context [○ OFF]` 开关。使用 `Tab` 或方向键移动到该行，按 `Space`/`Enter` 切换，或者直接用鼠标点击。开启后 CCSW 会自动为 Model ID 添加 `[1m]`，不需要手动输入后缀。

高级表单中可以点击字段和 Save/Cancel 按钮，也可以继续使用 `Tab` 切换字段、`Ctrl+S` 保存、`Esc` 取消。

## 配置文件

默认路径是 `~/.config/ccsw/config.toml`。可通过 `CCSW_CONFIG` 或 `XDG_CONFIG_HOME` 覆盖：

```toml
version = 2

[profiles.local]
name = "Local gateway"
base_url = "http://127.0.0.1:18080"
api_format = "anthropic"
default_model = "claude-sonnet-4-6"
subagent_model = "claude-haiku-4-5"
fallback_models = ["claude-haiku-4-5"]
enabled_models = ["claude-sonnet-4-6"]

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

`api_format` 支持：

- `anthropic`：直接使用 Anthropic Messages-compatible API。
- `openai-chat`：通过 CCSW 本地代理转发到 `/v1/chat/completions`。
- `openai-responses`：通过 CCSW 本地代理转发到 `/v1/responses`。

API URL 可以填写服务根地址、已有 `/v1` 的地址或完整生成端点。CCSW 会规范化路径并保留查询参数。

认证类型支持：

- `bearer`：作为 `Authorization: Bearer` 使用。
- `x-api-key`：作为 `x-api-key` 使用。
- `api-key`：作为字面量 `api-key` 请求头使用，适用于 Azure 类兼容端点。
- `none`：本地无认证网关。

版本 1 配置会在内存中自动迁移：旧 `auth-token` 变为 `bearer`，旧 `api-key` 保留原来的 `x-api-key` 语义。下次保存时写成版本 2。

配置包含明文上游 Token，CCSW 在 Unix 上强制使用 `0600` 权限；上游 Token 不会写入状态、缓存或运行日志。OpenAI 转发代理会在私有状态文件中保存一个独立的本地 Token。

`enabled_models` 只保存额外打开的目录模型。默认模型、角色别名、子代理模型和回退模型始终保持启用，因此旧配置不需要迁移，也不会因为模型发现结果过多而全部启用。

首次运行会检测 `~/.claude/settings.json`。确认导入前只显示脱敏预览，也可以单独执行：

```sh
ccsw import
ccsw import --yes
```

## 命令行模式

绕过 TUI 直接启动：

```sh
ccsw run --profile local --model claude-sonnet-4-6
ccsw run --profile local --resume SESSION_UUID -- --permission-mode plan
```

在 TUI 启动时透传参数：

```sh
ccsw -- --permission-mode plan --add-dir ../shared
```

CCSW 自己管理 `--model`、`--resume`、`--continue`、`--session-id` 和 `--fallback-model`，这些参数不能放在透传区。一个外部 `--settings` 可以透传；CCSW 会保留其中的设置并加入会话跟踪 Hook 与当前模型选择器。

Claude 原生 `/model` 中按 `Enter` 仍可能写入 Claude 的全局默认模型；这不会影响由 CCSW 以 `--model` 启动的实例，但可能影响之后直接运行的裸 `claude`。只想改变当前会话时，在 `/model` 选择器中按 `s`。

其他命令：

```sh
ccsw config path
ccsw doctor
ccsw apply --profile local
ccsw proxy status
```

`ccsw apply` 与 TUI 的 `Sync all` 按钮效果相同，会同步全部 Profile；`--profile` 指定哪个 Profile 的默认模型作为 Claude 初始默认值。

## 多提供商转发代理

执行 Sync all 时，CCSW 会自动启动只监听 `127.0.0.1` 的后台代理。Claude 得到的是本地随机 Token；Anthropic 和 OpenAI-compatible 的真正上游凭据只由代理从权限为 `0600` 的 CCSW 配置读取，不会写进 Claude settings。

```sh
ccsw proxy start
ccsw proxy status
ccsw proxy stop
```

默认监听 `127.0.0.1:17321`。若端口冲突，可以先执行 `ccsw proxy start --listen 127.0.0.1:19021`，再重新同步。

同步后即使退出 CCSW，后台代理仍会运行，因此可以直接启动 `claude`。如需机器重启后自动恢复，显式安装用户服务：

```sh
ccsw proxy install
ccsw proxy uninstall
```

macOS 使用 launchd，Linux 使用 systemd user service；同步操作不会静默安装开机启动项。`proxy stop` 或 `proxy uninstall` 不会改写 Claude settings，已经同步到 Claude 的模型在代理停止期间会不可用。

协议转换支持流式文本、图片、function tools、并行工具调用、工具结果、usage、停止原因和 reasoning summary。无法无损转换的内容块会返回明确错误。`/v1/messages/count_tokens` 使用 OpenAI tokenizer 进行近似估算，因为 OpenAI-compatible 服务没有统一的等价计数接口。

`ccsw doctor` 会检查 Claude Code 版本与安装状态、配置权限，并逐个测试网关模型接口。可用 `CCSW_CLAUDE_BIN` 指定非默认的 Claude 可执行文件。

## 会话与并发

CCSW 为新会话生成 UUID，并用 Claude Code 的 `SessionStart`、`PostModelSwitch`、`SessionEnd` Hook 跟踪 `/clear`、`/resume` 和 `/model` 后的实际状态。状态保存在 `~/.local/state/ccsw/state.json`，模型缓存保存在 `~/.cache/ccsw/models.json`。

配置、状态和缓存采用临时文件原子替换；配置和状态写入带文件锁。因此多个 CCSW 实例可以同时运行。正在运行的 Claude 使用启动时的配置快照，之后修改档案不会改变它。
