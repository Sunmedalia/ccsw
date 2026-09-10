# CCSW

[![CI](https://github.com/Sunmedalia/ccsw/actions/workflows/ci.yml/badge.svg)](https://github.com/Sunmedalia/ccsw/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/Sunmedalia/ccsw)](https://github.com/Sunmedalia/ccsw/releases/latest)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

CCSW 是 Claude Code、Codex 与 Pi Agent 的多厂商、多模型配置管理器，支持保存和切换 Codex 订阅账号。它负责维护 Endpoint、凭据、模型映射和本地协议代理，但不会启动 Claude；同步完成后，直接在自己的终端运行 `claude` 即可。

它提供三个核心能力：

- 在 TUI 中管理厂商、模型目录、默认模型、角色别名与 1M 上下文。
- 将所有已启用模型聚合到 Claude 原生 `/model`，并实时同步启用状态。
- 把 Anthropic Messages 请求转发到 Anthropic、OpenAI Chat Completions 或 Responses 兼容网关。

> 本文对应 main 分支。模型 Token 参数、模型表单 `Alt+1` 和完整卸载功能尚未包含在 v0.1.4 中；使用这些功能请从源码安装。

[快速开始](#快速开始) · [快捷键](#tui-导航) · [Codex 配置与账号](#codex-配置与账号) · [Pi Agent 配置](#pi-agent-配置) · [模型参数](#模型-token-参数) · [同步](#claude-model-同步) · [端口设置](#修改本地代理端口--多系统用户) · [卸载](#卸载与配置清理) · [开发与测试](#开发)

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
3. 用 `Space` 启用模型；按 `e` 编辑输出 Token 上限等参数，按 `1` 切换 1M 标记。禁用只暂停模型，不会删除模型。
4. 按 `p` 接入 Claude：自动启动本地代理，并将全部启用模型写入 Claude 设置。之后保存的变更会自动同步。
5. 在终端运行 `claude`，使用 `/model` 选择模型。退出 CCSW 不会停止代理。

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
| `e` | 编辑当前模型的完整配置；模型列表和详情面板均可使用 |
| `E`（`Shift+e`） | 编辑当前厂商配置 |
| `r` / `p` / `P` | 刷新目录 / 同步 / 代理 |

### Forms · 表单

| 按键 | 操作 |
| --- | --- |
| `↑/↓`、`Tab/Shift+Tab` | 切换字段 |
| `Enter` | 确认当前字段并进入下一项；最后一项直接保存 |
| `←/→`、`Home/End` | 移动文本光标 |
| `Backspace/Delete`、`Ctrl+U` | 删除字符 / 清空字段 |
| `Space`、`←/→` | 切换开关或选项 |
| `Ctrl+F`、`Ctrl+R` | 在自定义模型表单中，从厂商 API 刷新可选模型 |
| `Alt+1` | 在模型表单任意字段或 API 搜索中切换 1M 标记 |
| `Ctrl+S` / `Esc` | 保存 / 取消 |

模型表单中，`Tab`/`Shift+Tab` 会依次遍历字段和 API 搜索面板；窄窗口按焦点显示面板。搜索时按 `Esc` 先清空搜索，再退出搜索，最后关闭表单。`Ctrl+S` 在搜索面板中也能保存。

模型刷新、代理管理和同步在后台执行，等待时仍可导航。重复刷新同一厂商会合并提示；过期请求不会覆盖新表单或已修改的厂商配置。

## Pi Agent 配置

> 源码版本功能，尚未包含在 v0.1.4 发布包中。使用源码构建的新二进制；当前兼容验证基准为 Pi `0.85.1`。

点击顶部 **Pi** 标签或按 `F2` 切换到 **Pi API**。厂商及模型目录与 Claude/Codex 共用，Pi 的接入和默认模型独立保存。

| 按键 | 操作 |
| --- | --- |
| `i` | 导入现有 Pi 自定义 API 厂商和模型；重复导入更新原导入项 |
| `e` | 厂商页面编辑厂商；模型页面编辑模型 |
| `E` | 编辑所属厂商 |
| `p` | 同步全部已启用厂商和模型，并将选中模型设为 Pi 默认值 |
| `s` | 查看磁盘配置状态、待同步或外部编辑冲突 |
| `D` | 断开管理，恢复仍属于 CCSW 的配置字段 |

首次按 `p` 后，TUI 中的厂商、模型修改会自动同步到 Pi。若当前默认模型被禁用，优先选择同厂商的可用默认模型，再选择其他启用模型；全部禁用时清除受管目录并恢复原默认值，接入记录保留。同步失败可按 `p` 重试。手动编辑 CCSW 文件后使用 CLI apply 或 TUI `p` 同步。

```sh
ccsw pi import --dry-run       # 预览导入，不执行凭据命令
ccsw pi import
ccsw pi apply --profile deepseek
ccsw pi apply --profile deepseek --model deepseek-v4-flash
ccsw pi status
ccsw pi disconnect
```

Pi **直接连接厂商**，不启动或依赖 CCSW 代理。三种格式对应 Pi 原生 `anthropic-messages`、`openai-completions`、`openai-responses`。API Key 会写入 Pi 配置，Unix 文件权限为 0600；字符串按 Pi 规则转义。输出上限由 Pi 客户端使用，不是代理强制限制。

- 目标目录遵循 `PI_CODING_AGENT_DIR`，默认 `~/.pi/agent`。写入 `models.json` 的 `ccsw-<厂商 ID>` 项，以及 `settings.json` 的 `defaultProvider` / `defaultModel`。
- 模型 ID 移除 `[1m]` 后缀；显式 Context window 优先，否则 `[1m]` 对应 1,000,000。Max output tokens 对应 Pi 的 `maxTokens`。其他能力使用 Pi 默认值，导入模型保留其兼容性、输入类型和推理能力配置。
- 导入读取自定义厂商和普通 API Key。OAuth 令牌、命令或环境变量凭据、内置模型覆盖、混合协议和每模型独立认证等不支持项目会列出跳过原因，不执行命令。导入同名 CCSW 厂商时创建独立 ID。
- 原有 Pi 厂商、主题、扩展、订阅登录、会话保留；不会修改 `auth.json`。原厂商与 `ccsw-` 项可同时出现在 Pi 中。本次不提供订阅多账号管理。
- 在 Pi 内重新打开 `/model` 可重新加载目录；启动默认值请在新 Pi 进程中确认。命令行、项目设置和扩展可能覆盖全局配置。
- 多文件写入保存事务日志。中断后重试 apply/disconnect 恢复；发生外部编辑冲突时不会覆盖。断开及卸载仅恢复仍与最后写入一致的受管字段。

验证真实 Pi 进程（隔离 HOME、本地模拟上游，不使用真实凭据）：

```sh
cargo build
SMOKE_FORMAT=anthropic python3 tests/fixtures/pi_cli_smoke.py
SMOKE_FORMAT=openai-chat python3 tests/fixtures/pi_cli_smoke.py
SMOKE_FORMAT=openai-responses python3 tests/fixtures/pi_cli_smoke.py
```

可使用 `CCSW_TEST_BINARY` 和 `CCSW_PI_BIN` 指定 CCSW / Pi 二进制。

## Codex 配置与账号

> 此功能属于源码版本，尚未包含在 v0.1.4 中。CLI 与 ChatGPT App 内的 Codex 使用同一套目标配置。CCSW 显示的是磁盘配置状态；真实 App 的账号切换与新会话请求仍需在目标版本上验证，不能将“已写入”视为 App 已生效。

在 TUI 中点击顶部 **Claude Code / Codex / Pi** 标签，或按 `F2` 循环切换，按 `F3` 切换 Codex 的 API Providers / Accounts。厂商及模型目录共用；Claude 和 Codex 分别保存接入选择。

### Codex API

1. 在 Codex 的 API Providers 页面添加或选择厂商。
2. 进入厂商页面选中模型；`e` 编辑模型、`E` 编辑厂商，`g` 设置推理强度。
3. 按 `p` 应用。CCSW 启动本地代理，并将 `model`、专属 `model_providers.ccsw` 等字段写入 Codex 配置。
4. 重启 Codex CLI / ChatGPT App，打开新会话确认模型和请求地址。已有会话不会迁移到新模型。

支持 OpenAI Responses、Chat Completions 和 Anthropic 厂商。Responses 上游直接转发；另外两种格式转换文本、图片（取决于上游能力）、函数工具、命名空间工具、自定义编辑工具与流式输出。无法转换的内容返回明确错误，包括跨协议的加密推理历史、`previous_response_id` 和托管工具；转换型厂商默认关闭 Codex 托管网页搜索。远程 `/responses/compact` 只转发给 Responses 上游，其他上游需客户端本地压缩。

模型 ID 写入时移除 Claude 专用 `[1m]` 后缀。`Max output tokens` 在代理侧限制实际输出，`Context window` 写入 Codex 上下文设置，并将自动压缩阈值设为容量的 90%。推理强度仍需所选上游模型支持。

```sh
ccsw codex apply --profile my-provider --model my-model --reasoning high
ccsw codex status
ccsw codex disconnect
```

### Codex 订阅账号

| 按键 | 操作 |
| --- | --- |
| `n` / `N` | 浏览器登录 / 设备码登录，完成后保存账号 |
| `i` / `I` | 导入本机当前登录 / 指定 `auth.json` 路径 |
| `↑↓`、`j/k` | 选择账号 |
| `e` | 重命名账号 |
| `p`、`Enter` | 切换到选中账号 |
| `r` | 刷新所选账号的套餐与额度 |
| `x` | 删除保存的账号；需先切换或断开当前使用账号 |
| `s` | 检查磁盘配置与登录冲突 |
| `D` | 断开接管并恢复之前的受管配置 |
| `?` | 帮助；`↑↓`、`PgUp/PgDn` 滚动 |
| `Esc` | 取消登录或返回 API 页面 |

新增账号在隔离目录中完成登录，不会自动切换当前账号。同一用户的不同工作区分别保存；重复导入相同身份会更新已有记录。重新登录失效账号时再次使用 `n`，完成相同身份登录即可更新凭据。

切换前保存当前账号最新凭据，再写入目标账号。允许客户端运行时切换，但需要自行退出并重启 CLI / ChatGPT App；旧进程可能继续使用原账号或回写登录状态，`s` / `ccsw codex status` 会报告身份冲突。此时重启客户端后重新应用。App 可能共用整个 ChatGPT 登录身份，因此切换可能影响 App 主界面的账号。

额度通过已安装 Codex 的 App Server 查询，展示其返回的套餐、各额度窗口使用比例及重置时间。失败时保留旧数据并标记过期；缺失信息显示未知。只查询选中的账号，不自动轮换账号，也不办理订阅购买、续费或取消。

```sh
ccsw codex accounts login --name personal
ccsw codex accounts login --name work --device
ccsw codex accounts import --name existing
ccsw codex accounts import --name backup --file /absolute/path/auth.json
ccsw codex accounts list
ccsw codex accounts use <account-id>
ccsw codex accounts refresh <account-id>
ccsw codex accounts rename <account-id> new-name
ccsw codex accounts remove <account-id>
```

### Codex 文件与恢复

- Codex 目标目录遵循 `CODEX_HOME`，默认 `~/.codex`。登录与额度查询调用 `codex`，可用 `CCSW_CODEX_BIN` 指定二进制路径。
- 保存的账号元数据位于 CCSW 配置；凭据副本位于 CCSW 状态目录的 `codex-accounts/<id>/auth.json`。Unix 下目录为 `0700`、文件为 `0600`。这些文件包含登录凭据，不应提交或分享。
- 当前 Codex 登录遵循其 `cli_auth_credentials_store`：支持文件、系统凭据库及 `auto`。`ephemeral` 登录不能持久化切换。系统凭据库访问失败会报告错误。
- 保留 Codex 配置注释、MCP、权限、插件及其他非受管字段。项目配置、启动参数和认证环境变量仍可能覆盖用户级设置。
- 写入使用锁、原子替换、受管字段比较和事务日志。中断后执行 `ccsw codex recover`；外部修改冲突不会静默覆盖。
- `ccsw codex disconnect` 恢复仍属于 CCSW 的配置字段，保留外部编辑。卸载会预览并清理登记过的账号文件，恢复受管 Codex 配置，保留聊天记录。
- 首次保存后 CCSW 配置升级到版本 3；旧版本不识别该版本，升级前可自行保留配置备份。

## 模型状态规则

CCSW 将“模型存在”和“模型启用”分开处理：

- 添加或发现模型会把它放入目录；未启用时模型仍然存在。
- `Space` 只切换启用状态，不删除目录项。
- `x` 是统一的删除快捷键，只删除手动添加的自定义模型，并要求确认。
- `disabled_models` 记录显式禁用项，因此重启后不会被默认模型或角色引用意外重新启用。
- `model-a` 与 `model-a[1m]` 是同一个目录模型；`[1m]` 只表示上下文规格，导入和发现时不会生成重复项。

## 模型 Token 参数

通过 `a`（Add model）添加模型后，在 Provider 页面选中该模型，按 `e` 可重新编辑模型 ID、名称、描述、启用状态、1M 标记和 Token 参数，按 `Ctrl+S` 保存。无论焦点位于模型列表还是详情面板，`e` 都编辑当前模型；`E`（`Shift+e`）编辑厂商配置。Home 厂商列表中，`e` 编辑选中的厂商。

在添加或编辑模型窗口内，按 `Alt+1` 可直接切换 `1M context`，无需移动到开关字段；普通数字 `1` 仍用于输入。Provider 页面非搜索状态下使用 `1` 切换。可配置以下参数：

- `Max output tokens`：最大输出上限。留空不限制；填写 `8192` 时，Claude 请求 `16384` 会下调到 `8192`，请求 `4096` 保持不变；请求没有提供上限时使用此值。
- `Context window`：上下文容量记录，用于展示和检查最大输出不超过容量。不裁剪对话，也不改变 Claude 自动压缩行为；`1M context` 仍是独立的模型标记。

两个字段只接受正整数。例如模型配置可以包含 `max_output_tokens = 8192`、`context_window = 32768`。已接入的配置保存后自动同步；限制由代理在解析出实际模型后应用，覆盖 Anthropic、Chat Completions 和 Responses。若上限与请求的 thinking budget 冲突，代理报错而不会擅自更改推理参数。

`1M context` 为模型 ID 添加 `[1m]` 标记，不会提升上游模型的实际容量；请按服务商支持情况启用。

## Claude `/model` 同步

CCSW 不启动 Claude，也不接管 Claude 的会话参数。同步采用“首次手动接入，之后自动更新”：

- 首次编辑只保存 CCSW 配置。按 `p` 或执行 `ccsw apply --profile <id>` 成功后，建立与当前 Claude settings 文件的接入记录。
- 接入后，在任何页面修改厂商、模型启用状态、默认模型、角色或上下文规格，都会自动同步；连续修改会合并到最新状态。
- 自动同步沿用上次明确选择的默认厂商，不随浏览位置变化。厂商或默认模型不可用时，选择可用项；全部禁用时清空 CCSW 管理的模型，保留接入记录。
- 写入 Claude 前会备份 settings，并保留无关配置。同步失败时，本地修改仍然保存，按 `p` 重试；重新打开 TUI 时会检查未同步修改。
- 同步记录上次写入的受管字段快照。如果地址、Token、默认模型或其他受管字段被手动修改或被其他工具修改，自动同步暂停。确认需要 CCSW 重新管理后，按 `p` 重新接入。
- 没有快照的旧连接升级后需按 `p` 一次建立快照。

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

macOS / Linux 默认配置路径为 `~/.config/ccsw/config.toml`，Windows 为 `%APPDATA%\ccsw\config.toml`。支持 `CCSW_CONFIG` 或 `XDG_CONFIG_HOME` 覆盖；执行 `ccsw config path` 查看实际路径。

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
# 可选；根据上游模型能力填写，删除这两行即不设置
max_output_tokens = 8192
context_window = 32768
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

### 请求处理与停止

代理使用连接 10 秒、响应头 120 秒、流式空闲 180 秒和非流式总时限 600 秒的限制。单个 SSE 事件上限 1 MiB，非流式响应上限 32 MiB；错误体最多读取 16 KiB、展示 4 KiB。流式文本按完整事件解析 UTF-8，异常断流会报告错误，不自动重试生成请求。

所有平台的 Stop 都通过私有 token 认证的关闭接口执行，停止成功前等待 daemon 锁释放，不再按 PID 文件杀进程。旧版 Unix 代理需要先使用原版本的 Stop 停止。

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

以下为 macOS / Linux 默认路径；Windows 路径见安装说明，XDG 环境变量可覆盖默认目录。

- 配置：`~/.config/ccsw/config.toml`
- 模型缓存：`~/.cache/ccsw/models.json`
- 代理状态与日志：`~/.local/state/ccsw/`
- 同步接入记录：`~/.local/state/ccsw/sync-state.json`（包含路径、默认厂商、变更标记及上次写入的受管字段快照，含本地代理认证信息，不保存上游凭据）

配置包含明文上游 Token，Unix 下强制使用 `0600` 权限。Token 不会写入缓存或运行日志。配置和缓存使用原子替换并带文件锁。编辑会合并其他实例对独立字段的修改；同字段冲突会要求重新打开编辑器。文件正在被其他实例写入时，TUI 会提示重试，不会一直等待文件锁。

## 卸载与配置清理

```sh
ccsw uninstall --dry-run  # 只查看清理清单，不修改文件；不带参数也是预览
ccsw uninstall --yes      # 停止当前用户代理、禁用自启并清理已确认归属的配置
```

执行前先关闭其他 CCSW 窗口。卸载逐项清理当前路径对应的配置、缓存、代理注册表、日志、PID、同步状态和锁文件；只移除空的应用目录，不递归删除目录，也不扫描其他用户。程序文件保留，可在配置清理成功后手动删除安装位置的 `ccsw` / `ccsw.exe`。

Claude 的 `settings.json`、聊天记录及其他应用文件保留。只有当 Claude 的地址和 token 仍能确认属于本 CCSW 配置时，才清理对应备份，并按同步快照逐字段移除仍与上次写入一致的受管设置；已经切换到其他服务的 Claude 设置及备份原样保留。历史同步记录中登记的设置路径也会检查。没有快照的旧连接仅清除可确认归属的地址和 Token，保留未验证的模型字段。

为防止误删，卸载拒绝 HOME 外的自定义路径、符号链接、Windows reparse point、Unix 硬链接、跨用户文件、共享状态、损坏的配置和无法确认归属的自启项。此时会报错并要求先处理这些路径，不会扩大删除范围。`--yes` 不会绕过这些检查。旧版代理若不支持认证停止接口，需要先用旧版 `ccsw proxy stop` 停止。自启管理器失败或运行中的代理无法停止时保留配置；中途磁盘 I/O 失败会明确报告未完成，可修复后重试。

## 开发

```sh
cargo fmt -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets
cargo build --locked --release
```

CI 在 Pull Request、版本标签推送或手动触发时运行：在 Ubuntu、macOS 与 Windows 上执行相同检查并生成平台二进制，同时执行依赖安全审计和 Docker 安全回归。版本标签通过全部发布门禁后生成 Release；普通 main 推送不会自动运行当前工作流。

Codex 的自动测试使用隔离 HOME、模拟登录凭据和本地上游，不读取真实账号。可另行安装 Codex CLI 后运行真实进程冒烟测试（无 API 调用费用）：

```sh
cargo build --locked
SMOKE_FORMAT=openai-responses python3 tests/fixtures/codex_cli_smoke.py
SMOKE_FORMAT=anthropic python3 tests/fixtures/codex_cli_smoke.py
SMOKE_FORMAT=openai-chat python3 tests/fixtures/codex_cli_smoke.py
```

该脚本通过本地模拟上游驱动 Codex 执行固定的临时文件读取、修改与验证命令。已在 Codex CLI `0.153.4` 验证三种路径；ChatGPT App `26.901.41600` 内置后端已验证可读取生成的配置，App UI 登录切换与真实订阅额度仍需实际账号验收。

TUI 按状态、事件、页面、表单、模型规则、布局和后台任务拆分在 `src/tui/`；CLI 与 TUI 共用 `src/sync.rs` 的同步服务。回归测试包含真实事件序列、延迟本地 API、同步失败恢复、并发编辑及 120×36 到 40×12 的布局检查。

### Docker 安全回归

```sh
docker build -f tests/docker/Dockerfile -t ccsw-uninstall-safety .
docker run --rm --network none --read-only --tmpfs /tmp:rw,nosuid,nodev,exec ccsw-uninstall-safety
docker run --rm --network none --read-only --tmpfs /tmp:rw,nosuid,nodev,exec --user 10001:10001 ccsw-uninstall-safety
```

镜像内先执行 Rustfmt、Clippy 和 Rust 测试；卸载场景在独立临时 HOME 中运行，不挂载宿主机 HOME，也不挂载 Docker socket。检查覆盖预览、完整清理、重复卸载、中文路径、文件/目录链接、损坏文件、锁竞争、自启失败、伪造 PID、其他代理存活及无关文件内容不变。Linux Docker 测试不替代 Windows/macOS 自启管理器实机验证，也不构成对恶意同权限进程并发篡改或硬件故障的绝对保证。

详细覆盖范围和实测平台见 [Docker 测试记录](tests/docker/RESULTS.md)。
