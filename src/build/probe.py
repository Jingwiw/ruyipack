# SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
# SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
# SPDX-License-Identifier: MulanPSL-2.0
"""Probe this worker only; never register binfmt or change the daemon host."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile


def run(argv):
    return subprocess.check_output(argv, text=True, stderr=subprocess.STDOUT, timeout=10).strip()


def normalize(arch):
    return {"amd64": "x86_64", "arm64": "aarch64"}.get(arch, arch)


facts = {"translator": "unknown", "mount": False, "chroot": False}
try:
    facts["target_arch"] = json.loads(Path(os.environ["BUILD_TARGET"]).read_text())["architecture"]
    facts["rpm_arch"] = run(["rpm", "--eval", "%{_arch}"])
    facts["image_arch"] = sys.argv[1]
    if len({normalize(facts[key]) for key in ("target_arch", "rpm_arch", "image_arch")}) != 1:
        raise ValueError("target, worker image and RPM architectures disagree")
    with tempfile.TemporaryDirectory(prefix="ruyipack-probe-") as directory:
        run(["mount", "-t", "tmpfs", "-o", "size=1m", "tmpfs", directory])
        try:
            facts["mount"] = True
            run([sys.executable, "-c", "import os,sys; os.chroot(sys.argv[1]); os.chdir('/')", directory])
            facts["chroot"] = True
        finally:
            run(["umount", directory])
except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
    facts["error"] = str(error)
    if isinstance(error, subprocess.CalledProcessError):
        facts["diagnostic"] = error.output
    print(json.dumps(facts), flush=True)
    sys.exit(1)
print(json.dumps(facts), flush=True)
