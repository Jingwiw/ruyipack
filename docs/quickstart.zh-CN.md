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

先在空工作目录初始化，再准备有已提交 `main` 分支的配方 Git 仓库。
下面的 `RECIPE_REPOSITORY` 是你选择的配方仓库地址：

```sh
ruyipack init . --clone "$RECIPE_REPOSITORY"
ruyipack new example --pkgname example --build-system cmake
# 留在工作区根目录即可；各命令位置参数都是 WORK。
# 编辑 example.toml：确认上游信息、源码、依赖、构建选项和文件列表。
# 自动尝试下载没有摘要的 Source；已有摘要保持原值。
ruyipack gen example --stdout
ruyipack gen example
# 审阅补足后的 TOML / 缓存 SPEC，再显式发布
ruyipack gen example --spec=auto
```

省略 `--clone` 的 `init` 完全离线，不需要 Git、Docker 或网络。
初始化先完成，再将仓库 clone 到配置的 `recipes` 路径（默认 `openruyi`）；
Git 或网络失败退出 1，但保留配置和 Git 实际留下的文件，并分别报告初始化成功与 clone 失败。
重复裸 `init` 不会自动 clone；只有配置有效且目标不存在时，才可显式 `--clone` 重试。
已有目标（包括空目录和中断目录）不会覆盖；损坏配置也不会自动修复或启动 clone。

脚手架故意保留未知必填项，未填写时生成失败且不写 SPEC。
`new example-test --pkgname example` 可创建同一个包的独立开发区；后续 `new example-test`
读取保存的包绑定。固定的 `checkout/` 保存 Git 工作树，TOML 在其外侧。
已有包的脚手架不会反向导入 SPEC；`edit WORK` 建立绑定原 SPEC 的 stage。
`gen` 只认 WORK，可以处理完整配方或这种局部编辑输入，不调用 `new`。
WORK 记录当前输入；用 `gen WORK --input authoring` 或 `--input edit` 明确切换，不删除另一份输入。
`--check`、`--stdout`、`--diff` 的显式选择只对本次生效，不改变保存的选择。
默认落盘只读的 `PKG.resolved.toml` 和候选 SPEC，不改 checkout；
`--spec=.` 在补足 TOML 同级写 SPEC，`--spec=auto` 才写 checkout。
下载失败会说明原因而保留缺项；`--offline` 禁止下载，已有摘要不被默认覆盖。
补足不会猜版本、许可证或构建系统；局部 stage 的补足 TOML 不是整个 SPEC 的反向转换。

## 只读复验源码

```sh
ruyipack source verify --manifest work/example/example.toml --format toml
# 已有 SPEC：无需安装 RPM 或 curl
ruyipack source verify example
```

它重新下载全部远程 Source，包括已有摘要的项，分别报告匹配、不匹配、缺失或失败，
绝不补写或替换声明；本地材料不参与。全部适用项匹配才退出 0，否则退出 1。
发现不匹配先调查来源，不要直接重算覆盖。无法静态确定的宏会报告具体原因，
不会执行 Shell、Lua 或退回外部 RPM；必要时用 `-D 'archive_version 2.0'` 提供明确事实。

## 修改已有包

在工作区根目录或其子目录使用开发区名称：

```sh
ruyipack inspect busybox --editable --field package.version
ruyipack edit busybox --set package.version=1.37.1 --diff
ruyipack edit busybox --apply
ruyipack build busybox
ruyipack shell busybox
```

首次同名绑定要求已提交 main 中存在 `SPECS/NAME/NAME.spec`；
`--pkgname` 可明确指定不同包名。只读命令仅建立 `work/WORK/.config.toml`，
读 main（TOML 记录 commit），不建 checkout。写入时才建立稀疏 checkout。
已有开发区始终用保存绑定和当前分支，不重置或复制未提交的配方。
`edit busybox-test --pkgname busybox` 初次绑定后，后续只用 `busybox-test`。
构建结果在 `work/WORK/build`；`clean WORK` 不删作者 TOML、checkout 或分支。
详见 [构建说明](build.md)。外部文件必须显式指定：

```sh
ruyipack inspect --spec package.spec --editable --field package.version
ruyipack edit --spec package.spec --set package.version=2.0 --check --format toml
ruyipack edit --spec package.spec --set package.version=2.0 --diff
# 审阅后显式写回；可加 --expect-sha256 固定此前查看的原文。
ruyipack edit --spec package.spec --set package.version=2.0 --apply --format toml
```

完整视图不支持复杂构造时，选择需要的字段；未选内容保留原始字节。
条件歧义或 Source 隐式编号无法确定时，不要用猜测的编号绕过错误。
`edit WORK` 默认直接打开持久 TOML，不弹选择菜单、不自动检查、不改 SPEC。
`--menu` 先选字段再在终端修改原值；`--field package.version` 直接显示该字段原值供修改。
两者与编辑器共用同一份 TOML；`--set` 可无交互赋值。选字段后仍希望打开编辑器时，加 `--editor COMMAND`；
`--prepare DIR` 则只保存 TOML，不打开编辑器。
`--diff` 缓存候选 SPEC 并把 diff 放到输入同级，同时显示它；`--check` 编辑后检查。
`--apply` 默认检查局部准入，失败不发布。未选中的 SPEC 原文仍逐字节保留。
`--hash` 刻意刷新全部远程 Source（包括签名）；`--hash-source 0` 只刷新一个，
都依据候选中的版本和 URL，并把结果放回 stage。`source verify` 则只比较，绝不改声明。
普通编辑不联网；缺摘要在 authoring 策略中仍是警告，不代表已验证。


## check 通过后还要做什么？

- 用 `source verify` 复验已有摘要，并核对来源；摘要匹配不代表来源可信。
- 核对补丁是否适用，以及声明许可证是否符合上游实际内容。
- 在目标 openRuyi 环境验证 RPM 宏、依赖、构建、测试和产物文件归属。
- 最后审阅差异并提交；静态 pass 不是无人值守发布许可。

Version/Source URL 变化会产生复核提醒；TOML 的 `review_required` 是待办，
不是已经执行的检查。也不要把空复核列表理解成原生构建已通过。
仅应用自己可信工作区内的草稿。批次不是整体原子事务，失败后先查看已写文件。

更多字段、输出和边界见 [命令与 manifest 参考](reference.md)。

## 编辑器补全

运行 `ruyipack schema manifest > ruyipack.schema.json`，在手写 manifest 首行添加
`#:schema ./ruyipack.schema.json` 并空一行。Tombi 等 TOML 编辑器即可提供字段补全、
说明和结构诊断；升级工具后重新导出。缺摘要仍允许，宏和跨字段约束仍需
`gen WORK --offline --check`。这不是 `schema edit WORK --field FIELD` 的选中字段编辑投影。

普通 `edit` 可以保存未改变且有完整规则输入证据的遗留问题；修改后的非法值、
新增阻断或不完整检查仍拒绝。`edit --check` 预检同一保存门禁，独立 `check`
及 submit 策略仍严格。TOML v3 用 `success` 表示操作完成、`admissible` 表示
候选允许保存，`valid` 只表示整份候选静态检查通过；准备草稿时不输出 `valid` / `admissible`。

机器报告使用 `--format toml`，整份 stdout 是一个 TOML 文档；可选的未观测字段不输出，
编号 Source 观测用带 `number` 的记录数组。JSON Schema 仍遵循 JSON 标准；
build 的 TOML outcome 链接完整 `receipt.json`；backend/engine/host 等持久 JSON 回执迁移尚未实现，Docker 的 JSON 边界不变。
