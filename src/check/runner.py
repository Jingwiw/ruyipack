# SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
# SPDX-License-Identifier: MulanPSL-2.0

"""Run the upstream script unchanged; preserve every selected check result."""
import hashlib
import json
from pathlib import Path
import subprocess
import tomllib
import tempfile

root = Path("/input")
output = Path("/output")
output.mkdir(exist_ok=True)
selection = tomllib.loads((root / "selection.toml").read_text())
for item in [selection["script"], *selection["inputs"]]:
    if hashlib.sha256((root / item["path"]).read_bytes()).hexdigest() != item["sha256"]:
        raise RuntimeError("check input changed: " + item["path"])
with (output / "tools.log").open("wb") as log:
    for argv in (["rpm", "--version"], ["curl", "--version"], ["enosys", "--version"]):
        subprocess.run(argv, stdout=log, stderr=log, check=True)
results = []
(output / "results.toml").write_text("checks = []\n")
with tempfile.TemporaryDirectory(prefix="remoteasset-") as scratch:
    # The upstream script downloads into cwd. Keep downloads out of frozen input.
    (Path(scratch) / "SPECS").symlink_to(root / "SPECS", target_is_directory=True)
    for index, item in enumerate(selection["inputs"]):
        stdout, stderr = f"{index}.stdout.log", f"{index}.stderr.log"
        with (output / stdout).open("wb") as out, (output / stderr).open("wb") as err:
            result = subprocess.run(["python3", str(root / selection["script"]["path"]), "--workflow", item["path"]], cwd=scratch, stdout=out, stderr=err, check=False)
        values = dict(path=item["path"], exit_code=result.returncode, stdout=stdout, stderr=stderr)
        results.append("[[checks]]\n" + "".join(f"{key} = {json.dumps(value, ensure_ascii=False)}\n" for key, value in values.items()))
        temporary = output / "results.tmp"
        temporary.write_text("\n".join(results))
        temporary.replace(output / "results.toml")
