# SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
# SPDX-License-Identifier: MulanPSL-2.0

"""Run the upstream script unchanged; preserve every selected check result."""
import hashlib
import json
import shutil
from pathlib import Path
import subprocess
import tomllib
import tempfile

root = Path("/input")
output = Path("/output")
output.mkdir(exist_ok=True)
selection = tomllib.loads((root / "selection.toml").read_text())
for path, identity in selection.items():
    if hashlib.sha256((root / path).read_bytes()).hexdigest() != identity["sha256"]:
        raise RuntimeError("check input changed: " + path)
with (output / "tools.log").open("wb") as log:
    for argv in (["rpm", "--version"], ["curl", "--version"], ["enosys", "--version"], ["git", "--version"], ["pre-commit", "--version"]):
        subprocess.run(argv, stdout=log, stderr=log, check=True)
results = []
(output / "results.toml").write_text("checks = []\n")

def run_check(path, argv, cwd, check_changes=False):
    index = len(results)
    stdout, stderr = f"{index}.stdout.log", f"{index}.stderr.log"
    with (output / stdout).open("wb") as out, (output / stderr).open("wb") as err:
        result = subprocess.run(argv, cwd=cwd, stdout=out, stderr=err, check=False)
        code = result.returncode
        if check_changes:
            current = {}
            for file in Path(cwd).rglob("*"):
                relative = file.relative_to(cwd)
                if ".git" in relative.parts or file.is_dir():
                    continue
                if file.is_symlink():
                    raise RuntimeError("Check created a symbolic link: " + str(relative))
                current[str(relative)] = dict(sha256=hashlib.sha256(file.read_bytes()).hexdigest(), executable=bool(file.stat().st_mode & 0o111))
            if current != selection:
                code = 1
                err.write(b"Check modified input files. Changes were not published.\n")
                (output / "pre-commit.diff").write_bytes(subprocess.check_output(["git", "diff"], cwd=cwd))
    values = dict(path=path, exit_code=code, stdout=stdout, stderr=stderr)
    results.append("[[checks]]\n" + "".join(f"{key} = {json.dumps(value, ensure_ascii=False)}\n" for key, value in values.items()))
    temporary = output / "results.tmp"
    temporary.write_text("\n".join(results))
    temporary.replace(output / "results.toml")


with tempfile.TemporaryDirectory(prefix="pre-commit-") as scratch:
    for path in selection:
        target = Path(scratch) / path
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(root / path, target)
    # Git is only a disposable file index for pre-commit, not a publication repository.
    subprocess.run(["git", "init", "--quiet"], cwd=scratch, check=True)
    subprocess.run(["git", "add", "--force", "."], cwd=scratch, check=True)
    files = [path for path in selection if path.startswith("SPECS/")]
    run_check(".pre-commit-config.yaml", ["pre-commit", "run", "--show-diff-on-failure", "--files", *files], scratch, True)

with tempfile.TemporaryDirectory(prefix="remoteasset-") as scratch:
    (Path(scratch) / "SPECS").symlink_to(root / "SPECS", target_is_directory=True)
    for path in selection:
        if path.startswith("SPECS/") and path.endswith(".spec"):
            run_check(path, ["python3", str(root / "scripts/remoteassetify.py"), "--workflow", path], scratch)
