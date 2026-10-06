<!--
SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>

SPDX-License-Identifier: MulanPSL-2.0
-->

# RuyiPack 快速入门

RuyiPack 帮助编写、修改和构建 openRuyi RPM 包。
这是需要维护者审阅的源码预览版，不会自动发现全部依赖。
静态编辑和检查不需要 Docker；构建使用 Docker Compose 中的 Mock。

## 安装

以下命令适用于 Ubuntu。先安装工具，再安装 Rust 和 RuyiPack：

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

首次编译需要联网获取固定工具链和锁定依赖。
确保 `$HOME/.cargo/bin` 在 `PATH` 中。普通用户不需要 REUSE 或 cargo-deny。
其他系统参考 [Rust 安装说明](https://rust-lang.org/tools/install/)。

## 选择工作方式

| 输入 | 流程 |
| --- | --- |
| 已有 SPEC | `open` 或 `edit` → `check` → `build` |
| 手写 TOML | `new` → `open --authoring` → `gen --diff` → `gen --apply` → `build` |

WORK 是开发区名称。`open ed`、`build ed` 和 `shell ed` 使用同一个开发区。
包名默认与 WORK 同名。用 `--pkgname` 建立不同名称的绑定。
后续命令使用保存的绑定，不必重复指定包名。

## 初始化工作区

在空目录运行：

```sh
ruyipack init .
```

配置保存在 `.ruyiconfig/config.toml`。默认配方仓库路径是 `openruyi`，包目录是 `SPECS`。
如需同时克隆配方仓库，在初始化命令中添加 `--clone URL`。
省略此选项时，初始化离线执行；Git 和 Docker 均不是必需工具。

首次初始化尝试读取 Git 全局姓名和邮箱，保存为 `author = "Name <email>"`。
缺失时留空，并提示修改位置。修改默认作者只影响以后的新模板。
已有或导入 TOML 的 `spec.contributors` 不会被覆盖。

重复 `init` 不覆盖配置。克隆失败时，配置和 Git 留下的文件仍保留。
先检查这些文件，再决定如何重试。详见[初始化规则](reference.md#workspace-initialization)。

## 编写新包

本地开发区不需要 Git 仓库：

```sh
ruyipack new example --build-system cmake
ruyipack open example --authoring
ruyipack gen example --diff
ruyipack gen example --apply
```

在 TOML 中填写版本、许可证、源码、依赖和文件列表。
未填必需项时，生成失败，不写 SPEC。工具不猜测这些事实。
`gen` 默认尝试计算缺失摘要；下载失败时说明原因并保留缺项。
已有摘要保持原值。用 `--offline` 禁止下载。

`gen --diff` 保存候选并显示差异，不改配方。
审阅后，`gen --apply` 检查并写入 `recipe/` 中的 SPEC。
`build` 只构建该 SPEC，不自动生成或应用 TOML。

### 导入已有输入

```sh
ruyipack new review --from-toml existing-authoring.toml
ruyipack new review-spec --from-dir /path/to/SPECS/ed
ruyipack open review-spec --authoring
ruyipack gen review-spec --offline --diff
```

TOML 导入保留原文。`--from-dir` 导入整个包目录；`--from-spec` 只导入一个 SPEC，不复制相邻材料。
目录必须有且只有一个顶层 SPEC；没有或多个时拒绝导入，可用 `--from-spec` 指定文件。
保留原 SPEC 文件名与 Name，不一致只给出警告。
SPEC 导入保留脚本和未映射内容，只将支持的字段放入 TOML。
它不是任意 SPEC 的完整逆向转换。
两种 SPEC 来源均默认使用 SPEC 文件名（去掉 `.spec`）绑定包，不使用源目录名。`--pkgname` 可指定目录绑定，但不改写 SPEC 的 Name。名称不一致或尚不能确定时，commit 给出警告；材料、过期和提交规范检查仍然执行。
工具不会批量替换脚本、URL 或 Patch 中的名称。

提交时准备配置指定的 Git 仓库，并切换到要提交的分支。运行 `ruyipack commit WORK --dry-run` 查看变化，再运行 `ruyipack commit WORK`。

## 升级已有包

首次使用下列流程时，配置仓库的已提交 main 必须包含 `SPECS/ed/ed.spec`：

```sh
ruyipack edit ed --set package.version=1.22.6 --hash --diff --apply
ruyipack build ed
```

请先确认目标版本存在。`--hash` 按修改后的 URL 刷新所有远程 Source，包括签名文件。
它计算摘要，不验证签名或来源真实性。下载或准入失败时不写 SPEC。
`--apply` 已包含局部编辑检查，不必再加 `--check`。

如需先审阅，第一条命令去掉 `--apply`。确认后运行：

```sh
ruyipack edit ed --apply
```

首次只读操作仅保存 WORK 绑定，读取已提交 main。
编辑时将包文件复制到 `work/WORK/recipe/SPECS/PKG/`；已有文件不会被重置，不创建 Git 分支。
查看相对于 Git 基线的全部改动：

```sh
ruyipack commit ed --dry-run
```

### 选择编辑方式

- `open WORK`：直接编辑配方目录中的 SPEC 或 Patch。
- `edit WORK`：在编辑器中修改 WORK 的唯一 TOML。
- `edit WORK --menu`：选择字段，再在终端修改原值。
- `edit WORK --field package.version`：在终端修改指定字段。
- `edit WORK --set FIELD=VALUE`：无交互赋值。

`edit` 默认不检查、不写 SPEC。加 `--check` 检查候选；加 `--apply` 检查并发布。
`--diff` 保存候选 SPEC 和`.cache/` 中的 diff，并显示差异。
候选保留未选中的原始字节。歧义字段必须先解决，不能靠猜测绕过检查。

普通编辑允许保留已确认、未改变的遗留问题。
修改后的非法值、新增阻断和不完整检查仍阻止发布。
独立 `check` 判断整份输入；通过不代表构建成功。

外部文件必须显式选择：

```sh
ruyipack inspect --spec package.spec --editable --field package.version
ruyipack edit --spec package.spec --set package.version=2.0 --diff
```

`edit WORK` 和 `gen WORK` 读取同一份 TOML；不用再选择输入来源。
候选和补足 TOML 都不是隐式输入。完整规则见[命令参考](reference.md)。

## 复验材料

```sh
ruyipack source verify ed
ruyipack check ed --materials
```

`source verify` 重新下载远程 Source，对比已声明摘要，不改声明。
不匹配时先调查来源，不要直接重算覆盖。
`check --materials` 离线检查已准备的 Source/Patch 文件。
缺摘要在 authoring 策略中是警告，不代表材料已经验证。

静态检查通过后，仍需确认来源、许可证、补丁适用性和目标环境中的构建结果。
版本或 URL 变化产生的 `review_required` 是待办，不是检查结果。
仅使用可信的本地草稿。批量发布可能部分成功；失败后先查看已写文件。

## 调试补丁与重复构建

在 `.ruyiconfig/config.toml` 配置编辑器：

```toml
editor = "code --wait"
```

这是可信 shell 命令，不要复制未知来源的配置。
未配置时沿用 Git 的编辑器选择；`--editor` 可临时覆盖。GUI 编辑器必须等待退出。

```sh
ruyipack open ed
ruyipack build ed --stage prep
ruyipack shell ed
ruyipack build ed
```

`open` 不自动检查或提交。`shell` 需要保留的可用 Mock chroot。
现场修改不会自动回写配方；下一次构建使用新环境验证。
成功的 prep 可作为 `shell --export-patch` 的基线，失败的补丁应用现场不能。
详见[Patch 导出](build.md#prepare-debug-export-a-patch)。

重复构建不必先 clean。旧结果会归档；用以下命令选择清理范围：

```sh
ruyipack clean ed --history --force
ruyipack clean ed --force
```

它们保留配方、TOML、sources 和开发分支。
删除整个开发区前，先运行 `delete ed --dry-run`。
Git 开发区有未提交或未合并修改时，删除会被拒绝。

## 编辑器与机器接口

导出 authoring schema：

```sh
ruyipack schema manifest > ruyipack.schema.json
```

在 manifest 首行添加 `#:schema ./ruyipack.schema.json`，下一行留空。
Tombi 等编辑器可提供补全和结构诊断。升级后重新导出 schema。
宏与跨字段约束仍需 `gen WORK --offline --check`。
`schema edit` 则描述某份 SPEC 的选中字段，不是 authoring schema。

`--format toml` 输出机器报告；stdout 是一个 TOML 文档。
`success` 表示操作完成，`valid` 表示静态检查通过，`admissible` 表示允许局部保存。
未执行的检查不输出成功值。构建报告链接 JSON 回执，schema 仍使用 JSON。
用 `--debug` 查看内部诊断；它不改变操作行为。
