#!/usr/bin/env python3
# SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
# SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
# SPDX-License-Identifier: MulanPSL-2.0

"""Export selected source changes against the retained preparation baseline."""
import argparse
import difflib
import json
import subprocess
import sys
from pathlib import Path


def export_patch():
    parser = argparse.ArgumentParser(description="Export explicit text changes against a prepared source baseline")
    parser.add_argument("--source-root", type=Path, required=True)
    parser.add_argument("--path", type=Path, action="append", required=True)
    args = parser.parse_args()
    receipt = json.loads(Path("/output/receipt.json").read_text())
    if not receipt.get("success") or receipt.get("target_stage") != "prep":
        raise RuntimeError("export requires a successful build --stage prep baseline")
    mock = ["mock", "-r", "/output/mock.cfg"]
    root = Path(subprocess.check_output([*mock, "--print-root-path"], text=True, timeout=30).strip())
    directory = receipt["prepared_directory"]
    if not directory.startswith("/") or ".." in Path(directory).parts:
        raise RuntimeError("invalid prepared source directory")

    def confined(base, relative):
        if relative.is_absolute() or ".." in relative.parts or any(c.isspace() for c in str(relative)):
            raise RuntimeError("source paths must stay inside the selected source subtree")
        path = base
        for part in relative.parts:
            path = path / part
            if path.is_symlink():
                raise RuntimeError(f"symbolic link is not an editable text file: {path}")
        return path

    before = confined(Path("/output/prep-baseline"), args.source_root)
    after = confined(root / directory.lstrip("/"), args.source_root)
    changes = []
    for relative in dict.fromkeys(args.path):
        old = confined(before, relative)
        new = confined(after, relative)
        if not old.exists() and not new.exists():
            raise RuntimeError(f"selected file is absent from baseline and working source: {relative}")
        def lines(path):
            if not path.exists():
                return []
            if not path.is_file():
                raise RuntimeError(f"select regular text files, not directories: {path}")
            with path.open(encoding="utf-8", newline="") as stream:
                content = stream.read()
            if "\x00" in content or (content and not content.endswith("\n")):
                raise RuntimeError(f"binary or non-newline-terminated file requires manual Patch export: {path}")
            return content.splitlines(keepends=True)
        changes.extend(difflib.unified_diff(lines(old), lines(new),
                       fromfile=f"a/{relative}" if old.exists() else "/dev/null",
                       tofile=f"b/{relative}" if new.exists() else "/dev/null"))
    if not changes:
        raise RuntimeError("selected files have no text changes")
    sys.stdout.writelines(changes)

if __name__ == "__main__":
    try:
        export_patch()
    except (OSError, KeyError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        print(f"export: {error}", file=sys.stderr)
        sys.exit(1)
