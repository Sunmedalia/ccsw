# CCSW

[![CI](https://github.com/Sunmedalia/ccsw/actions/workflows/ci.yml/badge.svg)](https://github.com/Sunmedalia/ccsw/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/Sunmedalia/ccsw)](https://github.com/Sunmedalia/ccsw/releases/latest)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

CCSW 是 Claude Code、Codex 与 Pi Agent 的多厂商、多模型配置管理器，支持保存和切换 Codex 订阅账号。它负责维护 Endpoint、凭据、模型映射和本地协议代理，但不会启动 Claude；同步完成后，直接在自己的终端运行 `claude` 即可。

它提供三个核心能力：

- 在 TUI 中管理厂商、模型目录、默认模型、角色别名与 1M 上下文。
- 将所有已启用模型聚合到 Claude 原生 `/model`，并实时同步启用状态。
- 把 Anthropic Messages 请求转发到 Anthropic、OpenAI Chat Completions 或 Responses 兼容网关。

> 本文对应 v0.1.14。新增在线更新与更安全的卸载流程；提供 macOS ARM64、Linux x64/ARM64 和 Windows x64 发布包。

[快速开始](#快速开始) · [快捷键](#tui-导航) · [Codex 配置与账号](#codex-配置与账号) · [Pi Agent 配置](#pi-agent-配置) · [模型参数](#模型-token-参数) · [同步](#claude-model-同步) · [端口设置](#修改本地代理端口--多系统用户) · [Herdr Pulse](#herdr-pulse-常驻监控) · [卸载](#卸载与配置清理) · [开发与测试](#开发)

## 安装

如果你要安装的是 **Herdr 侧栏插件**，直接看 [Herdr 插件安装](#通过-herdr-安装发布版插件)，不需要先手工安装 CCSW；如需快捷键，可按下文配置。

### 一键安装（macOS / Linux）

安装 CCSW（默认安装最新发布版，自动校验 SHA-256，无需 sudo 或 Rust）：

```sh
curl -fsSL https://raw.githubusercontent.com/Sunmedalia/ccsw/main/install.sh | bash
```

安装 **Herdr 的 CCSW Pulse 插件**，请在 Herdr 普通终端中执行（只需已安装 Herdr 0.7.0+，不需要 Git、Rust 或 Cargo）：

```sh
curl -fsSL https://raw.githubusercontent.com/Sunmedalia/ccsw/main/install.sh | bash -s -- herdr
# 自定义快捷键
curl -fsSL https://raw.githubusercontent.com/Sunmedalia/ccsw/main/install.sh | bash -s -- herdr --key prefix+shift+u
```

CCSW 安装到 `~/.local/bin/ccsw`，覆盖前备份旧程序；若目录不在 PATH，脚本会提示添加方式。支持 macOS ARM64、Linux x86_64 / ARM64。
Herdr 模式先检查 `herdr` 命令，不存在就提示“没有 Herdr”并退出；不会下载或安装 Herdr 本体。检测通过后下载并校验预编译 CCSW，生成不含构建步骤的插件清单，安装到 `${XDG_DATA_HOME:-$HOME/.local/share}/ccsw/herdr/plugin.*`，链接插件并合并快捷键；请保留该目录。重复运行可更新，旧插件目录保留以便恢复。该模式也会安装 CCSW 命令，需要发布版支持 `herdr-install`。

指定 CCSW 发布版本：

```sh
curl -fsSL https://raw.githubusercontent.com/Sunmedalia/ccsw/main/install.sh | CCSW_VERSION=v0.1.14 bash -s -- ccsw
```

以上在线命令需要本脚本已合并到 GitHub 的 main 分支。本地源码安装方式见下文。

### 下载 Release

v0.1.14 提供 macOS Apple Silicon、Linux x86_64/ARM64 二进制与 Windows x64 ZIP。Windows 安装及环境变量说明见 [README-Windows.md](README-Windows.md)。

```sh
# macOS Apple Silicon
curl -L https://github.com/Sunmedalia/ccsw/releases/latest/download/ccsw-macos-arm64.tar.gz | tar -xz

# Linux x86_64
curl -L https://github.com/Sunmedalia/ccsw/releases/latest/download/ccsw-linux-x86_64.tar.gz | tar -xz

# Linux ARM64
curl -L https://github.com/Sunmedalia/ccsw/releases/latest/download/ccsw-linux-arm64.tar.gz | tar -xz

chmod +x ccsw
sudo install ccsw /usr/local/bin/ccsw
```

### Windows（x64 ZIP）

从 [最新 Release 下载 Windows x64 ZIP](https://github.com/Sunmedalia/ccsw/releases/latest/download/ccsw-windows-x86_64.zip)，并下载旁边的 [SHA-256 文件](https://github.com/Sunmedalia/ccsw/releases/latest/download/ccsw-windows-x86_64.zip.sha256) 校验：

```powershell
$release = 'https://github.com/Sunmedalia/ccsw/releases/latest/download/ccsw-windows-x86_64.zip'
Invoke-WebRequest "$release" -OutFile '.\ccsw-windows-x86_64.zip'
Invoke-WebRequest "${release}.sha256" -OutFile '.\ccsw-windows-x86_64.zip.sha256'
$expected = ((Get-Content '.\ccsw-windows-x86_64.zip.sha256' -Raw) -split '\s+')[0]
$actual = (Get-FileHash '.\ccsw-windows-x86_64.zip' -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actual -ne $expected) { throw 'SHA-256 校验失败' }
Expand-Archive '.\ccsw-windows-x86_64.zip' "$env:LOCALAPPDATA\Programs\ccsw" -Force
& "$env:LOCALAPPDATA\Programs\ccsw\ccsw.exe" --version
```

无需管理员权限，CCSW 自身无需 Node、Git Bash 或 Visual C++ 运行库。完整的 PowerShell/CMD 示例、目录覆盖优先级、字符转义、npm 启动器、自启和更新方法见 [Windows 使用说明](README-Windows.md)。

配置默认位于 `%APPDATA%\ccsw\config.toml`，状态与缓存位于 `%LOCALAPPDATA%\ccsw\state`、`cache`。关闭 TUI 不会停止后台代理；更新前先执行 `ccsw proxy stop`，移动程序前先卸载旧位置的自启项。

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

1. 按 `n` 新建厂商，填写 API Endpoint、认证方式和凭据；点击底部 `Fetch models` 按钮或按 `Alt+F` 从该站点 API 获取模型，输入关键词搜索、用方向键选择、按 `Enter` 或点击 `Select` 回填默认模型。无需先保存厂商，`Esc` 或 `Back` 返回表单，获取失败可点击 `Refresh` 或按 `Ctrl+R` 重试，也可手填模型 ID。
2. 进入厂商详情，按 `a` 打开添加模型；在添加模型表单中按 `Alt+F` 获取 API 模型目录，也可以手动填写模型 ID。厂商详情页不再提供整站模型刷新。
3. 用 `Space` 启用模型；按 `e` 编辑输出 Token 上限等参数，按 `1` 切换 1M 标记。禁用只暂停模型，不会删除模型。
4. 按 `p` 接入 Claude：自动启动本地代理，并将全部启用模型写入 Claude 设置。之后保存的变更会自动同步。
5. 在终端运行 `claude`，使用 `/model` 选择模型。退出 CCSW 不会停止代理。

首次运行若检测到 `~/.claude/settings.json`，CCSW 会显示脱敏导入预览。也可以手动执行：

```sh
ccsw import
ccsw import --yes
```

## TUI 导航

界面采用统一英文标签，会随终端尺寸调整。100 列及以上并排显示模型与详情；窄窗口改为单面板，并让厂商信息自动换行。最低可用尺寸为 40×12，小于该尺寸时显示调整提示，仅在厂商首页可按 `q` 或 `Ctrl+C` 退出；其他页面使用 `Esc` 返回。表单会滚动以保持当前字段和文本光标可见；凭据始终遮蔽显示。任何主页面按 `?` 都会打开当前场景对应的 Help：

- `←/→` 或 `Tab`：切换 Home、All Models、Provider、Forms 分区。
- `↑/↓`：滚动当前帮助内容。
- `1`–`4`：直接打开对应分区。
- `Esc`、`q`、`?` 或 `Enter`：关闭 Help。

按钮统一使用青色主题，选中状态为青色底，删除操作为红色，不可用操作为灰色。厂商详情卡提供删除按钮；详情面板按 `x` 删除厂商，模型面板按 `x` 删除模型，删除前均需确认。

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
| `x` | 删除模型；网关模型不能删除 |
| `e` | 编辑当前模型的完整配置；模型列表和详情面板均可使用 |
| `E`（`Shift+e`） | 编辑当前厂商配置 |
| 鼠标点击 Provider 状态行 | 首次选中厂商，再次点击打开编辑表单 |
| `p` / `P` | 同步 / 代理 |

### Forms · 表单

模板和厂商的获取模型页面支持 `j/k` 下/上、`l` 使用、`h` 返回。获取模型页面按 `/` 或点击搜索栏进入搜索，`Esc` 返回导航；搜索和表单文本输入中 `hjkl` 按普通字母输入。帮助面板支持 `h/l` 切换分区、`j/k` 滚动；代理面板支持 `h/k` 上一项、`j/l` 下一项。

按 `n` 新建厂商时先选择模板：CommandCode、Volcengine、DeepSeek，或选择 `Custom` 手动填写。方向键或鼠标选择，按 `Enter` 或点击 `Use template` 使用。模板预填名称、URL、协议和 Bearer 认证方式，自动生成不冲突的 ID；仅需填写 Key 和默认模型（也可从 API 获取），按 `Ctrl+S` 保存。模板不包含密钥、已有模型或角色映射，预填字段仍可编辑。

| 模板 | URL | 协议 |
| --- | --- | --- |
| CommandCode | `https://api.commandcode.ai/provider/v1` | OpenAI Chat |
| Volcengine | `https://ark.cn-beijing.volces.com/api/coding` | Anthropic |
| DeepSeek | `https://api.deepseek.com/anthropic` | Anthropic |

API 模型列表支持鼠标：单击选中，再次单击同一模型使用；也可以按 `Enter` 或点击 `Select` 使用。厂商表单会回填选定的目标字段。

DeepSeek 的 `https://api.deepseek.com/anthropic`、`/anthropic/v1` 等地址获取模型时使用 `https://api.deepseek.com/models`，目录请求使用 Bearer 认证；对话仍使用配置的 Anthropic 地址和认证方式。

获取模型会回填当前选中的模型字段：先选中 `Default model`、`Opus`、`Sonnet`、`Haiku`、`Fable`、`Subagent` 或 `Fallbacks`，再点击 `Fetch models` 或按 `Alt+F`。选择界面标题会显示目标字段；`Fallbacks` 追加且不重复添加，其他字段替换当前值。焦点在地址、凭据等非模型字段时，默认回填 `Default model`。

厂商表单中的 `Opus`、`Sonnet`、`Haiku`、`Fable` 填写上游实际模型 ID。Claude 会话发送 `A::x` 请求后，该会话后续角色请求使用 A 的配置；发送 `B::y` 后切换到 B。同步写入 `ccsw-role::sonnet` 等角色标识，让代理根据会话选择上游模型；也兼容 `sonnet`、`sonnet5`、`claude-sonnet-…` 和旧式 Claude 模型 ID。

会话识别支持 `metadata.user_id` 中 JSON 的 `session_id` 和旧式 `_session_<UUID>`。没有可识别会话、会话尚未发送具体模型请求时，使用同步时的默认厂商。不同会话及不同代理路由分别记录，角色请求和 token 计数不切换厂商。仅在模型请求实际到达代理后切换；模型选择菜单本身不会通知代理。新子会话使用自己的记录，不自动继承父会话。记录在内存中保存，24 小时不活动或代理重启后重置，最多保存 4096 个会话。

已同步的精确路由优先；带其他厂商前缀的 ID 不会跨厂商兜底。当前会话厂商的角色未配置、目标模型未同步或已禁用时仍报错，不自动换厂商。升级后需重启本地代理、按 `p` 重新同步并重启 Claude，使角色请求使用新的角色标识。

| 按键 | 操作 |
| --- | --- |
| `↑/↓`、`Tab/Shift+Tab` | 切换字段 |
| `Enter` | 确认当前字段并进入下一项；最后一项直接保存 |
| `←/→`、`Home/End` | 移动文本光标 |
| `Backspace/Delete`、`Ctrl+U` | 删除字符 / 清空字段 |
| `Space`、`←/→` | 切换开关或选项 |
| `Alt+F`、`Ctrl+R` | 在厂商或自定义模型表单中，从厂商 API 获取可选模型 |
| `Alt+1` | 在模型表单任意字段或 API 搜索中切换 1M 标记 |
| `Ctrl+S` / `Esc` | 保存 / 取消 |

模型表单中，`Tab`/`Shift+Tab` 会依次遍历字段和 API 搜索面板；窄窗口按焦点显示面板。搜索时按 `Esc` 先清空搜索，再退出搜索，最后关闭表单。`Ctrl+S` 在搜索面板中也能保存。

模型刷新、代理管理和同步在后台执行，等待时仍可导航。重复刷新同一厂商会合并提示；过期请求不会覆盖新表单或已修改的厂商配置。

## 客户端配置隔离

配置版本为 v4：Claude 厂商保存在 `[profiles]`，Codex 保存在 `[codex.profiles]`。Pi TUI 直接读取 Pi 的 `models.json` 和 `settings.json`；旧的 `[pi.profiles]` 仅供兼容 CLI 导入/同步使用，不再作为 Pi 页面数据源。三个客户端的模型缓存分别保存。

旧版 v1–v3 的共享厂商会在迁移时复制为三份独立列表，以保留已添加的模型；之后编辑不再互相影响。Codex/Pi 副本不保留 Claude 角色别名。已有账号和接入快照保留，首次保存写入 v4；旧版本 CCSW 不能编辑 v4 文件。

## Pi Agent 配置

> v0.1.7 包含 Pi 原生文件管理；兼容验证基准为 Pi `0.85.1`。

点击顶部 **Pi** 标签或按 `F2` 切换到 Pi 配置管理。首页与 Claude Code 使用相同的 `CCSW Providers · F2 <下一个 Agent> · N providers` 标题和厂商布局。Pi 的厂商、模型、目录缓存和接入配置独立管理，编辑与导入不会更改 Claude/Codex。

进入 Pi 标签即读取 `PI_CODING_AGENT_DIR` 指定目录，默认 `~/.pi/agent`。无需导入：页面展示 `models.json` 中可编辑的自定义厂商，包括已有 `ccsw-*` 条目。新增、编辑、删除保存时直接修改原条目，不写入 CCSW 的 `[pi.profiles]`，也不生成带前缀的副本。状态栏显示实际读取目录。

Pi 自行连接厂商；页面不显示代理按钮，`P` 不打开代理面板。

| 按键 | 操作 |
| --- | --- |
| `i` | 重新读取 Pi 原生配置文件 |
| `n` / `a` | 新增厂商 / 模型，保存到 `models.json` |
| `e` | 厂商页面编辑厂商；模型页面编辑模型 |
| `E` | 编辑所属厂商 |
| `x` | 删除选中的厂商或模型，确认后写回文件 |
| `p` / Set default | 将选中厂商和模型写为 `settings.json` 的默认值 |
| `s` | 查看实际配置目录、可编辑厂商数量和只读条目原因 |

表单保存即生效，不需要先连接或同步。Pi 原生文件没有厂商/模型启用开关，配置中的模型都可用；列表统一显示 configured（已配置），不显示启用/禁用状态，模型表单也不再提供 Enable now 开关。单击仅选中条目，再次点击进入；`Space`、`A`、`C` 不修改文件，需要移除时使用 `x` 删除操作。删除当前默认厂商会清除默认引用，删除当前默认模型会改用该厂商的替代模型。`p`（或模型页的 `d`）仅设置默认选择，其他 `settings.json` 设置保留。

可用以下命令检查原生配置路径和读取结果：

```sh
ccsw pi files
```

以下旧版 CLI 命令仍保留，操作 CCSW 中的 Pi 副本并生成 `ccsw-*` 条目，与原生 TUI 编辑不同：

```sh
ccsw pi import --dry-run       # 预览导入，不执行凭据命令
ccsw pi import
ccsw pi apply --profile deepseek
ccsw pi apply --profile deepseek --model deepseek-v4-flash
ccsw pi status
ccsw pi disconnect
```

Pi **直接连接厂商**，不启动或依赖 CCSW 代理。三种格式对应 Pi 原生 `anthropic-messages`、`openai-completions`、`openai-responses`。API Key 会写入 Pi 配置，Unix 文件权限为 0600；字符串按 Pi 规则转义。输出上限由 Pi 客户端使用，不是代理强制限制。

- 目标目录遵循 `PI_CODING_AGENT_DIR`，默认 `~/.pi/agent`。TUI 保留原厂商 ID，`settings.json` 仅修改 `defaultProvider` / `defaultModel`。
- 模型 ID 移除 `[1m]` 后缀；显式 Context window 优先，否则 `[1m]` 对应 1,000,000。Max output tokens 对应 Pi 的 `maxTokens`。其他能力使用 Pi 默认值，导入模型保留其兼容性、输入类型和推理能力配置。
- 自定义 API 厂商和普通 API Key 可编辑；内置模型覆盖、OAuth、命令或环境变量凭据、混合协议及每模型独立认证等暂不支持的条目会保留在文件中，并在状态中列出只读原因，不执行凭据命令。
- 原有 Pi 厂商、主题、扩展、订阅登录、会话保留；不会修改 `auth.json`。原厂商与 `ccsw-` 项可同时出现在 Pi 中。本次不提供订阅多账号管理。
- 在 Pi 内重新打开 `/model` 可重新加载目录；启动默认值请在新 Pi 进程中确认。命令行、项目设置和扩展可能覆盖全局配置。
- 原生编辑保留厂商、模型中的额外字段（例如 `compat`、`cost`、`reasoning`、`input`）。写入使用文件锁、原子替换和事务日志；中断后重新加载恢复，遇到并发文件修改会提示重试。直接编辑无需断开管理。

验证真实 Pi 进程（隔离 HOME、本地模拟上游，不使用真实凭据）：

```sh
cargo build
SMOKE_FORMAT=anthropic python3 tests/fixtures/pi_cli_smoke.py
SMOKE_FORMAT=openai-chat python3 tests/fixtures/pi_cli_smoke.py
SMOKE_FORMAT=openai-responses python3 tests/fixtures/pi_cli_smoke.py
```

可使用 `CCSW_TEST_BINARY` 和 `CCSW_PI_BIN` 指定 CCSW / Pi 二进制。

## Codex 配置与账号

> Codex 配置随 v0.1.7 发布。CLI 与 ChatGPT App 内的 Codex 使用同一套目标配置。CCSW 显示的是磁盘配置状态；真实 App 的账号切换与新会话请求仍需在目标版本上验证，不能将“已写入”视为 App 已生效。

在 TUI 中点击顶部 **Claude Code / Codex / Pi** 标签，或按 `F2` 循环切换，按 `F3` 切换 Codex 的 API Providers / Accounts。三个标签分别读取独立的厂商和模型配置；修改只作用于当前客户端；Pi 直接管理原生配置文件，没有启用/禁用和代理同步操作。Codex 的 API 页面不显示 Claude 的角色别名设置。

### Codex API

1. 在 Codex 的 API Providers 页面添加或选择厂商。
2. 进入厂商页面选中模型；`e` 编辑模型、`E` 编辑厂商，`g` 设置推理强度。
3. 按 `p` / **Apply Codex**，同步 Codex 配置中所有已启用厂商的已启用模型，并将所选模型设为启动默认值。CCSW 启动独立的聚合代理，并写入 `model`、`model_catalog_json` 和专属 `model_providers.ccsw`。
4. 首次接入后重启 Codex CLI。在同一会话中使用原生 `/model` 选择已登记模型，可跨厂商切换，无需再次重启；请求由代理转发到所选厂商。模型列表使用 `厂商ID::模型ID` 区分同名模型，并附带“厂商名 · 模型名”。
5. 新增或修改模型后，再按 `p` 并重启 Codex CLI 加载新目录。Codex 的目录仅在启动时加载；禁用或删除模型后，代理立即拒绝新请求，但旧进程的列表可能仍显示它。

CCSW 显示的是磁盘启动默认值，不代表运行中会话正在使用的模型。按 `p` 不会改变已有会话的选择；请在 Codex 中通过 `/model` 切换。ChatGPT 订阅账号继续独立管理，不会加入 API 模型列表；桌面端仍需在目标版本验证。

支持 OpenAI Responses、Chat Completions 和 Anthropic 厂商。Responses 上游直接转发；另外两种格式转换文本、图片（取决于上游能力）、函数工具、命名空间工具、自定义编辑工具与流式输出。无法转换的内容返回明确错误，包括跨协议的加密推理历史、`previous_response_id` 和托管工具；转换型厂商默认关闭 Codex 托管网页搜索。远程 `/responses/compact` 只转发给 Responses 上游，其他上游需客户端本地压缩。

应用时为所有已启用的 Codex API 模型生成 `model_catalog_json`，避免 Codex 提示模型元数据缺失。每个模型保留各自的上下文容量；未配置时暂用 128K，`[1m]` 使用 1M。不写入固定的全局上下文或压缩阈值覆盖，让 Codex 按当前模型的目录元数据处理。默认只声明文本与基本工具能力，不假定第三方模型支持 Codex 托管工具或原生推理参数；目录中包含 Chat Completions 或 Anthropic 厂商时关闭托管网页搜索。切换订阅及断开管理时恢复原目录设置。

发往上游时，代理去除厂商命名空间和 Claude 专用 `[1m]` 后缀，使用真实模型 ID。`Max output tokens` 在代理侧限制实际输出；`Context window` 写入每个模型的目录元数据。推理强度仍需所选上游模型支持；转为 Chat Completions 时，目录默认的 `none` 不作为 `reasoning_effort` 发送，而是使用上游默认行为，并不保证关闭上游推理。上游返回结构化错误时，代理显示参数和具体原因，并过滤本地及上游认证凭据。跨协议切换保留已有兼容性检查：无法转换的历史会明确报错，不会静默丢弃。

```sh
ccsw codex apply --profile my-provider --model my-model --reasoning high
ccsw codex status
ccsw codex disconnect
```

可用本地模拟供应商验证真实 CLI 的 `/model` 切换（无 API 凭据，隔离配置，POSIX 环境）：

```sh
cargo build
python3 tests/fixtures/codex_model_switch.py
```

该检查验证同一进程、同一会话跨供应商切换，保留对话历史，并确认新增模型需同步和重启后出现。可用 `CCSW_CODEX_BIN` 指定 Codex CLI。

### Codex 订阅账号

Codex 首页依次显示 **All Models**、**ChatGPT Account** 和 API 提供商。**All Models** 汇总所有已启用厂商的已启用模型；单击选中，再次点击或按 Enter 打开所属厂商，按 `p` 同步目录并设置选中模型为启动默认值。

选中首页的 **ChatGPT Account**，按 **Space** 启用或禁用订阅配置，弹窗确认后生效（Enter / y 确认，Esc / n 取消，也可点击按钮）。首次启用前先 Enter 进入账号页，导入账号并用 Space 选中；之后会记住已使用的账号。

- **启用订阅**：保存所有 API 厂商当前的启用状态，自动关闭它们并应用所选 ChatGPT 账号；TUI 在 ChatGPT Account 卡片标记订阅 Enabled、原先开启的厂商标记 Paused by ChatGPT。All Models 保持与 Claude 相同的 API 模型汇总视图，此时为空。
- **关闭订阅**：恢复被自动关闭的厂商及其模型；原本手动关闭的厂商继续关闭。优先恢复先前使用的 API 模型；该模型已不可用时选择一个恢复启用的模型；没有可用 API 模型时断开 CCSW 管理并恢复原配置。
- 切换订阅账号不会覆盖保存的 API 启用状态。订阅期间 API 厂商保持关闭；先禁用订阅再启用 API 厂商。新增厂商不会被自动恢复为开启。
- 普通 API 厂商和模型的启用/禁用不弹窗；订阅的启用/禁用必须确认，包括通过 Apply 从 API 模式切入订阅。账号列表的 Space 仍仅选择账号，按 `p` 才应用。

订阅与 API 模式之间切换后需重启 Codex。API 模式内已加载的模型仍可通过原生 `/model` 切换，无需重启。CLI 可用 `ccsw codex accounts disable` 关闭订阅并恢复 API 配置。

账号页只提供导入与切换，不进行额度查询或浏览器登录：

| 按键 / 按钮 | 操作 |
| --- | --- |
| `i` / Import | 导入本机当前 Codex 登录，输入保存名称 |
| `I` / File | 导入指定 `auth.json` 文件 |
| `↑↓`、`j/k` | 选择账号 |
| `Space` | 选中光标所在账号，不应用配置 |
| `p` / Apply Codex | 使用账号，同时切换为 ChatGPT 提供商 |
| `Esc` / Back | 返回提供商列表 |
| `?` | 与 Claude 一致的分栏 Help：Providers / Accounts / Models / Forms |

导入成功后自动高亮，空格选中，再按 p 或点击 Apply Codex 才激活。同一身份重新导入会更新凭据，不增加重复账号；即使目标目录中仍有同一账号的旧凭据，也不会在激活时覆盖刚导入的新副本。不同工作区分别保存。已撤销的 refresh token 无法通过切换恢复：先在 Codex 中重新登录，再导入新凭据。

切换完成后重启 Codex CLI / App 并打开新会话。账号页的 `[Applied]` 是当前写入 Codex 的账号，`[Local login]` 是磁盘凭据身份；API 模式保留的登录文件不代表 API 请求使用 ChatGPT 订阅。本地身份读取不验证远程凭据有效性。

在 Codex 内切换模型或推理强度后，可以直接回 CCSW 按 `p` 应用。连接地址等受管字段发生外部变化时，按 `s` 查看，必要时按 `D` 断开后再应用；事务恢复使用 `ccsw codex recover`。

```sh
ccsw codex accounts import --name current
ccsw codex accounts import --name another --file /absolute/path/auth.json
ccsw codex accounts list
ccsw codex accounts use <account-id>
```

### Codex 文件与恢复

- Codex 目标目录遵循 `CODEX_HOME`，默认 `~/.codex`。登录与额度查询调用 `codex`，可用 `CCSW_CODEX_BIN` 指定二进制路径。
- 保存的账号元数据位于 CCSW 配置；凭据副本位于 CCSW 状态目录的 `codex-accounts/<id>/auth.json`。Unix 下目录为 `0700`、文件为 `0600`。这些文件包含登录凭据，不应提交或分享。
- 当前 Codex 登录遵循其 `cli_auth_credentials_store`：支持文件、系统凭据库及 `auto`。`ephemeral` 登录不能持久化切换。系统凭据库访问失败会报告错误。
- 保留 Codex 配置注释、MCP、权限、插件及其他非受管字段。项目配置、启动参数和认证环境变量仍可能覆盖用户级设置。
- 写入使用锁、原子替换、受管字段比较和事务日志。中断后执行 `ccsw codex recover`；外部修改冲突不会静默覆盖。
- `ccsw codex disconnect` 恢复仍属于 CCSW 的配置字段，保留外部编辑。卸载会预览并清理登记过的账号文件，恢复受管 Codex 配置，保留聊天记录。
- 首次保存后 CCSW 配置升级到版本 4；旧版本不识别该版本，升级前可自行保留配置备份。

## 模型状态规则

CCSW 将“模型存在”和“模型启用”分开处理：

- 添加或发现模型会把它放入目录；未启用时模型仍然存在。
- `Space` 只切换启用状态，不删除目录项。
- 模型列表中的 `x` 和删除按钮统一删除已配置模型，不区分模型来源，并要求确认。删除会清理角色、子代理和回退引用；默认模型由剩余模型接替。刷新接口目录不会自动恢复已删除模型，可通过 Add model 重新添加。最后一个默认模型需先添加替代模型才能删除。
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

### Provider 用量统计

日期显示在 `‹ / ›` 之间。`1 day / 1 week / 1 month / All time` 分别统计所选日期当天、截至该日最近 7 天、最近 30 天和全部记录；快捷键为 `d / w / m / y`。摘要以并排指标区突出所选范围和全部累计：调用数为主值，附带 tokens 与已完成请求成功率；各表格的累计列仍保留全部记录。All time 时日期切换禁用。

点击 **Chart 5** 或按 `5` 查看时间用量柱状图，`Calls c / Tokens v` 切换调用次数和 tokens。1 day 按小时，其他范围按天；窗口较窄或数据较长时自动合并相邻时段，并标注每柱跨度，始终展示完整范围。无调用时段标记为 `0`，缺失 token 用量标记为 `?`。图表遵循当前客户端和 provider 筛选以及账本固定时区，旧记录也可按小时查看。Usage 按钮统一使用对称内边距和固定间隔，窄屏自动压缩。

在 Usage 页点击 **Models 4** 或按 `4` 查看具体调用模型：按客户端、provider、模型分别展示当日/累计调用次数与 tokens，沿用当前日期和 provider 筛选。选中行下方显示模型及 provider 标识，窄屏也可查看 tokens。已有账本记录可以直接显示，无需重新开始统计；这里展示的是路由选择的上游模型，不是供应商内部实际执行模型的验证结果。

Provider 详情显示今日/累计调用次数和 tokens。点击顶部 **Usage** 标签（与 Claude Code / Codex / Pi 并列）或按 **F6** 打开独立用量页，默认汇总全部客户端。**F2** 循环切换四个标签；切换后保留筛选和视图状态。`1/2/3/4/5/6` 切换 Provider、历史、指标、模型、图表、Sessions；选中 provider 或历史日期按 Enter 查看对应模型。第一次点击选中行，再次点击进入。`←/→` 切换日期（不能晚于今天），`t` / Today 回到今天并自动跟随跨日更新。`Tab` 切换客户端并清除 provider 筛选，`a` / All × 查看全部 provider；`↑↓` / `PgUp/PgDn` 滚动，`r` 刷新。`Esc` / Back 优先清除 provider 筛选并返回 Provider 表，再次返回原客户端。统计每两秒自动刷新。

**Sessions（`6`）** 读取本机 Claude Code 和 Codex 会话日志，展示 session、项目、客户端、累计 tokens；较宽终端还显示输入、输出、缓存读取和最后活动时间。选中行后，下方显示完整 session ID、项目路径、模型列表及精确用量。`s` 或 Sort 按钮切换最近活动 / tokens 排序；`y` / All time 查看全部历史会话。

- 数据来源：`~/.claude/projects/`、`~/.codex/sessions/` 和 `~/.codex/archived_sessions/`，支持 `CLAUDE_CONFIG_DIR` / `CODEX_HOME`。只统计这些目录中仍存在的日志，包含有本地日志的直连会话；Pi 暂不支持。
- 日期范围筛选**最后活动时间**，每行 tokens 为**整个会话累计值**，不代表该日期范围内新增消耗。时间沿用 Usage 账本时区。进入 Sessions 会清除 provider 筛选；本地会话数据不与代理账本相加，也不回填代理请求记录。
- 输入统一包含缓存读取和写入，合计为输入 + 输出；缓存只作细分展示，不重复相加。Claude 按消息 ID 去重，Codex 取最新有效累计用量，不累加重复累计事件。模型列表表示日志中出现的模型，不提供按模型分摊的估算。
- 子会话独立列出，不自动并入父会话。Codex fork 标记 `*`，其累计值可能包含继承的历史，因此不提供跨会话总和。缺失用量显示 `?`，损坏或跳过的日志记录标记为部分数据。统计是本地日志报告值，不是账单或订阅额度。
- 打开 Sessions 时在后台扫描，每两秒检查更新并只解析新增内容；统计缓存仅保留在内存中，不修改客户端日志，不保存对话正文。首次打开或重启后会重新扫描历史；删除日志后对应会话不再展示。

- 覆盖经过 CCSW 本地代理的 Claude Code 和 Codex API 请求，按实际路由的 provider ID 和客户端分别归属。一条对话可能产生多次请求；每次向上游发起请求计一次，包括失败请求。成功、失败、中断、进行中分别显示。
- 每日按请求开始时间归属；首次创建用量库时保存本机 UTC 偏移，此后固定使用该偏移，界面显示具体时区。累计为启用记录以来的总数，不回填历史数据。改名保留历史；删除 provider 不删除账本，复用同一 ID 会接续原有累计。
- 输入、输出、缓存读取和缓存写入 tokens 来自上游原始 `usage`。流式响应在结束事件确认结果，同一请求的累计 usage 不重复相加。缺失值显示 `unknown` 或 `+ ?`，缓存计数单列，不重复加进 tokens 合计；不同协议的输入/缓存口径可能不同。
- 远程 Responses 压缩调用单独计数，不混入生成调用。模型目录刷新、健康检查和本地 token 估算不计数。Pi 直连和 ChatGPT 账号显示未接入统计。
- 账本存于 CCSW 状态目录的 `usage.sqlite3`（默认 `~/.local/state/ccsw/`）；仅保存请求时间、客户端、provider、模型、结果和 token 数，不保存对话、请求头或密钥。代理异常退出留下的进行中记录在下次启动时标记为中断。数据库读写失败会提示/记入代理日志，不阻止 API 转发；失败期间统计可能不完整。

升级后需重启旧的后台代理，新的请求才会开始记录：先等待正在运行的请求结束，再执行 `ccsw proxy stop` 和 `ccsw proxy start`。

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
ccsw uninstall --yes      # 停止代理、清理配置，并自动解绑本地 CCSW Herdr 插件
ccsw uninstall --yes --herdr  # 要求 Herdr 检查成功；检查失败则中止卸载
```

执行前先关闭其他 CCSW 窗口及 Pulse pane。卸载逐项清理当前路径对应的配置、缓存、代理注册表、日志、PID、同步状态和锁文件；只移除空的应用目录，不递归删除目录，也不扫描其他用户。如已安装 Herdr，普通卸载会检测并解绑经确认的本地 CCSW 插件链接，同时移除 `ccsw.open` 快捷键；GitHub 托管的插件保留，需使用 `herdr plugin uninstall ccsw` 卸载。`--herdr` 要求 Herdr 检查成功，检查失败则中止卸载。其他 Herdr 配置保持不变。程序文件和源码 checkout 保留，可在配置清理成功后手动删除安装位置的 `ccsw` / `ccsw.exe`。

Claude 的 `settings.json`、聊天记录及其他应用文件保留。只有当 Claude 的地址和 token 仍能确认属于本 CCSW 配置时，才清理对应备份，并按同步快照逐字段移除仍与上次写入一致的受管设置；已经切换到其他服务的 Claude 设置及备份原样保留。历史同步记录中登记的设置路径也会检查。没有快照的旧连接仅清除可确认归属的地址和 Token，保留未验证的模型字段。

为防止误删，卸载拒绝 HOME 外的自定义路径、符号链接、Windows reparse point、Unix 硬链接、跨用户文件、共享状态、损坏的配置和无法确认归属的自启项。此时会报错并要求先处理这些路径，不会扩大删除范围。`--yes` 不会绕过这些检查。旧版代理若不支持认证停止接口，需要先用旧版 `ccsw proxy stop` 停止。自启管理器失败或运行中的代理无法停止时保留配置；中途磁盘 I/O 失败会明确报告未完成，可修复后重试。

## 在线更新

```sh
ccsw update --check             # 查看 GitHub 最新 Release
ccsw update                     # 下载对应平台的发布包、校验 SHA-256 后更新程序
ccsw update --source .          # 在 Herdr 终端中快进更新当前源码并重新安装插件
```

Release 更新仅在版本号更新时执行，下载包必须带 GitHub 发布资产的 SHA-256 摘要；校验失败不会替换程序。源码更新要求当前分支没有未提交的已跟踪文件，并使用 `git pull --ff-only`，不会合并或覆盖本地提交。使用源码链接的 Herdr 插件请选择 `--source`；更新后重新打开 CCSW/Pulse pane，代理可在请求空闲时重启以加载新程序。Windows 若锁定正在运行的 EXE，已校验的新文件会留在原目录，关闭 CCSW 后按命令输出的路径替换。

## 开发

```sh
cargo fmt -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
cargo build --locked --release

# 可选：十万/百万条统计记录的查询与无变化刷新基准（使用临时数据库）
cargo test --locked --bin ccsw large_ledger_query_benchmark -- --ignored --nocapture
```

CI 在 main/dev 分支推送、Pull Request、版本标签推送或手动触发时运行：Ubuntu、macOS 与 Windows 执行检查，Windows 额外验证 Rust 1.88、npm 启动器、MSVC 静态运行库及解压后的 ZIP。版本标签通过跨平台测试、依赖安全审计和 Docker 安全回归后生成 Release。Windows ZIP 附带 SHA-256 校验文件。

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

### Claude Code 客户端设置

点击 **Settings** 或按 **F4** 打开 TUI 设置。在 Claude 标签页选择 **Claude settings**（或按 `c`）进入客户端设置。这些设置对当前 CCSW 配置的所有 Claude Provider 共用，切换模型仍使用各自的地址、认证和协议，同时保留客户端设置。它们不会修改系统或 shell 环境变量，也不影响 Codex / Pi。

TUI 设置提供五套完整主题：**Graphite（石墨）**采用炭黑背景与象牙白导航；**Tundra（苔原）**采用深松绿背景、羊皮纸文字和黄铜导航；**Paper（纸页）**采用浅纸色背景、深墨色文字和蓝墨导航；**Nightfall（夜航）**采用深海军蓝背景、淡紫导航与青绿成功状态；**Pulse** 与 Herdr 侧栏插件配色一致，采用蓝灰背景、青蓝导航和青绿状态色。每套主题统一背景、正文、选中块、边框和状态色；Classic 保留原有配色及终端背景。F4 设置页使用 `Tab`（或点击页签）切换 **CCSW UI** 与 **Pulse pane**，侧栏可独立选择 Pulse、Graphite、Tundra、Paper、Nightfall 五套主题。方向键或 `j/k` 切换当前页签的主题，`Enter` / Save 保存两项选择，`Esc` / Cancel 取消。运行中的侧栏会自动读取新配色。进入 Claude settings 后按 `Esc` 或点击 **Themes** 返回主题设置，保留之前的配色预览；有未保存的客户端设置时先确认放弃。主界面主题覆盖客户端页、模型表单、账号和用量图表。两项配色分别保存在状态目录的 `tui-theme.json` 和 `pulse-theme.json`，下次启动自动恢复；不会修改厂商配置或触发代理同步。所有客户端与用量页均可用 `F4` 打开。已有主题选择保留。

- 六项预设：AI 署名、Teammates、Tool Search、思考强度、禁用自动升级、禁用 Artifact。默认 `inherit` 表示不覆盖已有配置。
- 点击 **Fill presets**（窄屏显示 **Presets**），或按 **Alt+P**，填入隐藏署名、开启 Teammates / Tool Search、`max` 思考、禁用自动升级和 Artifact；这只修改草稿。
- **Add variable / Alt+N** 添加自定义变量；选中条目后 **Delete / Alt+D** 删除，**Show / Alt+V** 切换值的遮罩。值按原样保存，不执行 `$()` 或其他 shell 表达式。
- **Ctrl+S / Save** 保存。已连接时自动同步，未连接时等待按 `p`；**Esc** 返回，有改动时按 `y` 丢弃，其他键继续编辑。
- **Disconnect / Alt+X** 断开管理，恢复接管前的设置。删除覆盖或改回 `inherit` 也会在下一次同步时恢复原值；外部修改不会被恢复操作覆盖。

存储在 CCSW 配置的 `[claude]` / `[claude.env]` 下。同步目标遵循 `CLAUDE_CONFIG_DIR`，默认 `~/.claude/settings.json`。地址、认证、模型和配置目录变量由转发管理，不能作为自定义变量重复覆盖。文件使用私有权限，自定义值不会进入同步日志。

```toml
[claude]
hide_attribution = true

[claude.env]
CLAUDE_CODE_EFFORT_LEVEL = "max"
ENABLE_TOOL_SEARCH = "true"
CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS = "1"
DISABLE_AUTOUPDATER = "1"
CLAUDE_CODE_DISABLE_ARTIFACT = "1"
```

**同步成功表示配置已写入，不代表每个上游都支持对应功能。** 启动时读取的选项以及删除环境变量，需要重启 Claude Code。项目或组织层配置也可能影响实际生效值。

Anthropic 转发保留原生 Tool Search 内容。OpenAI Chat / Responses 使用普通函数调用兼容 Claude 客户端执行的搜索：传入当前请求的完整工具目录，将搜索结果中的引用转换为工具名称，保留调用 ID。这个模式不保证延迟加载的上下文节省；引用不在请求目录中或使用 Anthropic 服务端搜索工具时，返回明确错误。

模型编辑页新增 **Reasoning max**：`off / low / medium / high / xhigh`，默认 `high`，配置字段为 `reasoning_max`。转到 OpenAI 时，`max` 映射为该模型的上限，其他等级超过上限时下调，`off` 不发送推理参数。Chat 使用 `reasoning_effort`，Responses 使用 `reasoning.effort`；Anthropic 不改变原始等级。上游拒绝参数时不会暗中降级重试。

配置版本升级到 5；旧配置加载后默认不接管任何客户端偏好。升级后请勿使用只支持旧配置版本的 CCSW 写回该文件。

### 模型最小测试

在模型详情页点击 **F5 Test model**，或按 **F5**，向所选模型所属的 Provider 发送一次简短的 `Reply OK.` 请求。支持 Anthropic、OpenAI Chat 和 Responses，最多请求 64 个输出 token，30 秒超时。收到实际模型输出（包括推理输出）即通过，不要求必须回复 OK。状态栏显示模型名称、响应耗时或失败原因；HTTP 成功但没有输出不会被判定为通过。此测试仅验证基础文本推理，不覆盖 Codex 的工具调用、流式响应、推理参数或已有对话兼容性。

测试在后台执行，不改变模型选择、Provider 配置或同步状态；测试的是上游模型响应，不依赖本地转发是否启动。环境变量配置继续从底部 **Settings** 或 **F4** 进入，主页面右上角不再单独放置入口。

新增或编辑 Provider 时，每个模型输入框末尾提供 `[Test]` 和 `[1m]`。Test（或选中该行按 F5）使用草稿中的 URL、认证与模型名发送最小请求，不要求先保存 Provider；Fallbacks 行依次测试全部填写的模型。测试结果显示在表单底部。`[1m]` 高亮表示启用，灰色表示未启用，仍可用 Alt+1 切换。

Base URL 行也提供 `[Test]`（选中该行按 F5）。使用草稿地址和认证发送一次 GET 请求，8 秒超时，不调用模型。结果区分网络连接失败和 HTTP 状态：401/403 表示服务器可达但认证被拒绝，404/405 表示地址可达但基础路径不提供 GET 接口；需要确认模型可用时再使用模型行的 Test。

Codex 账号页支持 `Browser (b)` 浏览器登录和 `Device (d)` 设备码登录。输入账号名称后开始登录；等待时按 `Esc` 或 Back 取消。登录成功后账号自动保存并高亮，按 Space 选中、p 应用。网页登录使用本机 Codex 客户端和独立临时目录，不覆盖当前登录；Import / File 仍可导入已有凭据。

Codex 账号页的 `Rename (e)` 可修改当前高亮账号的显示名称；邮箱、工作区、套餐和登录凭据来自账号身份，不支持手动修改。Import 与 File 使用统一的可选账号备注，留空默认为 ChatGPT；File 先输入文件路径，再填写备注。重命名不会切换账号或重新登录。

Codex 账号详情显示缓存额度使用率、用量窗口重置倒计时、最近成功刷新时间，以及已应用/本机登录状态。点击 `Refresh (r)` 主动查询，平时浏览不会请求额度接口；`PgUp/PgDn` 滚动详情。查询失败保留旧缓存并标记失败，不把网络错误直接判定为登录过期。

## Herdr Pulse 常驻监控

`ccsw quick` 是独立的只读监控页面，不加载配置编辑器，也不会在启动时同步配置。
展示今日 Token、请求次数、缓存 Token、调用健康度、24 小时请求趋势，
以及服务商/模型的调用数和失败数。蓝灰底色、三行大号数字和独立的文字层级用于常驻侧栏；
字体家族继承终端，不修改其他 pane 的字体。面板使用完整高度，短屏可滚动；pane 小于 32 × 12 时自动切换为迷你布局，保留客户端切换、关键统计、滚动和常用操作。

### 通过 Herdr 安装发布版插件

```sh
herdr plugin install Sunmedalia/ccsw
```

Herdr 下载仓库后，安装钩子会按 `herdr-plugin.toml` 的版本下载对应 Release 的预编译 CCSW，校验 SHA-256 和程序版本，再放入插件目录的 `target/release/ccsw`。无需 Rust / Cargo，不在用户机器上编译，也不安装全局 `ccsw` 命令。需要 Git（Herdr 下载仓库）、Bash、curl、tar 和 sha256sum 或 shasum；支持 macOS ARM64、Linux x86_64 / ARM64。对应版本的 Release 必须已发布，下载失败会中止安装。

安装后在 Herdr 内打开侧栏：

```sh
herdr plugin action invoke ccsw.open
```

快捷键可按下方“手动安装”中的配置添加。更新时重新运行安装命令；如果之前使用本地脚本链接过同名插件，先执行 `herdr plugin unlink ccsw`，再安装。

此方式需包含下载钩子的提交已发布到仓库默认分支；旧 tag 仍使用该 tag 自己的安装流程。

### Herdr 插件一键安装（本地源码开发）

支持 macOS / Linux，需要支持 `herdr plugin` 的 Herdr（0.7.0+）和 Rust 1.88+ / Cargo。**在 Herdr 的普通终端中**，进入本项目目录，执行：

```sh
bash scripts/install-herdr.sh
```

这一条命令会依次：

- 构建当前源码的 release 程序，包括尚未发布的本地修改。
- 更新 `~/.local/bin/ccsw`，替换前备份旧程序，不需要 sudo。
- 把 Herdr 的 `ccsw` 插件链接到**当前项目目录**并启用，替换之前链接的旧目录。
- 自动合并 `prefix+u` 快捷键并重载 Herdr 配置；已有的 CCSW 快捷键会保留，不重复添加。配置改动前会输出备份位置，其他插件和快捷键保持原样。

安装完成后，默认按 **Ctrl+B，再按 u** 打开 / 关闭侧栏。侧栏直接按 **`s` / Sessions** 查看会话用量；按 `s` 返回今日用量。也可以按 `e` 打开完整 CCSW，在 **Usage → `6` Sessions** 查看更详细的列表。如果你修改过 Herdr 的 prefix，使用自己的 prefix。

项目目录是插件的运行位置，安装后请保留它。**更新时，在同一目录更新源码，再运行同一条安装命令即可。** 已打开的 CCSW 窗口仍是旧进程，需要退出后重新打开。脚本不会关闭工作中的 agent、重启 Herdr，或自动中断代理请求。

尚未下载源码时，先执行 `git clone https://github.com/Sunmedalia/ccsw.git && cd ccsw`，然后运行上面的脚本。脚本安装的是当前 checkout；旧 Release tag 可能尚未包含该脚本及 Sessions 功能。

如果 `prefix+u` 已被其他功能占用，安装器会说明冲突，不覆盖它；改用一个空闲键：

```sh
bash scripts/install-herdr.sh --key prefix+shift+u
```

**更新缓存 / 速度采集逻辑后**，旧代理也需要更新。在没有正在进行的 API 请求时执行（如果代理本来没启动，只执行 start）：

```sh
"$HOME/.local/bin/ccsw" proxy stop
"$HOME/.local/bin/ccsw" proxy start
```

代理保留已有监听地址和路由；不会同步或改写 Claude / Codex 的模型配置。新指标从新版代理采集的新请求开始显示。

确认安装结果：

```sh
herdr plugin list --plugin ccsw
"$HOME/.local/bin/ccsw" --version
```

### 使用侧栏

快捷键在当前 pane 右侧打开常驻监控，保留原 pane 的焦点；
再次触发会关闭当前标签页已有的监控，再按则重新打开。也可执行 `ccsw quick --open` 切换开关。
首次打开时按触发快捷键的 pane 自动选择统计页：Codex → Codex，Claude Code → Claude，
未识别到这两种 agent → All。识别仅针对触发 pane，不受其他 pane 的 agent 影响。
直接运行 `ccsw quick` 默认展示 All。

- `Tab` 或 `1/2/3` 切换 Claude / Codex / 全部统计，鼠标点击同样可用。
- 每两秒读取本地用量库；`r` 立即刷新。读取失败保留旧数据并标记 STALE。
- 首页在 `PROVIDERS / TODAY` 标题右侧点击 `[Models m]`（或按 `m`）切换服务商与模型明细；不再使用 `d`。滚轮、方向键、PgUp/PgDn、鼠标点击或拖动滚动条均可滚动，Esc/Home 回到顶部。
- 侧栏首页首屏先显示**今日 CCSW 网关用量**，再显示启动它的 Claude / Codex pane 的**当前 session 累计 token**；两者均用大数字展示，互不相加，各自保留输入/输出、缓存读写和缓存率。session 缓存复用率 = 缓存读取 ÷ 总输入（不含输出；缓存写入不算命中）。当前会话来自本地日志，约每 2 秒更新；Codex 恢复同一 session 时会合并多份日志的累计计数，避免新日志尚未写入 token 事件时用量暂时消失。`s` / `Sessions` 打开会话页，顶部保留当前会话摘要，下方显示其他本地会话；沿用 Claude / Codex / All 筛选。agent 切换 session 时侧栏随之同步。若 Herdr 尚未提供 session ID，会显示等待识别，不会把最近的日志误标为当前会话。会话页按 `t` 切换最近活动 / token 排序，`r` 刷新，`?` 查看统计说明。Fork 会话标记 `*`，可能包含继承用量。
- `c` / `Chart` 切换首页与图表页：上方为今日网关每小时请求数，下方为当前 session 今日每小时 token 增量（输入+输出，来自本地日志）；两组图分别缩放，不应直接比较柱高。
- `v`（或点击右上角 `V(v)` / `T(v)`）在文字版和简洁图形版之间切换，当前页面、客户端筛选和排序保持不变。简洁版仍保留网关与当前 session 的大 Token 数字；网关指标改为输入/输出双色条、请求/未知计数、缓存命中条（内含 R/W 读写量）及速率/测量流数的紧凑读数，保留各项数值而减少重复标签。健康条按已完成请求分为绿色成功、红色失败、金色中断，待完成请求不计入比例。图表页原本就是图形展示，切换后图表数据不变。
- `e` / `↗ Edit` 新开 Herdr 标签页运行完整 CCSW，并立即切换到新标签页和编辑 pane；监控 pane 继续常驻。
- 监控与 Edit 均由 Herdr 原生插件直接启动，终端不再显示 `exec` 或启动命令。
- `q` / `×` 退出监控。
- 操作按钮直接标注 `(e)`、`(c)`、`(s)`、`(r)`、`(q)`；会话页把 `(c)` 换为排序 `(t)`。
- 两组 24 小时趋势使用铺满内容宽度的六行柱状图，标出小时刻度；零用量不画虚假柱。
- `?` 或右上角帮助按钮查看统计范围，`Esc` 返回，首页不再常驻显示范围说明。

统计范围为当前 CCSW 配置经过本地网关的请求，**不是当前 Claude 会话统计或订阅剩余额度**。
直连 API 和 ChatGPT/Claude 订阅流量不包含在内。成功率 = 成功 /（成功 + 失败 + 中断），
待完成请求不计入分母；没有已完成请求时显示“无样本”。请求计数包含单独标注的压缩请求，
Token 缺失时显示未知提示，不当作零用量。插件支持 macOS / Linux。

监控首页在 `TOKENS / TODAY` 大数字下紧接显示请求数、网关缓存读写、`Cache hit`、`Output rate (E2E)` 和测量流数，然后才进入当前 Session 区块。缓存命中率按有完整缓存计数的成功生成请求计算，
分母统一为包含缓存读取/写入的全部输入 Token（OpenAI 输入本身已包含缓存，Anthropic 需相加），
缓存写入不算命中。协议取实际路由的上游格式，OpenAI 兼容网关即使返回 Anthropic 风格缓存字段，也不重复加到输入分母；支持 DeepSeek `prompt_cache_hit_tokens`，cache miss 不当作缓存写入。缺失 Anthropic 缓存写入计数时不猜测完整分母。
端到端输出速率是今日成功流式生成请求的输出 Token 总数除以对应请求耗时总秒数，
从发出上游请求计到流完成，包含首 Token 等待、隐藏推理和网络传输时间，不等于模型纯解码速度。页面显示测量样本数；无有效样本时显示 `—`。
旧记录保留，不回填未知协议或计时，也不将旧版缓存分母和输出阶段计时混入新指标；新版网关启动后采集新请求，面板重开即可展示。

### 手动安装（可选）

推荐使用上面的一键脚本。需要自己管理程序和快捷键时，可以仅构建并链接源码：

```sh
cargo build --locked --release --bin ccsw
herdr plugin link "$PWD"
herdr plugin enable ccsw
```

当前 Herdr 的 `plugin link` 默认启用，不接受旧示例中的 `--enabled`。手动安装需自行将下面的绑定合并到 Herdr `config.toml`（遵循 `HERDR_CONFIG_PATH`，默认 `~/.config/herdr/config.toml`），然后执行 `herdr server reload-config`：

```toml
[[keys.command]]
key = "prefix+u"
type = "plugin_action"
command = "ccsw.open"
description = "Toggle CCSW Pulse usage monitor"
```

也可以安装固定的已发布版本：`herdr plugin install Sunmedalia/ccsw --ref v0.1.14`。这会安装该 tag 的代码；仍需手动配置快捷键。
