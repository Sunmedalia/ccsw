#!/usr/bin/env bash
# Install or update this checkout; safe to repeat. Run inside a Herdr terminal.
set -euo pipefail

if [[ "${HERDR_ENV:-}" != 1 ]]; then
    echo '请在 Herdr 的终端中运行此脚本。' >&2
    exit 1
fi
case "$(uname -s)" in
    Darwin|Linux) ;;
    *) echo 'Herdr 插件目前支持 macOS / Linux。' >&2; exit 1 ;;
esac
for program in cargo herdr; do
    if ! command -v "$program" >/dev/null 2>&1; then
        echo "缺少 $program，请先安装 Rust 1.88+ 和支持插件的 Herdr。" >&2
        exit 1
    fi
done

ccsw_source_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
echo "正在构建 CCSW：$ccsw_source_dir"
# Herdr's manifest uses this exact target path even with a custom Cargo target dir.
cargo build --locked --release --bin ccsw \
    --manifest-path "$ccsw_source_dir/Cargo.toml" \
    --target-dir "$ccsw_source_dir/target"
"$ccsw_source_dir/target/release/ccsw" herdr-install --source "$ccsw_source_dir" "$@"
