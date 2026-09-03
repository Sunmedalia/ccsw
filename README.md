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

## 快捷键与层级化操作逻辑

CCSW 采用现代两层层级式（Two-Tier / Drill-Down）交互模型：启动后直接呈现 **Router 厂商列表首页**；选中厂商后按 `Enter` 或鼠标点击下钻进入 **厂商详情与模型管理子页面**；随时按 `Esc` 或点击顶部 `[‹ 返回]` 平滑回退到首页。

### 1. 厂商首页（Home View）

| 按键 | 动作 |
| --- | --- |
| `↑/↓`、`j/k` | 在厂商卡片列表中上下移动光标 |
| `Enter`、鼠标单击 | **进入该厂商**：下钻进入该厂商的模型列表与配置详情页 |
| `n` | 新建厂商路由配置 |
| `e`、`E` | 编辑当前选中的厂商基础配置（API Endpoint、Key/Token 与角色映射） |
| `d`、`Delete` | 删除当前选中的厂商路由（弹窗确认） |
| `r`、`t` | 自动探测网络连接并获取厂商最新模型目录 |
| `m`、`Space` | 切换会话模式（Resume 恢复上次会话 / New 开启全新会话） |
| `R` | **直接恢复会话**：以 Resume 模式直接拉起 Claude 并恢复该厂商最新会话 |
| `N` | **直接新建会话**：以 New 模式直接拉起独立的新 Claude 会话 |
| `p` | 将所有厂商的已启用模型聚合同步到 Claude `/model` |
| `P` | 打开后台代理与开机自启动管理面板（查看状态/端口/PID，支持 Start/Stop/开机自启） |
| `?` | 打开详细帮助面板（按 `Esc`、`q` 或 `Enter` 关闭） |
| `q` | 退出程序 |

### 2. 厂商详情与模型全量管理页（Provider View & Catalog Showcase）

进入厂商详情后，**直接全屏呈现全量模型目录与搜索栏，无需再按 `/` 打开多余弹窗**：

- **左侧模型全量目录**：
  - 顶部搜索框：按 `/` 或直接点击激活，输入关键字实时模糊筛选全量模型（Esc 清空搜索词或退出搜索）。
  - 全量模型列表：直观呈现 `◆` 默认模型、`◈` 系统依赖、`●` 已启用、`○` 未启用状态，以及 `1M` 扩展标记与 `[自定义]` 标签。
- **右侧独立展出卡片**：
  - **当前选中模型独立展出卡片（Selected Model Showcase）**：精美呈现光标选中模型的完整名称、Canonical 模型 ID、状态标识、上下文规格（200k / 1M）、来源与角色别名映射（Sonnet/Opus/Haiku），以及直观的操作快捷指南。
  - **厂商基础连接与配置卡片**：展示 Endpoint 路由地址、协议格式、Token 掩码、Claude /model 聚合统计及快捷操作按钮。

| 按键 | 动作 |
| --- | --- |
| `Esc`、点击顶部 `[‹ 返回]` | **返回厂商首页**：退出当前厂商详情，回退至首页列表（搜索中按 Esc 则优先清空并退出搜索） |
| `/` | **激活模型搜索框**：快速模糊过滤全量模型目录 |
| `↑/↓`、`j/k` | **浏览模型**：在全量模型目录中移动光标，右侧独立展出卡片实时同步展示当前模型详细属性 |
| `Space` | **切换启用状态**：一键切换当前高亮模型的启用/禁用（`●` 已启用 / `○` 未启用） |
| `d` | **设为默认**：将当前选中的模型设为该厂商的默认启动模型（`◆` 默认） |
| `1` | **1M 上下文切换**：一键为选中模型开启/关闭 `[1m]` 扩展长上下文规格 |
| `Enter` | **启动 Claude**：以当前选中的模型直接启动 Claude（若未启用将自动激活） |
| `m` | **切换启动模式**：在 Resume（恢复上次会话）与 New（新建独立会话）间切换 |
| `R` / `N` | **快捷拉起**：以选中模型直接恢复最新会话 / 新建独立会话 |
| `A` / `C` | **批量操作**：一键启用当前筛选出的全部模型 / 清空非必要启用模型 |
| `x`、`Delete` | **安全禁用/删除**：禁用选中模型（若为手动添加的自定义模型则弹窗确认删除） |
| `a` | 手动为当前厂商添加自定义模型 |
| `E` | 编辑当前厂商的基础 Endpoint、密钥与角色映射 |
| `p` | 同步所有厂商模型到 Claude |

### 3. 表单与输入框操作

- **行内光标编辑**：文本字段支持 `←` / `→` 逐字移动光标，`Home` / `Ctrl+A` 跳到行首，`End` / `Ctrl+E` 跳到行尾，`Delete` 向后删除，`Backspace` 向前删除，`Ctrl+U` 一键清空。
- **流程化跳转**：在文本字段中按 `Enter` 自动跳至下一个字段；在最后一个字段按 `Enter` 直接提交保存。
- **选项切换**：在单选/下拉选项或开关字段中，按 `Space`、`Enter` 或左右键切换选项。
- **快速保存/取消**：随时按 `Ctrl+S` 保存表单，按 `Esc` 取消并返回。

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

这些操作也可以全部在 TUI 的 Proxy 管理页完成。`Sync all` 会自动启动后台代理，但不会自动安装开机启动；需要在 Proxy 页点击 `Enable at login`。安装完成后可以退出 TUI，代理由 macOS launchd 或 Linux systemd user service 管理。

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
