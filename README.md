<!--
SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>

SPDX-License-Identifier: MulanPSL-2.0
-->

# RuyiPack

RuyiPack is a project for openRuyi RPM package workflows.

Preview a SPEC from an Autotools manifest:

```sh
cargo run -- gen ed --manifest examples/ed/ed.toml
```

The command reads the authoring fields and prints a SPEC candidate to stdout.
