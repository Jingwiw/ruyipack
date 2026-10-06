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
import shutil
import subprocess
import sys
import time
import urllib.request
import xml.etree.ElementTree as ET


def select_repository(target, deadline):
    """Choose one complete repository before any chroot dependency transaction."""
    attempts = []
    for url in dict.fromkeys([target["repository"], target["repository_fallback"]]):
        record = {"url": url}
        attempts.append(record)
        try:
            if not url.startswith("https://"):
                raise ValueError("build repositories must use HTTPS")
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("repository selection budget exhausted")
            with urllib.request.urlopen(url.rstrip("/") + "/repodata/repomd.xml",
                                        timeout=min(10, remaining)) as response:
                if not response.url.startswith("https://"):
                    raise ValueError("repository metadata redirected away from HTTPS")
                data = response.read(4 * 1024 * 1024 + 1)
                if len(data) > 4 * 1024 * 1024:
                    raise ValueError("repository metadata exceeds 4 MiB")
                root = ET.fromstring(data)
                if root.tag != "{http://linux.duke.edu/metadata/repo}repomd" or not root.findall("{*}data"):
                    raise ValueError("repository did not return RPM metadata")
                record.update(status=response.status, metadata_url=response.url,
                              repomd_sha256=hashlib.sha256(data).hexdigest())
                return {"selected": url, "fallback": url != target["repository"], "attempts": attempts}
        except (OSError, ValueError, ET.ParseError) as error:
            record["error"] = str(error)
    return {"selected": None, "fallback": False, "attempts": attempts}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--spec", type=Path, required=True)
    parser.add_argument("--sources", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--timeout", type=int, required=True)
    parser.add_argument("--stage", choices=("prep", "build"), default="build")
    args = parser.parse_args()
    output = args.output
    output.mkdir(parents=True, exist_ok=True)
    deadline = time.monotonic() + args.timeout
    receipt = {"format_version": 1, "engine": "mock", "success": False,
               "target_stage": args.stage, "stages": [], "artifacts": [], "failure": None, "collection_errors": []}
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
                # Tail regular log files, not pipes: descendants cannot hold an EOF open.
                try:
                    with stdout.open("rb") as out_read, stderr.open("rb") as err_read:
                        streams = [(out_read, sys.stdout.buffer), (err_read, sys.stderr.buffer)]
                        def relay():
                            if stage not in ("srpm", "prep", "rebuild"):
                                return
                            for reader, display in streams:
                                end = os.fstat(reader.fileno()).st_size
                                while reader.tell() < end:
                                    chunk = reader.read(min(65536, end - reader.tell()))
                                    if not chunk:
                                        break
                                    try:
                                        display.write(chunk)
                                        display.flush()
                                    except BrokenPipeError:
                                        pass  # Logs and process cleanup remain authoritative.
                        try:
                            while True:
                                relay()
                                remaining = deadline - time.monotonic()
                                if remaining <= 0:
                                    raise subprocess.TimeoutExpired(argv, 0)
                                try:
                                    record["exit_code"] = process.wait(timeout=min(0.05, remaining))
                                    break
                                except subprocess.TimeoutExpired:
                                    continue
                        finally:
                            relay()
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
        bootstrap = Path("/etc/ruyipack-bootstrap-repository")
        if bootstrap.is_file():
            receipt["bootstrap_repository"] = bootstrap.read_text().strip()
        receipt["input_mock_config_sha256"] = digest(config)
        configured = config.read_text()
        # Evaluate Mock's actual configuration rather than grepping Python source.
        configured += "\nif config_opts['target_arch'] != " + repr(receipt["target"]["architecture"]) + ": raise ValueError('Mock target_arch differs from build target')\n"
        preflight = output / "mock-preflight.cfg"
        preflight.write_text(configured)
        required("target-config", ["mock", "-r", str(preflight), "--debug-config"])
        if receipt["target"].get("repository_fallback"):
            selection = select_repository(receipt["target"], deadline)
            receipt["repository_selection"] = selection
            save()
            if selection["selected"] is None:
                raise RuntimeError("all configured repositories are unavailable; see repository_selection")
            # The shipped profile has exactly one repository. Override the complete
            # dnf configuration, not a mirror list that could mix package origins.
            selected = selection["selected"]
            configured += "\n" + "\n".join([
                "from configparser import ConfigParser",
                "from io import StringIO",
                "selected_repos = ConfigParser(interpolation=None)",
                "selected_repos.read_string(config_opts['dnf.conf'])",
                "if set(selected_repos.sections()) != {'main', 'openruyi'}: raise ValueError('repository fallback requires the single-repository profile')",
                "selected_repos['openruyi']['baseurl'] = " + repr(selected),
                "selected_repos['openruyi'].pop('mirrorlist', None)",
                "selected_repos['openruyi'].pop('metalink', None)",
                "selected_config = StringIO()",
                "selected_repos.write(selected_config)",
                "config_opts['dnf.conf'] = selected_config.getvalue()",
                "config_opts['macros']['%_vendor_repo_url'] = " + repr(selected),
            ]) + "\n"
            print("repository: " + selected + (" (fallback; primary unavailable)" if selection["fallback"] else ""), flush=True)
        config = output / "mock.cfg"
        config.write_text(configured)
        receipt["mock_config_sha256"] = digest(config)
        (output / "target.json").write_bytes(target.read_bytes())
        # The environment supplies a privilege-aware mock entry point.
        mock = ["mock", "-r", str(config)]
        if receipt.get("repository_selection"):
            # The worker retains this identity outside /input and /output. A change
            # of origin requires discarding installed dependencies and cached RPMs.
            origin = Path.home() / ".ruyipack-repository"
            selected = receipt["repository_selection"]["selected"]
            if not origin.is_file() or origin.read_text() != selected:
                required("repository-clean", [*mock, "--scrub", "all"])
                origin.write_text(selected)
        required("environment", ["rpm", "-qa", "--qf", "%{NAME}\t%{EPOCHNUM}:%{VERSION}-%{RELEASE}\t%{ARCH}\n"])
        required("mock-version", [mock[0], "--version"])
        required("effective-config", [*mock, "--debug-config"])
        # Discard prior builds/debug sessions once; both stages share this fresh chroot.
        required("clean", [*mock, "--clean"])
        required("srpm", [*mock, "--resultdir", str(output / "srpm"),
                          "--no-clean", "--no-cleanup-after", "--buildsrpm", "--spec", str(args.spec), "--sources", str(args.sources)])
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
        if args.stage == "prep":
            required("prep", [*mock, "--resultdir", str(output / "prep"), "--no-clean", "--no-cleanup-after",
                              "--rebuild", str(srpms[0]), "--short-circuit", "prep"])
            root = Path(required("root-path", [*mock, "--print-root-path"]).read_text().strip())
            builddir = required("build-directory", [*mock, "--chroot", "--", "rpm", "--eval", "%{_builddir}"]).read_text().strip()
            if not builddir.startswith("/") or ".." in Path(builddir).parts:
                raise RuntimeError("native build directory is not an absolute confined path")
            prepared = root / builddir.lstrip("/")
            if not prepared.is_dir():
                raise RuntimeError("prepared source directory is absent")
            shutil.copytree(prepared, output / "prep-baseline", symlinks=True)
            receipt["prepared_directory"] = builddir
        else:
            required("rebuild", [*mock, "--resultdir", str(output / "rpm"), "--no-clean", "--no-cleanup-after", "--rebuild", str(srpms[0])])
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


if __name__ == "__main__":
    sys.exit(main())
