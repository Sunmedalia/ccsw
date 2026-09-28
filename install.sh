#!/usr/bin/env bash
# Usage: curl -fsSL https://raw.githubusercontent.com/Sunmedalia/mux/main/install.sh | bash -s -- [mux|herdr]
set -euo pipefail

main() {
    local mode="${1:-mux}"
    if [[ $# -gt 0 ]]; then shift; fi
    case "$mode" in
        -h|--help)
            echo '用法: bash install.sh [mux|herdr [--key prefix+shift+u]]'
            echo 'mux: 下载发布包到 ~/.local/bin；MUX_VERSION 可指定版本（如 v0.1.15）。'
            echo 'herdr: 下载预编译 Mux Pulse 插件；需要已安装 Herdr，并在 Herdr 终端运行。'
            return ;;
        mux|herdr) ;;
        *) echo "未知安装目标: $mode（可选 mux / herdr）" >&2; return 1 ;;
    esac
    local platform arch
    case "$(uname -s)" in
        Darwin) platform=macos ;;
        Linux) platform=linux ;;
        *) echo '此脚本支持 macOS / Linux；Windows 请参阅 README-Windows.md。' >&2; return 1 ;;
    esac
    local key=prefix+u
    if [[ "$mode" == herdr ]]; then
        command -v herdr >/dev/null 2>&1 || { echo '没有 Herdr，请先安装 Herdr 后再安装 Mux 插件。' >&2; return 1; }
        [[ "${HERDR_ENV:-}" == 1 ]] || { echo '请在 Herdr 的普通终端中运行。' >&2; return 1; }
        if [[ $# == 2 && "$1" == --key && -n "$2" ]]; then
            key="$2"
        elif [[ $# != 0 ]]; then
            echo '用法: bash install.sh herdr [--key prefix+shift+u]' >&2; return 1
        fi
    else
        [[ $# == 0 ]] || { echo 'mux 模式不接受额外参数。' >&2; return 1; }
    fi
    case "$(uname -m)" in
        arm64|aarch64) arch=arm64 ;;
        x86_64|amd64) arch=x86_64 ;;
        *) echo '当前 CPU 架构没有预编译发布包。' >&2; return 1 ;;
    esac
    [[ "$platform-$arch" != macos-x86_64 ]] || {
        echo 'macOS Intel 暂无发布包，请使用 cargo install --path . 从源码安装。' >&2; return 1;
    }
    local program
    for program in curl tar; do
        command -v "$program" >/dev/null 2>&1 || { echo "缺少依赖: $program" >&2; return 1; }
    done
    local checksum
    if command -v sha256sum >/dev/null 2>&1; then checksum=sha256sum
    elif command -v shasum >/dev/null 2>&1; then checksum=shasum
    else echo '需要 sha256sum 或 shasum 校验下载文件。' >&2; return 1
    fi
    local base asset version
    version="${MUX_VERSION:-latest}"
    if [[ "$version" == latest ]]; then
        base=https://github.com/Sunmedalia/mux/releases/latest/download
    else
        [[ "$version" =~ ^v[0-9]+\.[0-9]+\.[0-9]+([.-][a-zA-Z0-9.-]+)?$ ]] || {
            echo 'MUX_VERSION 必须是版本标签，如 v0.1.15。' >&2; return 1;
        }
        base="https://github.com/Sunmedalia/mux/releases/download/$version"
    fi
    asset="mux-$platform-$arch.tar.gz"
    # Run staging in a subshell so cleanup also applies to download/validation failures.
    (
        set -e
        work="$(mktemp -d)"
        staged=''
        trap 'rm -rf "$work"; if [[ -n "$staged" ]]; then rm -f "$staged"; fi' EXIT
        echo "正在下载 $asset ($version)"
        curl --fail --show-error --silent --location --retry 3 "$base/$asset" -o "$work/$asset"
        curl --fail --show-error --silent --location --retry 3 "$base/$asset.sha256" -o "$work/checksum"
        read -r expected _ < "$work/checksum"
        [[ "$expected" =~ ^[a-fA-F0-9]{64}$ ]] || { echo '无效的 SHA-256 文件。' >&2; exit 1; }
        if [[ "$checksum" == sha256sum ]]; then
            actual="$(sha256sum "$work/$asset")"
        else
            actual="$(shasum -a 256 "$work/$asset")"
        fi
        actual="${actual%% *}"
        [[ "$actual" == "$expected" ]] || { echo 'SHA-256 校验失败，未安装。' >&2; exit 1; }
        # Extract only the binary as bytes, never archive paths or links.
        tar -xzOf "$work/$asset" mux > "$work/mux"
        [[ -s "$work/mux" ]] || { echo '发布包中的程序为空。' >&2; exit 1; }
        chmod 755 "$work/mux"
        binary_version="$("$work/mux" --version)"
        echo "$binary_version"
        if [[ "$mode" == herdr ]]; then
            plugin_version="${binary_version#mux }"
            [[ "$plugin_version" =~ ^[0-9]+\.[0-9]+\.[0-9]+([.-][a-zA-Z0-9.-]+)?$ ]] || {
                echo '无法识别 Mux 发布版本。' >&2; exit 1;
            }
            # The native installer validates config and preserves existing shortcuts/backups.
            "$work/mux" herdr-install --help >/dev/null || {
                echo '此发布版不支持插件安装，请选择包含 herdr-install 的新版 Release。' >&2; exit 1;
            }
            parent="${XDG_DATA_HOME:-$HOME/.local/share}/mux/herdr"
            mkdir -p "$parent"
            plugin_dir="$(mktemp -d "$parent/plugin.XXXXXX")"
            mkdir -p "$plugin_dir/target/release"
            cp "$work/mux" "$plugin_dir/target/release/mux"
            cat > "$plugin_dir/herdr-plugin.toml" <<MANIFEST
id = "mux"
name = "Mux Pulse"
version = "$plugin_version"
description = "Persistent side-pane monitor for Claude usage, request health and traffic"
min_herdr_version = "0.7.0"
platforms = ["macos", "linux"]

[[panes]]
id = "quick"
title = "Mux Pulse"
placement = "split"
command = ["./target/release/mux", "quick"]

[[panes]]
id = "editor"
title = "Mux"
placement = "tab"
command = ["./target/release/mux"]

[[actions]]
id = "open"
title = "Toggle Mux Pulse at the right edge"
command = ["./target/release/mux", "quick", "--open"]
MANIFEST
            "$plugin_dir/target/release/mux" herdr-install --source "$plugin_dir" --key "$key"
            echo "已安装 Mux Pulse 插件，请保留目录: $plugin_dir"
            exit 0
        fi
        destination="$HOME/.local/bin"
        mkdir -p "$destination"
        [[ ! -L "$destination/mux" && ! -d "$destination/mux" ]] || {
            echo "$destination/mux 是链接或目录，请先手动处理。" >&2; exit 1;
        }
        staged="$(mktemp "$destination/.mux-install.XXXXXX")"
        cp "$work/mux" "$staged"
        chmod 755 "$staged"
        if [[ -e "$destination/mux" ]]; then
            backup="$(mktemp "$destination/mux-backup.XXXXXX")"
            cp -p "$destination/mux" "$backup"
            echo "旧程序备份: $backup"
        fi
        mv -f "$staged" "$destination/mux"
        staged=''
        echo "已安装: $destination/mux"
        case ":$PATH:" in
            *":$destination:"*) ;;
            *) echo '请在 shell 配置中添加: export PATH="$HOME/.local/bin:$PATH"' ;;
        esac
        echo '更新后请重新打开 Mux；运行中的代理请在请求结束后手动重启。'
    )
}

# Keep the entrypoint last so bash reads all definitions before a piped installation starts.
main "$@"
