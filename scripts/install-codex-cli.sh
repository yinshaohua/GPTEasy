#!/usr/bin/env bash

set -Eeuo pipefail

NODE_VERSION="${NODE_VERSION:-v24.21.0}"
NODE_MIRROR="${NODE_MIRROR:-https://registry.npmmirror.com/-/binary/node}"
NPM_REGISTRY="${NPM_REGISTRY:-https://registry.npmmirror.com}"
MINIMUM_NODE_VERSION="22.20.0"
NODE_INSTALL_ROOT="${XDG_DATA_HOME:-"$HOME/.local/share"}/nodejs"
NPM_GLOBAL_PREFIX="$HOME/.local"

# This script intentionally uses direct connections.
unset HTTP_PROXY HTTPS_PROXY ALL_PROXY http_proxy https_proxy all_proxy

log() {
  printf '[codex-install] %s\n' "$*"
}

fail() {
  printf '[codex-install] 错误：%s\n' "$*" >&2
  exit 1
}

version_ge() {
  [[ "$(printf '%s\n' "$2" "$1" | sort -V | head -n1)" == "$2" ]]
}

get_node_version() {
  local node_path="$1"
  "$node_path" --version 2>&1 | sed -n '1{s/^v//;p;q;}'
}

is_writable_directory() {
  local directory="$1"
  local probe

  mkdir -p "$directory" 2>/dev/null || return 1
  probe="$directory/.codex-write-test-$$-$(date +%s%N)"
  if ! (umask 077 && : > "$probe") 2>/dev/null; then
    return 1
  fi
  rm -f "$probe"
}

add_path_entry() {
  local entry="$1"
  local profile
  local path_line="export PATH=\"$entry:\$PATH\""

  case ":$PATH:" in
    *":$entry:"*) ;;
    *) export PATH="$entry:$PATH" ;;
  esac

  for profile in "$HOME/.profile" "$HOME/.bashrc"; do
    if [[ -f "$profile" ]] && ! grep -Fqx "$path_line" "$profile"; then
      printf '\n%s\n' "$path_line" >> "$profile"
    fi
  done
}

download_with_retry() {
  local uri="$1"
  local output="$2"
  curl -fL --retry 3 --retry-delay 2 --connect-timeout 15 -o "$output" "$uri"
}

install_system_tools() {
  command -v apt-get >/dev/null 2>&1 || fail '此脚本需要 Ubuntu/Debian 的 apt-get。'

  local sudo_cmd=()
  if [[ "${EUID:-$(id -u)}" -ne 0 ]]; then
    command -v sudo >/dev/null 2>&1 || fail '未找到 sudo，请以 root 运行或安装 sudo。'
    sudo_cmd=(sudo)
  fi

  log '准备系统工具：ca-certificates curl unzip xz-utils'
  "${sudo_cmd[@]}" apt-get update
  "${sudo_cmd[@]}" apt-get install -y ca-certificates curl unzip xz-utils
}

get_node_architecture() {
  case "$(uname -m)" in
    x86_64) printf 'x64\n' ;;
    aarch64|arm64) printf 'arm64\n' ;;
    *) fail "不支持的 CPU 架构：$(uname -m)" ;;
  esac
}

install_user_node() {
  local node_arch node_file node_dir tmp_dir archive_file checksums_file

  node_arch="$(get_node_architecture)"
  node_file="node-${NODE_VERSION}-linux-${node_arch}.tar.xz"
  node_dir="$NODE_INSTALL_ROOT/node-${NODE_VERSION}-linux-${node_arch}"

  if [[ ! -x "$node_dir/bin/node" ]]; then
    tmp_dir="$(mktemp -d)"
    archive_file="$tmp_dir/$node_file"
    checksums_file="$tmp_dir/SHASUMS256.txt"

    trap 'rm -rf "$tmp_dir"' RETURN

    log "下载 Node.js $NODE_VERSION（$node_arch）"
    download_with_retry "$NODE_MIRROR/$NODE_VERSION/$node_file" "$archive_file"
    download_with_retry "$NODE_MIRROR/$NODE_VERSION/SHASUMS256.txt" "$checksums_file"
    if ! grep -F "  $node_file" "$checksums_file" > "$tmp_dir/node-checksum.txt"; then
      fail "Node.js 校验清单中未找到：$node_file"
    fi
    (cd "$tmp_dir" && sha256sum -c node-checksum.txt)

    mkdir -p "$NODE_INSTALL_ROOT"
    tar -xJf "$archive_file" -C "$NODE_INSTALL_ROOT"
    [[ -x "$node_dir/bin/node" ]] || fail "Node.js 压缩包内容不符合预期：$node_dir/bin/node 不存在。"
    trap - RETURN
    rm -rf "$tmp_dir"
  fi

  add_path_entry "$node_dir/bin"
  NODE_BIN="$node_dir/bin/node"
  NPM_BIN="$node_dir/bin/npm"
}

