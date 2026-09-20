<!--
SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>

SPDX-License-Identifier: MulanPSL-2.0
-->

# RuyiPack 快速入门

这是需要维护者审阅的源码预览版。它帮助减少 SPEC 的填写和修改工作，
不会自动推断全部依赖、下载源码或构建软件。

## 从新包开始

在空工作目录运行：

```sh
ruyipack init example --build-system cmake
# 编辑 example.toml：确认上游信息、源码、依赖、构建选项和文件列表。
ruyipack gen example --stdout
ruyipack gen example
```

脚手架故意保留未知必填项，未填写时生成失败且不写 SPEC。
本地包名检查没有发现冲突，不代表上游不存在同名包。

## 修改已有包

```sh
ruyipack check example.spec --format json
ruyipack edit example.spec --field package.version --view
ruyipack edit example.spec --set package.version=2.0 --diff
# 审阅差异、源码和补丁后再显式写回：
ruyipack edit example.spec --set package.version=2.0
```

完整视图不支持复杂构造时，选择需要的字段；未选内容保留原始字节。
条件歧义或 Source 隐式编号无法确定时，不要用猜测的编号绕过错误。
没有 `sha256` 字段的旧 Source 不能通过固定形状草稿直接添加摘要。

## check 通过后还要做什么？

- 核对源代码来源、实际下载内容和摘要；工具不会刷新摘要。
- 核对补丁是否适用，以及声明许可证是否符合上游实际内容。
- 在目标 openRuyi 环境验证 RPM 宏、依赖、构建、测试和产物文件归属。
- 最后审阅差异并提交；静态 pass 不是无人值守发布许可。

Version/Source URL 变化会产生复核提醒；JSON 的 `review_required` 是待办，
不是已经执行的检查。也不要把空复核列表理解成原生构建已通过。
仅应用自己可信工作区内的草稿。批次不是整体原子事务，失败后先查看已写文件。

更多字段、输出和边界见 [命令与 manifest 参考](reference.md)。
