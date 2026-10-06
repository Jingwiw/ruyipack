#!/usr/bin/env python3
# SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
# SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
# SPDX-License-Identifier: MulanPSL-2.0

"""Enter the retained Mock chroot; independent of its original build script."""
import os
import shlex
import subprocess
import sys
from pathlib import Path


def shell(command=()):
    mock = ["mock", "--quiet", "-r", "/output/mock.cfg"]
    def query(argv):
        return subprocess.check_output(argv, text=True, timeout=30).strip()
    root = Path(query([*mock, "--print-root-path"]))
    if not root.is_dir():
        raise RuntimeError("the retained Mock chroot is absent; build the package first")
    # Resolve the native directory inside the same chroot entry as the command.
    # Mock joins --shell arguments before invoking /bin/sh; quote at that boundary.
    enter = r"""directory=$(timeout --kill-after=5 30 rpm --eval '%{_builddir}') || exit
case "$directory" in
    /*) [ -d "$directory" ] ;;
    *) false ;;
esac || { echo "shell: the native RPM build directory does not exist yet" >&2; exit 1; }
cd -- "$directory" || exit
exec "$@"
"""
    argv = [*mock, "--shell", "--", shlex.join([
        "/bin/sh", "-c", enter, "ruyipack-shell", *(command or ["/bin/sh", "-i", "-l"]),
    ])]
    os.execvp(mock[0], argv)

if __name__ == "__main__":
    try:
        if sys.argv[1:] and sys.argv[1] != "--":
            raise ValueError("shell command must follow --")
        shell(sys.argv[2:])
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        print(f"shell: {error}", file=sys.stderr)
        sys.exit(1)
