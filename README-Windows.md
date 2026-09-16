# CCSW for Windows x64

面向 Windows 10 22H2 / Windows 11 x64。ZIP 包含 `ccsw.exe`、本文和许可证。CCSW 自身不需要 Rust、Node、Git Bash 或额外安装 Visual C++ 运行库。管理已安装客户端时，该客户端仍需自己的运行环境。

## 解压运行

在 PowerShell 中，解压下载的归档并运行：

```powershell
Expand-Archive -LiteralPath '.\ccsw-windows-x86_64.zip' -DestinationPath "$env:LOCALAPPDATA\Programs\ccsw" -Force
& "$env:LOCALAPPDATA\Programs\ccsw\ccsw.exe" --version
& "$env:LOCALAPPDATA\Programs\ccsw\ccsw.exe"
```

在 CMD 中运行已经解压的程序：

```bat
"%LOCALAPPDATA%\Programs\ccsw\ccsw.exe" --version
"%LOCALAPPDATA%\Programs\ccsw\ccsw.exe"
```

推荐使用 Windows Terminal，配合支持中文的字体。可以自行将安装目录加入用户 Path；解压不会修改永久环境变量。不要把整个程序路径再次包进环境变量值中的引号。

SHA-256 验证（将输出与同目录 `.zip.sha256` 文件比较）：

```powershell
Get-FileHash -LiteralPath '.\ccsw-windows-x86_64.zip' -Algorithm SHA256
Get-Content -LiteralPath '.\ccsw-windows-x86_64.zip.sha256'
```

## 环境变量和路径

| 用途 | 优先级 |
|---|---|
| 用户主目录 | USERPROFILE → HOME → HOMEDRIVE 与 HOMEPATH 的有效组合 |
| CCSW 配置 | CCSW_CONFIG → XDG_CONFIG_HOME\ccsw\config.toml → APPDATA\ccsw\config.toml |
| CCSW 状态 | XDG_STATE_HOME\ccsw → LOCALAPPDATA\ccsw\state |
| CCSW 缓存 | XDG_CACHE_HOME\ccsw → LOCALAPPDATA\ccsw\cache |
| Claude | CLAUDE_CONFIG_DIR → 主目录\.claude |
| Codex | CODEX_HOME → 主目录\.codex |
| Pi | PI_CODING_AGENT_DIR → 主目录\.pi\agent |

空变量视为未设置。AppData 变量缺失时回退到主目录内的 `AppData\Roaming` / `AppData\Local`。相对路径以 CCSW 启动目录为基准。Pi 的覆盖目录支持 `~`、`~/…`、`~\…`。

PowerShell 示例：

```powershell
$env:CCSW_CONFIG = 'D:\工作目录\ccsw\config.toml'
$env:CCSW_CODEX_BIN = 'C:\Program Files\Codex\codex.exe'
$env:CCSW_CLAUDE_BIN = "$env:APPDATA\npm\claude.cmd"
& "$env:LOCALAPPDATA\Programs\ccsw\ccsw.exe" config path
```

CMD 的 `set "名称=值"` 写法中，外层引号不会进入变量值：

```bat
set "CCSW_CONFIG=D:\工作目录\ccsw\config.toml"
set "CCSW_CODEX_BIN=C:\Program Files\Codex\codex.exe"
```

CCSW 不会二次展开变量值里的 `%PATH%`、`$env:NAME`、`$NAME`。需要引用其他变量时，先让当前 Shell 完成展开。也不会把变量值拆成“程序 + 参数”。PowerShell alias、function 和 `.ps1` 不是 CCSW 内部客户端启动入口；npm 的 `.cmd` 启动器受支持。

程序搜索遵循 Path 目录顺序，并按 PATHEXT 匹配 `.exe`、`.com`、`.cmd`、`.bat`。显式指定程序后，启动失败不会切换到另一个同名程序。批处理对特殊字符有额外限制，无法安全编码时会报错；此时将 `CCSW_CODEX_BIN` / `CCSW_CLAUDE_BIN` 指向原生 EXE。原生程序参数、环境变量和 RPC JSON 中的秘密值不经过 Shell 拼接。

手动编辑 TOML 时，Windows 路径可以用单引号字面字符串；JSON 必须转义反斜杠。例如，同一路径分别写成：

```toml
model_catalog_json = 'C:\Users\用户\catalog.json'
```

```json
{"path": "C:\\Users\\用户\\catalog.json"}
```

`model_catalog_json` 示例属于 Codex 配置。CCSW 自动生成的 JSON/TOML 会完成相应转义，无需自行加反斜杠。

## 后台代理、自启和升级

```powershell
.\ccsw.exe proxy start
.\ccsw.exe proxy status
.\ccsw.exe proxy stop
.\ccsw.exe proxy install
.\ccsw.exe proxy uninstall
```

`proxy install` 在当前用户 Startup 目录创建 `CCSW Proxy.lnk`。代理在关闭 TUI/终端后继续运行。停止通过本地认证接口完成；CCSW 不按磁盘中记录的 PID 强制结束其他进程。

自启项必须属于当前程序和当前配置。同名快捷方式指向其他程序、其他配置或含有额外参数时，安装／卸载会拒绝覆盖。移动或重命名 EXE 前，先用原位置的程序执行 `proxy uninstall`，移动后重新执行 `proxy install`。

更新时先 `proxy stop`，关闭其他 CCSW 窗口，再覆盖同一路径的 EXE。保持安装路径不变时可保留自启项。

`uninstall --yes` 清理当前配置，保留程序本身和无关文件。用户主目录外的配置可正常使用，但自动卸载仍拒绝此类路径；junction/reparse point 也不会被递归清理。文件被其他程序锁住时会明确失败，保留原内容，可解除占用后重试。

Windows 凭据存储使用 Credential Manager；文件模式沿用父目录 ACL，不会自动重写用户目录权限。

## 源码构建和验证

安装 Rust 1.88+ 与 Visual Studio Build Tools 的 C++ 工具链，在 PowerShell 中执行：

```powershell
cargo test --locked --target x86_64-pc-windows-msvc --all-targets --all-features
cargo build --locked --release --target x86_64-pc-windows-msvc --bin ccsw
.\scripts\package-windows.ps1
```

打包和 npm 测试脚本使用 PowerShell 7（`pwsh`）；日常运行及上述安装示例兼容 PowerShell 5.1。构建启用静态 CRT，使用控制台子系统，并嵌入长路径感知 manifest。不自动修改系统长路径策略；UNC/网络文件系统的可达性、锁和权限取决于系统配置。

`test-support` 仅启用回归辅助程序；发布包不包含它。CI 检查 Rust 1.88、执行 Windows 原生回归、生成真实 npm shim，检查 PE 导入，再验证解压后的程序。Windows Server runner 的通过结果不能替代 Windows 10/11 桌面交互和实际登录自启验收；工作区验证记录见 `tests/WINDOWS_VALIDATION.md`。
