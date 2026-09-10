# Pi Agent 验证记录

日期：2026-09-10。

这次使用用户现有的 **`debian` 容器（Debian 13 / trixie，aarch64）**，并非前一次独立的 Debian 12 测试镜像。容器内使用 Rust 1.95.0 编译源码，安装 Pi 0.85.1 进行真实进程测试。

## 通过的检查

- macOS 和 Debian 13：133 项 Rust 测试通过（117 单元、16 集成），包含导入去重、凭据转义、禁用后目录更新、外部编辑保护、事务恢复及卸载恢复。
- Debian 13：额外 33 项卸载安全回归通过。
- macOS：格式检查、Clippy 全 targets/features 检查通过（warnings as errors）。
- 两个平台的真实 Pi 进程：Anthropic、OpenAI Chat、OpenAI Responses 均通过模型列表、实际 HTTP 路径、流式工具调用、工具结果回传与最终回复测试。上游为本地模拟服务器，工具仅写入临时文件。
- Debian 13 的真实 PTY：CCSW 启动、F2 Claude/Codex/Pi 切换、厂商页面 e、模型页面 e、Help、退出通过。使用 120×36 终端及当前 CCSW 配置副本；Rust 渲染测试另覆盖窄窗口。
- 当前 Pi 配置副本：导入 5 个厂商、跳过 1 个内置模型覆盖项；重复导入不增加重复厂商。合并后 11 个厂商应用/恢复全部通过，auth.json 保持不变。

## 真实 Pi 请求

使用当前 CCSW 配置副本，由 Pi 直连厂商。每个厂商请求 `Reply only OK.`，禁用工具与思考，测试副本将最大输出设为 128 tokens。不启动 CCSW 协议代理。

| 厂商 | 结果 |
| --- | --- |
| 9router | 失败，Pi exit 1；宿主机 127.0.0.1:20128 连接被拒绝，服务未启动。容器副本使用 host.docker.internal。 |
| deepseek | 通过，Pi exit 0，1.2 秒 |
| edgefn | 通过，Pi exit 0，1.3 秒 |
| flatke | 通过，Pi exit 0，3.4 秒 |
| orcarouter | 通过，Pi exit 0，2.1 秒 |
| volcengine | 通过，Pi exit 0，1.5 秒 |

所有输入副本内容校验未变。测试不修改宿主机 CCSW/Pi 配置，不刷新订阅令牌，不输出密钥或登录数据；容器中的凭据副本在测试后删除。

## 运行方式

容器内源码及二进制保留在 `/tmp/ccsw-pi-source`，供本次调试使用。打开交互界面需要 `-it`：

```sh
docker exec -it debian /tmp/ccsw-pi-source/target/debug/ccsw
```

此命令使用容器自身配置；测试用的宿主机配置副本已清理，不会自动出现在该界面。`/tmp` 中的构建不是永久安装。

可复用脚本：

- `tests/fixtures/pi_cli_smoke.py`：真实 Pi + 模拟上游，通过 `SMOKE_FORMAT` 选择三种协议。
- `tests/docker/pi_current_config.py`：通过 `CCSW_TEST_INPUT` 指定含 ccsw.toml、models.json、settings.json、auth.json 的私有副本目录；默认验证导入/恢复，`--live` 发起真实 API 请求。
- `tests/docker/pi_tui.py`：Linux PTY 交互验证。

以上脚本用 `CCSW_TEST_BINARY` 指定 CCSW，Pi 进程测试另用 `CCSW_PI_BIN` 指定 Pi。真实 API 测试会产生正常 API 用量。OAuth、多订阅账号切换不在本次功能范围。
