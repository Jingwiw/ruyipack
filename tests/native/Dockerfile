# SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
# SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
# SPDX-FileContributor: Jingwiw <wangjingwei@iscas.ac.cn>
#
# SPDX-License-Identifier: MulanPSL-2.0

FROM ghcr.io/openruyi-project/creek-x86_64@sha256:8306d5fc6ecfab85fe9c604d97f84bda633a88ff72b02648353fe9ea6c6ce49b
# Pin the semantics under test, not a Fedora substitute. A removed package is
# an environment failure; do not silently upgrade RPM or the distribution macros.
RUN dnf -y --setopt=install_weak_deps=False install \
    rpm-6.0.1-18.54.or rpm-build-6.0.1-18.54.or \
    rpm-config-openruyi-20251216-33.3.or python3 tar gzip patch \
    && dnf clean all
