#!/usr/bin/env python3
# SPDX-FileCopyrightText: (C) 2026 Institute of Software, Chinese Academy of Sciences (ISCAS)
# SPDX-FileCopyrightText: (C) 2026 openRuyi Project Contributors
# SPDX-License-Identifier: MulanPSL-2.0

"""Mock engine. The host supplies files and executes argv; this module owns RPM semantics."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--spec", type=Path, required=True)
    parser.add_argument("--sources", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--timeout", type=int, required=True)
    args = parser.parse_args()
    output = args.output
    output.mkdir(parents=True, exist_ok=True)
    deadline = time.monotonic() + args.timeout
    receipt = {"format_version": 1, "engine": "mock", "success": False,
               "stages": [], "artifacts": [], "failure": None, "collection_errors": []}
    os.environ["LC_ALL"] = "C"

    def save():
        temporary = output / "receipt.json.tmp"
        temporary.write_text(json.dumps(receipt, indent=2) + "\n")
        temporary.replace(output / "receipt.json")

    def run(stage, argv):
        index = len(receipt["stages"])
        stdout = output / f"{index:02}-{stage}.stdout"
        stderr = output / f"{index:02}-{stage}.stderr"
        record = {"stage": stage, "argv": list(map(str, argv)), "exit_code": None,
                  "stdout": stdout.name, "stderr": stderr.name,
                  "status": "running", "started_unix": time.time()}
        receipt["stages"].append(record)
        save()
        print(f"{stage}: starting", flush=True)
        try:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("engine execution budget exhausted")
            with stdout.open("wb") as out, stderr.open("wb") as err:
                process = subprocess.Popen(argv, stdin=subprocess.DEVNULL, stdout=out, stderr=err,
                                           start_new_session=True)
                try:
                    record["exit_code"] = process.wait(timeout=remaining)
                except subprocess.TimeoutExpired:
                    # sudo/Mock descendants may have different credentials or sessions.
                    # Never wait indefinitely here; the backend stops the entire worker.
                    try:
                        os.killpg(process.pid, signal.SIGKILL)
                    except OSError as error:
                        record["termination_error"] = str(error)
                    try:
                        record["exit_code"] = process.wait(timeout=1)
                    except subprocess.TimeoutExpired:
                        record["termination_error"] = "child did not exit after termination"
                    raise TimeoutError("engine execution budget exhausted")
            record["status"] = "passed" if record["exit_code"] == 0 else "failed"
        except (OSError, TimeoutError) as error:
            record["status"] = "timed-out" if isinstance(error, TimeoutError) else "unavailable"
            record["error"] = str(error)
        finally:
            record["finished_unix"] = time.time()
            save()
        return record["status"] == "passed", stdout

    def required(stage, argv):
        ok, path = run(stage, argv)
        if not ok:
            raise RuntimeError(f"{stage} failed; see the recorded exit status and raw logs")
        return path

    def digest(path):
        with path.open("rb") as stream:
            return hashlib.file_digest(stream, "sha256").hexdigest()

    mock = None
    try:
        if sys.version_info < (3, 11):
            raise RuntimeError("Mock engine requires Python 3.11 or newer")
        config = Path(os.environ["MOCK_CONFIG"])
        target = Path(os.environ["BUILD_TARGET"])
        receipt["target"] = json.loads(target.read_text())
        receipt["mock_config_sha256"] = digest(config)
        (output / "mock.cfg").write_bytes(config.read_bytes())
        (output / "target.json").write_bytes(target.read_bytes())
        # The environment supplies a privilege-aware mock entry point.
        mock = ["mock", "-r", str(config)]
        required("environment", ["rpm", "-qa", "--qf", "%{NAME}\t%{EPOCHNUM}:%{VERSION}-%{RELEASE}\t%{ARCH}\n"])
        required("mock-version", [mock[0], "--version"])
        required("effective-config", [*mock, "--debug-config"])
        required("srpm", [*mock, "--resultdir", str(output / "srpm"),
                          "--no-cleanup-after", "--buildsrpm", "--spec", str(args.spec), "--sources", str(args.sources)])
        srpms = sorted((output / "srpm").glob("*.src.rpm"))
        if len(srpms) != 1:
            raise RuntimeError(f"expected one SRPM, found {len(srpms)}")
        def record_artifact(package):
            query = required("artifact-query", ["rpm", "-qp", "--qf", "%{NAME}\t%{VERSION}\t%{RELEASE}\t%{ARCH}\n", str(package)])
            identity = query.read_text().strip()
            if len(identity.split("\t")) != 4 or "%" in identity:
                raise RuntimeError(f"invalid native RPM identity: {identity!r}")
            receipt["artifacts"].append({"path": str(package.relative_to(output)),
                "size": package.stat().st_size, "sha256": digest(package), "identity": identity})
            save()

        # A reusable SRPM remains identified even if dependency preparation or rebuild fails.
        record_artifact(srpms[0])
        required("rebuild", [*mock, "--resultdir", str(output / "rpm"), "--no-cleanup-after", "--rebuild", str(srpms[0])])
        rpms = sorted((output / "rpm").glob("*.rpm"))
        if not any(not p.name.endswith((".src.rpm", ".nosrc.rpm")) for p in rpms):
            raise RuntimeError("Mock returned success without a binary RPM")
        for package in rpms:
            record_artifact(package)
        receipt["success"] = True
    except (OSError, KeyError, ValueError, RuntimeError) as error:
        receipt["failure"] = str(error)
    finally:
        # A failed build still has useful dependency and macro evidence. Collection
        # never overwrites the primary failure and remains within the same budget.
        if mock and deadline > time.monotonic():
            ok, root_output = run("root-path", [*mock, "--print-root-path"])
            if ok:
                root = Path(root_output.read_text().strip())
                if root.is_dir():
                    ok, packages = run("installed-packages", [*mock, "--chroot", "--", "rpm", "-qa", "--qf",
                        "%{NAME}\t%{EPOCHNUM}:%{VERSION}-%{RELEASE}\t%{ARCH}\n"])
                    if ok:
                        rows = [line.split("\t") for line in packages.read_text().splitlines()]
                        if rows and all(len(row) == 3 and all(row) for row in rows):
                            receipt["installed_packages"] = [dict(zip(("name", "evr", "arch"), row)) for row in sorted(rows)]
                        else:
                            receipt["collection_errors"].append("invalid native installed-package query output")
                    else:
                        receipt["collection_errors"].append("could not query the installed buildroot packages")
                else:
                    receipt["collection_errors"].append("buildroot is absent; installed package evidence unavailable")
            else:
                receipt["collection_errors"].append("could not locate the buildroot")
        if receipt["success"] and (receipt["collection_errors"] or not receipt.get("installed_packages")):
            receipt["success"] = False
            receipt["failure"] = "build finished but required environment evidence is incomplete"
        save()
    return 0 if receipt["success"] else 1


def shell():
    mock = ["mock", "-r", os.environ["MOCK_CONFIG"]]
    def query(argv):
        return subprocess.check_output(argv, text=True, timeout=30).strip()
    root = Path(query([*mock, "--print-root-path"]))
    if not root.is_dir():
        raise RuntimeError("the retained Mock chroot is absent; build the package first")
    directory = query([*mock, "--chroot", "--", "rpm", "--eval", "%{_builddir}"])
    if not directory.startswith("/") or not (root / directory.lstrip("/")).is_dir():
        raise RuntimeError("the native RPM build directory does not exist yet")
    # Enter Mock's actual chroot, not merely the outer worker container.
    os.execvp(mock[0], [*mock, "--shell", "--cwd", directory])


if __name__ == "__main__":
    if sys.argv[1:] == ["--shell"]:
        try:
            shell()
        except (OSError, KeyError, RuntimeError, subprocess.SubprocessError) as error:
            print(f"shell: {error}", file=sys.stderr)
            sys.exit(1)
    else:
        sys.exit(main())
