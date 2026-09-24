#!/usr/bin/env bash
# Herdr install hook: download only; Herdr owns registration and configuration.
set -euo pipefail

root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
# Read only top-level version, before any TOML tables. No external TOML runtime needed.
version="$(sed -n '/^[[:space:]]*\[/q; s/^version = "\([^"]*\)"[[:space:]]*$/\1/p' "$root/herdr-plugin.toml")"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+([.-][a-zA-Z0-9.-]+)?$ ]] || {
    echo '插件清单缺少有效版本号。' >&2; exit 1;
}
case "$(uname -s)/$(uname -m)" in
    Darwin/arm64|Darwin/aarch64) platform=macos-arm64 ;;
    Linux/x86_64|Linux/amd64) platform=linux-x86_64 ;;
    Linux/arm64|Linux/aarch64) platform=linux-arm64 ;;
    *) echo '没有适用于当前平台的预编译插件（支持 macOS ARM64、Linux x86_64/ARM64）。' >&2; exit 1 ;;
esac
for program in curl tar; do
    command -v "$program" >/dev/null 2>&1 || { echo "缺少依赖: $program" >&2; exit 1; }
done
if command -v sha256sum >/dev/null 2>&1; then
    checksum=(sha256sum)
elif command -v shasum >/dev/null 2>&1; then
    checksum=(shasum -a 256)
else
    echo '需要 sha256sum 或 shasum 校验发布包。' >&2; exit 1
fi
asset="ccsw-$platform.tar.gz"
base="https://github.com/Sunmedalia/ccsw/releases/download/v$version"
work="$(mktemp -d)"
staged=''
trap 'rm -rf "$work"; if [[ -n "$staged" ]]; then rm -f "$staged"; fi' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

echo "下载 CCSW v$version ($platform)"
if ! curl -fsSL --retry 3 "$base/$asset" -o "$work/archive"; then
    echo "无法下载 $asset，请确认 v$version Release 已发布对应二进制。" >&2; exit 1
fi
curl -fsSL --retry 3 "$base/$asset.sha256" -o "$work/checksum"
read -r expected _ < "$work/checksum"
[[ "$expected" =~ ^[a-fA-F0-9]{64}$ ]] || { echo '无效的 SHA-256 文件。' >&2; exit 1; }
actual="$("${checksum[@]}" "$work/archive")"
actual="${actual%% *}"
[[ "$actual" == "$expected" ]] || { echo 'SHA-256 校验失败，未安装。' >&2; exit 1; }
# Stream only this entry; never extract archive paths or symlinks.
tar -xzOf "$work/archive" ccsw > "$work/ccsw"
[[ -s "$work/ccsw" ]] || { echo '发布包中的程序为空。' >&2; exit 1; }
chmod 755 "$work/ccsw"
[[ "$("$work/ccsw" --version)" == "ccsw $version" ]] || {
    echo '二进制版本与插件清单不一致，未安装。' >&2; exit 1;
}
destination="$root/target/release"
mkdir -p "$destination"
[[ ! -L "$destination/ccsw" && ! -d "$destination/ccsw" ]] || {
    echo '插件程序路径是链接或目录，请先手动处理。' >&2; exit 1;
}
staged="$(mktemp "$destination/.ccsw-download.XXXXXX")"
cp "$work/ccsw" "$staged"
chmod 755 "$staged"
mv -f "$staged" "$destination/ccsw"
staged=''
echo "已准备插件程序: $destination/ccsw"
# Releases from before automatic binding do not have this subcommand. Keep
# their installation working; the next release will configure the key here.
if ! "$destination/ccsw" herdr-bind; then
    echo '此发布版未能自动配置快捷键；可手动绑定 ccsw.open。' >&2
fi
