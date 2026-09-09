# 当前配置 Docker 测试记录

日期：2026-09-09。镜像：`ccsw-codex-test:local`（Linux arm64，Rust 1.95.0 / Debian Bookworm）。

## 自动化检查

- Docker 构建中的 `cargo fmt --check`、Clippy（所有 targets/features，warnings as errors）和 debug 构建通过。
- Rust 测试：125 项通过（114 单元 + 11 集成）。
- Docker 卸载安全回归：root 33 项通过；普通用户 32 项通过、1 项跳过（需要 root 执行 chown）。

## 当前配置副本

使用宿主机 CCSW v2 配置、Codex 配置和现有文件型 ChatGPT 登录凭据。输入文件只读挂载，工作副本位于容器 tmpfs；容器退出后销毁。源文件内容校验未变化，未打印密钥或令牌。

断网容器验证通过：真实账号离线导入、重复导入去重、账号激活、6 个厂商 API 配置与订阅模式切换、断开管理后恢复原始 Codex 配置及登录数据、CCSW v2 → v3 迁移。

联网测试未挂载订阅凭据。每个厂商通过生成的本地 Codex Responses 路由发送 `Reply only OK.`，流式输出，输出上限 128 tokens。

| 厂商 | 结果 |
| --- | --- |
| 9router | HTTP 502；宿主机 127.0.0.1:20128 连接被拒绝，无服务监听。容器副本已改用 host.docker.internal。 |
| deepseek | HTTP 200，文本和 response.completed 正常，1.4 秒。 |
| edgefn | HTTP 200，文本和 response.completed 正常，2.4 秒。 |
| flatke | HTTP 200，文本和 response.completed 正常，3.0 秒。 |
| orcarouter | 首次 HTTP 200 有文本但无 completed；增加终态诊断后原参数复测通过，HTTP 200 / response.completed，2.7 秒。首次原因未确定。 |
| volcengine | HTTP 200，文本和 response.completed 正常，3.5 秒。 |

## 复现

先构建镜像：

```sh
docker build -f tests/docker/Dockerfile -t ccsw-codex-test:local .
```

离线配置及账号测试（按实际位置调整源文件路径）：

```sh
docker run --rm -i --read-only --network none --tmpfs /tmp:rw,nosuid,nodev \
  -v "$HOME/.config/ccsw/config.toml:/input/ccsw.toml:ro" \
  -v "$HOME/.codex/config.toml:/input/codex.toml:ro" \
  -v "$HOME/.codex/auth.json:/input/auth.json:ro" \
  ccsw-codex-test:local python3 - --offline < tests/docker/current_config.py
```

在线 API 测试：去掉 `--network none`、auth.json 挂载和 `--offline`。可加 `-e TEST_PROFILE=orcarouter` 仅测试一个厂商，`-e TEST_MAX_TOKENS=128` 设置输出上限。真实请求使用配置中的 API 凭据，可能产生正常 API 用量。任何厂商请求失败，脚本以非零状态退出。

## 验证范围

本次未刷新真实订阅令牌，未测试两个真实账号的在线切换（本机只有一份当前登录）。双账号逻辑由隔离的模拟账号集成测试覆盖。Linux Docker 不验证 macOS Codex App UI 或系统 Keychain；真实 API 测试验证 CCSW Responses 路由，不等同于完整 Codex CLI/App 交互测试。
