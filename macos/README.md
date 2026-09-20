# CCSW Menu

macOS 原生菜单栏应用。查看 Codex 订阅额度、CCSW 代理用量、Claude / Codex / Pi 配置，并快速切换已有账号和模型。

- 320 × 460 原生毛玻璃浮动面板，系统明暗外观、淡青蓝强调色。
- 首行 Claude Code / Codex / Pi 标签，各自展示配置切换与用量；ChatGPT 订阅仅在 Codex 页展示。
- 独立 GUI 管理窗口：厂商、全部模型、订阅账号、用量明细、客户端设置、代理与状态六个页面。新增厂商可直接获取模型、测试连接和测试模型。
- 简体中文和英文；默认跟随系统，设置中可以切换。
- Codex 套餐、多个额度窗口、重置倒计时、缓存时间及刷新失败状态。
- 今日 / 7 天 / 30 天调用数、输入输出 Token 和趋势，支持客户端筛选。
- 账号切换、Claude / Codex / Pi 默认配置切换、代理启停及打开 TUI。

## 构建和运行

需要 macOS 13+、Xcode Command Line Tools（Swift 5.9+）与 Rust 1.88+。无第三方 Swift 依赖，按本机架构构建；已在 Apple Silicon 上验证。

从仓库根目录运行：

```sh
./scripts/build-macos-menu.sh
open "target/macos/CCSW Menu.app"
```

生成的 `.app` 自带同版本 `ccsw`，无需额外安装 CLI。可将它复制到 `/Applications` 后使用。使用临时本地签名；向其他机器公开分发时需另行签名、公证。

运行后点击菜单栏的叠层图标。应用不显示 Dock 图标；退出菜单栏应用不会停止代理。底栏滑杆按钮打开独立 GUI 管理窗口。GUI 与 TUI 共用配置文件、账号和账本，草稿互不影响；保存前会检查同一厂商是否被另一端修改，冲突时保留草稿并要求重新读取。GUI 可独立完成浏览器／设备码登录、导入、重命名、切换、刷新和删除账号。

**Claude Code 经代理切换无需重启。** Codex 应用配置或切换账号后仍会提示按需重启客户端。Pi 在管理窗口点击“使用”设置原生默认模型。

“登录时启动”默认关闭，在设置中可开启。若 macOS 要求批准，在系统设置的登录项中完成。需要菜单栏常驻时，请先将应用移至固定位置。

## 数据与设置

沿用现有配置及状态目录。Finder 启动通常不继承终端环境，可在设置中覆盖：

| 设置 | 含义 |
| --- | --- |
| `CCSW_CONFIG` | 完整 CCSW 配置文件路径 |
| `XDG_STATE_HOME` | 状态父目录，内部追加 `ccsw` |
| `XDG_CACHE_HOME` | 缓存父目录，内部追加 `ccsw` |
| `CLAUDE_CONFIG_DIR` | Claude 配置目录 |
| `CODEX_HOME` | Codex 配置目录 |
| `PI_CODING_AGENT_DIR` | Pi 配置目录 |
| `CCSW_CODEX_BIN` | Codex 可执行文件路径 |

路径支持 `~/`，留空沿用默认值。应用会补充 Homebrew、`~/.local/bin`、`~/.cargo/bin` 等常见可执行文件搜索目录；如 Codex 安装在版本管理器目录中，请指定完整程序路径。

本地状态在展开面板时和展开期间每 5 秒更新。当前本机订阅账号每 5 分钟按需查询额度；其他账号展开详情时查询过期缓存。失败后保留上次数据，自动重试有 5 分钟间隔；手动刷新不受缓存间隔限制。未安装可用 Codex CLI 时仍能查看已有缓存和其他配置。

菜单用量包含 CCSW 代理账本里的生成请求；GUI 用量明细还可切换压缩请求。不含 ChatGPT 订阅请求或 Pi 直连；不估算费用、续费日期或余额。时间范围遵循账本固定时区；缺失 Token 显示 `?`，图表只累计已知值。无账本与读取失败分别展示。

Swift 只通过内置 Rust helper 读取快照、执行操作，不直接编辑配置和凭据。快照 JSON 使用显式字段白名单，不输出密钥、原始认证或网关 URL。管理接口提供可编辑字段，地址中的用户名、密码、查询参数与片段会隐藏；不修改地址时保留原值。已保存的密钥不回传，默认保留；新密钥通过标准输入传递，不放在进程参数或临时文件中。错误返回稳定代码；同步暂停时，GUI 的“代理与状态”显示冲突字段并提供重新接管。客户端设置接口会返回用户自定义环境变量用于编辑，GUI 默认隐藏变量值。

## JSON 接口 v1

```sh
ccsw dashboard snapshot --json
ccsw dashboard management --json
ccsw dashboard action refresh-account ACCOUNT_ID --json
ccsw dashboard action use-account ACCOUNT_ID --json
ccsw dashboard action apply --client claude --profile PROVIDER_ID --model MODEL_ID --json
ccsw dashboard action apply --client codex --profile PROVIDER_ID --model MODEL_ID --json
ccsw dashboard action apply --client pi --profile PROVIDER_ID --model MODEL_ID --json
ccsw dashboard action proxy-start --json
ccsw dashboard action proxy-stop --json
```

`snapshot` 包含 `schema_version`、`generated_at`、`config_path`、`accounts`、`clients`、`proxy`、`usage` 和 `errors`。查询不进行登录刷新、迁移保存或 Pi 事务恢复；不同部分可独立失败。`usage.ranges` 提供三个日期范围与四个客户端筛选组合，包含总计和小时/日聚合点。

