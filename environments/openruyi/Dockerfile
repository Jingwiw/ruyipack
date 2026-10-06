# SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
# SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
# SPDX-License-Identifier: MulanPSL-2.0

FROM ghcr.io/openruyi-project/creek-x86_64@sha256:8306d5fc6ecfab85fe9c604d97f84bda633a88ff72b02648353fe9ea6c6ce49b
COPY target.json /etc/ruyipack-target.json
# The pinned minimal base has no Python yet. Read only these shipped URL lines.
# Bootstrap selects one repository; never add the fallback alongside Base.
RUN repo=$(sed -n 's/^[[:space:]]*"repository":[[:space:]]*"\([^"]*\)".*/\1/p' /etc/ruyipack-target.json) \
    && test -n "$repo" \
    && if ! dnf --repo=ruyipack-bootstrap --repofrompath=ruyipack-bootstrap,"$repo" --setopt=timeout=30 --setopt=retries=1 --refresh makecache; then \
         repo=$(sed -n 's/^[[:space:]]*"repository_fallback":[[:space:]]*"\([^"]*\)".*/\1/p' /etc/ruyipack-target.json); \
       fi \
    && test -n "$repo" \
    && printf '%s\n' "$repo" > /etc/ruyipack-bootstrap-repository \
    && dnf --repo=ruyipack-bootstrap --repofrompath=ruyipack-bootstrap,"$repo" -y --setopt=install_weak_deps=False install mock dnf5-plugins rpm-build rpm-config-openruyi python3 tar gzip patch sudo \
    && dnf clean all \
    && mkdir -p /input /output \
    && useradd -m -u 1000 -G mock builder \
    && chown builder:builder /output \
    && printf 'builder ALL=(root) NOPASSWD: /usr/libexec/mock/mock\n' > /etc/sudoers.d/mock \
    && printf '#!/bin/sh\nexec sudo -n /usr/libexec/mock/mock "$@"\n' > /usr/local/bin/mock \
    && chmod 755 /usr/local/bin/mock
COPY openruyi.cfg /etc/mock/openruyi.cfg
USER builder
CMD ["sleep", "infinity"]
