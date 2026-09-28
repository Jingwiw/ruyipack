<!--
SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>

SPDX-License-Identifier: MulanPSL-2.0
-->

# RuyiPack 快速入门

这是需要维护者审阅的源码预览版。它帮助减少 SPEC 的填写和修改工作，
不会自动推断全部依赖或构建软件；生成时默认尝试补全缺失的源码摘要。

## 安装

Ubuntu 先安装证书、下载工具和 C 链接器，再按 [Rust 官方说明](https://rust-lang.org/tools/install/)
安装 rustup：

```sh
sudo apt-get update
sudo apt-get install -y ca-certificates curl git build-essential
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs -o rustup-init.sh
sh rustup-init.sh -y --profile minimal
. "$HOME/.cargo/env"
git clone https://github.com/Jingwiw/ruyipack.git
cd ruyipack
cargo install --path . --locked
ruyipack --version
```

仓库固定所需工具链；首次编译需要联网取得它和锁定依赖。
`$HOME/.cargo/bin` 必须在 PATH 中。普通使用者不需要安装开发门禁的 REUSE / cargo-deny。

## 从新包开始

在空工作目录运行：

```sh
ruyipack init example --build-system cmake
# 编辑 example.toml：确认上游信息、源码、依赖、构建选项和文件列表。
# 自动尝试下载没有摘要的 Source；已有摘要保持原值。
ruyipack gen example --stdout
ruyipack gen example
```

脚手架故意保留未知必填项，未填写时生成失败且不写 SPEC。
本地包名检查没有发现冲突，不代表上游不存在同名包。
`gen` 只输出 SPEC，不改作者 TOML，也不落盘中间 TOML。下载失败或超时
会逐项警告原因，仍生成缺摘要的 SPEC；使用 `--offline` 完全禁止下载。
预览和 `--check` 也默认补全；每次运行都会重新下载作者 TOML 中仍缺摘要的 Source。
若要固定后续生成结果，将审阅后的摘要填回 TOML；已有摘要不会被自动刷新。

## 只读复验源码

```sh
ruyipack verify-sources --manifest example.toml --format json
# 已有 SPEC：无需安装 RPM 或 curl
ruyipack verify-sources example.spec
```

它重新下载全部远程 Source，包括已有摘要的项，分别报告匹配、不匹配、缺失或失败，
绝不补写或替换声明；本地材料不参与。全部适用项匹配才退出 0，否则退出 1。
发现不匹配先调查来源，不要直接重算覆盖。无法静态确定的宏会报告具体原因，
不会执行 Shell、Lua 或退回外部 RPM；必要时用 `-D 'archive_version 2.0'` 提供明确事实。

## 修改已有包

```sh
ruyipack edit example.spec --field package.version --view
ruyipack edit example.spec --set package.version=2.0 --check --format json
ruyipack edit example.spec --set package.version=2.0 --diff
# 审阅差异、源码和补丁后再显式写回：
ruyipack edit example.spec --set package.version=2.0 --format json
```

完整视图不支持复杂构造时，选择需要的字段；未选内容保留原始字节。
条件歧义或 Source 隐式编号无法确定时，不要用猜测的编号绕过错误。
给编辑命令加 `--hash-source 0`，
会按候选版本/Source 下载并补入摘要；相邻 bare `#!RemoteAsset` 也支持。
普通编辑仍离线；缺摘要在生成和检查中均为警告，不是假装已验证。
终端直接运行 `edit example.spec` 会先选择字段；脚本使用 `--field` / `--set`，
只有需要完整映射时才用 `--all`。

## check 通过后还要做什么？

- 用 `verify-sources` 复验已有摘要，并核对来源；摘要匹配不代表来源可信。
- 核对补丁是否适用，以及声明许可证是否符合上游实际内容。
- 在目标 openRuyi 环境验证 RPM 宏、依赖、构建、测试和产物文件归属。
- 最后审阅差异并提交；静态 pass 不是无人值守发布许可。

Version/Source URL 变化会产生复核提醒；JSON 的 `review_required` 是待办，
不是已经执行的检查。也不要把空复核列表理解成原生构建已通过。
仅应用自己可信工作区内的草稿。批次不是整体原子事务，失败后先查看已写文件。

更多字段、输出和边界见 [命令与 manifest 参考](reference.md)。