`management` 返回 `schema_version`、`providers`、`readonly_pi` 和 `errors`。每个可编辑厂商包含客户端、ID、版本指纹、连接信息、模型及角色映射，不含原密钥。

`dashboard action save-profile --json` 从标准输入读取单个厂商 JSON；现有厂商须携带读取时的 `revision`，新建时为 `null`。`credential_change` 为 `keep` 时保留认证，为 `replace` 时使用 `credential_kind` 和 `secret`（无认证使用 `none`）。输入上限 1 MiB。

`dashboard action delete-profile --json` 从标准输入读取 `{ "client": "claude", "id": "provider-id", "revision": "读取时的指纹" }`。GUI 删除前显示确认，并保留历史账本。Codex 保存厂商不会自动应用到客户端，需点击“使用”；已连接 Claude 的配置修改会自动同步代理。Pi 原生只读条目在管理窗口单独列出，不能覆盖。

`action` 返回 `{ "schema_version": 1, "ok": true, "restart_required": false, "error": null }`。操作失败仍输出合法 JSON，调用方必须检查 `ok`；启动和参数解析失败可能以非零进程状态返回。错误代码：

- `busy`：其他菜单操作持有写锁，未执行本次操作。
- `configuration_saved_sync_paused`：配置已保存，但 Claude 设置被外部修改；自动同步暂停以保留改动，用户可在 GUI 查看字段并明确重新接管。
- `configuration_saved_sync_failed`：配置已保存，但同步操作失败；在 GUI 的代理与状态页检查后重试。
- `saved` 表示是否已保存，`sync_conflicts` 只包含字段名称，不包含敏感值。
- `edit_conflict`：同一厂商已被另一端修改，不覆盖最新数据。
- `invalid_input`：输入、认证或模型参数无效，未保存。
- `action_failed`：操作未成功，需重新读取真实状态，不得乐观更新选择。

所有菜单写操作跨实例串行，底层继续使用现有配置/账号锁与事务。

## 验证

```sh
cargo fmt -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets -- --test-threads=1
swift test --package-path macos
```

Swift 测试使用离线快照，Rust 集成测试使用隔离 HOME、模拟配置及独立代理端口，不使用真实账号。覆盖凭据排除、无配置、部分故障、只读查询、账本时区与未知 Token、三类客户端操作及代理生命周期、操作冲突、缓存策略与语言切换。

可离线渲染面板做视觉检查，不启动 helper 或额度刷新：

```sh
swift build --package-path macos
mkdir -p target/macos/previews
macos/.build/debug/CCSWMenu --render-preview \
  macos/Tests/CCSWMenuTests/Fixtures/snapshot.json \
  target/macos/previews/light-en.png light en
macos/.build/debug/CCSWMenu --render-preview \
  macos/Tests/CCSWMenuTests/Fixtures/snapshot.json \
  target/macos/previews/dark-zh.png dark zh-Hans
```

离屏图片用于布局检查，实际毛玻璃效果取决于桌面背景与系统“减少透明度”设置。真实账号额度查询与登录启动仍取决于本机 Codex 安装、账号和系统授权状态。

预览新的客户端页与管理窗口，可在渲染命令最后追加 `codex`、`pi`、`manager`、`manager accounts`、`manager usage`、`manager preferences`、`manager proxy`、`editor` 或 `editor models`。管理预览使用同目录的 `management.json` 离线数据。

## GUI / TUI 功能对应

两套界面通过同一 Rust 服务层读写，不需要同时启动。

| TUI 功能 | GUI 入口 |
| --- | --- |
| 厂商新增、模板、编辑、删除、启停、搜索 | 厂商 |
| 模型获取、搜索、批量添加／启停、默认模型、角色、1M、Token 参数 | 厂商编辑 → 模型 |
| 测试连接、实际推理测试（最多 64 个输出 Token） | 厂商编辑 |
| 跨厂商模型浏览／搜索／应用 | 全部模型 |
| ChatGPT 登录、取消、导入、账号备注、切换、删除、额度刷新 | 订阅账号 |
| 日期、范围、客户端／厂商筛选，生成／压缩，历史／指标／模型／趋势 | 用量明细 |
| Claude 环境预设、自定义变量、署名、Codex 推理强度 | 客户端设置 |
| 代理启停、端口、自启动、接管／断开、外部配置导入、Codex 恢复 | 代理与状态 / 客户端设置 |
| Pi 原生可写配置管理、只读来源说明 | 厂商 / 代理与状态 |

新增只读接口：`dashboard workspace --json`、`dashboard usage-detail --json`。探测接口 `dashboard probe models|connection|model --json` 从 stdin 接收未保存厂商草稿；模型推理测试另带 `--model MODEL_ID`。管理动作 `dashboard action manage OPERATION --json` 从 stdin 接收参数；偏好设置使用版本指纹避免覆盖 TUI 的并发编辑。`dashboard login --name NAME [--device] --json` 输出 NDJSON 进度，关闭 stdin 取消登录。

本机验证记录：Rust 231 项测试串行通过，Swift 9 项测试通过；默认并行 Rust 测试曾在 SQLite 内部锁等待中停滞，因此以上命令使用串行测试。

导航范围：Claude Code / Codex / Pi 仅管理各自厂商及模型；订阅账号位于 Codex 下。全局用量默认汇总全部客户端、厂商及模型，可在页面内筛选。代理状态独立于客户端，管理全局代理服务。一级导航和窗口尺寸不随页面内容变化。