install_system_tools

NODE_BIN="$(command -v node || true)"
NPM_BIN="$(command -v npm || true)"
EXISTING_NPM_PREFIX=''

if [[ -n "$NPM_BIN" ]]; then
  EXISTING_NPM_PREFIX="$("$NPM_BIN" prefix -g 2>/dev/null | sed -n '1{s/\r$//;p;q;}')" || true
fi

USE_EXISTING_NODE=false
if [[ -n "$NODE_BIN" && -n "$NPM_BIN" ]]; then
  installed_node_version="$(get_node_version "$NODE_BIN")" || installed_node_version=''
  if [[ "$installed_node_version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] && \
     version_ge "$installed_node_version" "$MINIMUM_NODE_VERSION"; then
    USE_EXISTING_NODE=true
    log "复用现有 Node.js $installed_node_version。"
  else
    log "现有 Node.js 版本过低或不可用，将安装用户态 Node.js $NODE_VERSION。"
  fi
else
  log "未找到可用的 Node.js/npm，将安装用户态 Node.js $NODE_VERSION。"
fi

if [[ "$USE_EXISTING_NODE" != true ]]; then
  install_user_node
  installed_node_version="$(get_node_version "$NODE_BIN")"
  version_ge "$installed_node_version" "$MINIMUM_NODE_VERSION" || \
    fail "安装的 Node.js 版本 $installed_node_version 低于要求的 $MINIMUM_NODE_VERSION。"
  log "已安装用户态 Node.js $installed_node_version。"
fi

[[ -x "$NPM_BIN" ]] || NPM_BIN="$(command -v npm || true)"
[[ -n "$NPM_BIN" ]] || fail '找不到 npm，请确认 Node.js 安装完整。'

if [[ -n "$EXISTING_NPM_PREFIX" ]] && is_writable_directory "$EXISTING_NPM_PREFIX"; then
  NPM_GLOBAL_PREFIX="$EXISTING_NPM_PREFIX"
  log "保留现有 npm 全局目录：$NPM_GLOBAL_PREFIX"
else
  log "使用用户级 npm 全局目录：$NPM_GLOBAL_PREFIX"
fi

mkdir -p "$NPM_GLOBAL_PREFIX"
add_path_entry "$NPM_GLOBAL_PREFIX/bin"

log "配置 npm 全局目录：$NPM_GLOBAL_PREFIX"
"$NPM_BIN" config set prefix "$NPM_GLOBAL_PREFIX" --location=user
log "配置 npm 镜像：$NPM_REGISTRY"
"$NPM_BIN" config set registry "$NPM_REGISTRY" --location=user

log '检查 npm 镜像连通性 ...'
"$NPM_BIN" ping --registry "$NPM_REGISTRY"

log '安装 @openai/codex@latest ...'
"$NPM_BIN" install --global '@openai/codex@latest' --registry "$NPM_REGISTRY"

hash -r 2>/dev/null || true
CODEX_BIN="$(command -v codex || true)"
if [[ -z "$CODEX_BIN" && -x "$NPM_GLOBAL_PREFIX/bin/codex" ]]; then
  CODEX_BIN="$NPM_GLOBAL_PREFIX/bin/codex"
fi
[[ -n "$CODEX_BIN" ]] || fail "安装完成但找不到 codex 命令。请重新打开 shell，或确认 PATH 包含：$NPM_GLOBAL_PREFIX/bin"

log '安装完成，版本信息：'
"$CODEX_BIN" --version
log '当前 shell 已可运行 codex；新 shell 会通过 ~/.profile 或 ~/.bashrc 读取 PATH。'
