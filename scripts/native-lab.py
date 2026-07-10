#!/usr/bin/env python3
"""Reserve native test resources and classify infrastructure failures."""

import argparse
import datetime
import hashlib
import json
import os
import platform
import shutil
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path


INFRASTRUCTURE_UNAVAILABLE = 75
RESERVED_KINDS = {"android", "ios-device", "ios-simulator", "local", "ssh"}


def probe(command, timeout=15):
    try:
        completed = subprocess.run(
            command,
            check=False,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            timeout=timeout,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        return False, str(error)
    return completed.returncode == 0, completed.stdout.strip()


def check_health(spec):
    kind, separator, value = spec.partition(":")
    if not separator or not value:
        return {"spec": spec, "available": False, "detail": "expected kind:value"}
    if kind == "command":
        path = shutil.which(value)
        return {"spec": spec, "available": bool(path), "allocation": path or ""}
    if kind == "env":
        allocation = os.environ.get(value, "")
        return {"spec": spec, "available": bool(allocation), "allocation": allocation}
    if kind == "local":
        actual = platform.system().lower()
        expected = {"macos": "darwin"}.get(value.lower(), value.lower())
        return {"spec": spec, "available": actual == expected, "allocation": actual}
    if kind == "docker":
        ok, detail = probe(["docker", "info", "--format", "{{.ServerVersion}}"])
        return {"spec": spec, "available": ok, "allocation": value, "detail": detail[-1000:]}
    if kind == "ssh":
        ok, detail = probe(
            ["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=5", value, "exit 0"],
            timeout=10,
        )
        return {"spec": spec, "available": ok, "allocation": value, "detail": detail[-1000:]}
    if kind == "ios-simulator":
        if platform.system() != "Darwin" or not shutil.which("xcrun"):
            return {"spec": spec, "available": False, "detail": "xcrun requires macOS"}
        ok, output = probe(["xcrun", "simctl", "list", "devices", "available", "--json"])
        if not ok:
            return {"spec": spec, "available": False, "detail": output}
        try:
            devices = [
                device
                for runtime in json.loads(output).get("devices", {}).values()
                for device in runtime
                if device.get("isAvailable") and "iPhone" in device.get("name", "")
            ]
        except (TypeError, ValueError) as error:
            return {"spec": spec, "available": False, "detail": str(error)}
        matches = (
            sorted(devices, key=lambda item: item.get("state") != "Booted")
            if value == "auto"
            else [item for item in devices if value in (item.get("udid"), item.get("name"))]
        )
        if not matches:
            return {"spec": spec, "available": False, "detail": "simulator not found"}
        selected = matches[0]
        return {
            "spec": spec,
            "available": True,
            "allocation": selected.get("udid", ""),
            "name": selected.get("name", ""),
            "state": selected.get("state", ""),
        }
    if kind == "android":
        ok, output = probe(["adb", "devices"])
        if not ok:
            return {"spec": spec, "available": False, "detail": output}
        devices = [
            row.split()[0]
            for row in output.splitlines()[1:]
            if len(row.split()) >= 2 and row.split()[1] == "device"
        ]
        matches = devices if value == "auto" else [device for device in devices if device == value]
        return {
            "spec": spec,
            "available": bool(matches),
            "allocation": matches[0] if matches else "",
            "detail": "" if matches else "no authorized Android device",
        }
    if kind == "ios-device":
        if platform.system() != "Darwin" or not shutil.which("xcrun"):
            return {"spec": spec, "available": False, "detail": "devicectl requires macOS"}
        ok, output = probe(["xcrun", "devicectl", "list", "devices"], timeout=20)
        if not ok:
            return {"spec": spec, "available": False, "detail": output}
        devices = []
        for row in output.splitlines()[2:]:
            columns = [column.strip() for column in row.split("  ") if column.strip()]
            if len(columns) >= 4 and columns[3].startswith("available"):
                devices.append({"name": columns[0], "identifier": columns[2]})
        matches = (
            devices
            if value == "auto"
            else [item for item in devices if value in (item["name"], item["identifier"])]
        )
        return {
            "spec": spec,
            "available": bool(matches),
            "allocation": matches[0]["identifier"] if matches else "",
            "name": matches[0]["name"] if matches else "",
            "detail": "" if matches else "no paired available iOS device",
        }
    return {"spec": spec, "available": False, "detail": "unknown health kind"}


def health_report(specs):
    return [check_health(spec) for spec in specs]


def state_root():
    configured = os.environ.get("IRIS_NATIVE_LAB_STATE_DIR") or os.environ.get(
        "NVPN_NATIVE_LAB_STATE_DIR"
    )
    return Path(configured) if configured else Path(tempfile.gettempdir()) / "iris-native-lab"


def lock_path(resource):
    digest = hashlib.sha256(resource.encode()).hexdigest()[:12]
    readable = "".join(char if char.isalnum() else "-" for char in resource)[:40]
    return state_root() / "locks" / f"{readable}-{digest}"


def alive(pid):
    try:
        os.kill(int(pid), 0)
        return int(pid) > 0
    except (OSError, TypeError, ValueError):
        return False


def acquire(resource, stale_after):
    path = lock_path(resource)
    path.parent.mkdir(parents=True, exist_ok=True)
    owner_path = path / "owner.json"
    for _ in range(2):
        try:
            path.mkdir()
            owner_path.write_text(
                json.dumps(
                    {
                        "resource": resource,
                        "pid": os.getpid(),
                        "host": socket.gethostname(),
                        "started_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
                    },
                    sort_keys=True,
                ),
                encoding="utf-8",
            )
            return path, None
        except FileExistsError:
            try:
                owner = json.loads(owner_path.read_text(encoding="utf-8"))
            except (OSError, ValueError):
                owner = {}
            age = time.time() - path.stat().st_mtime
            local_dead = owner.get("host") == socket.gethostname() and not alive(owner.get("pid"))
            if age >= stale_after or local_dead:
                shutil.rmtree(path, ignore_errors=True)
                continue
            return None, owner
    return None, {"resource": resource, "detail": "could not acquire resource"}


def health_resource(check):
    kind = str(check.get("spec", "")).partition(":")[0]
    allocation = str(check.get("allocation", ""))
    if not check.get("available") or kind not in RESERVED_KINDS or not allocation:
        return None
    if kind == "local":
        return f"host:local:{socket.gethostname()}"
    if kind == "ssh":
        return f"host:ssh:{allocation}"
    return f"{kind}:{allocation}"


def child_environment(checks, mappings):
    environment = os.environ.copy()
    for mapping in mappings:
        kind, separator, variable = mapping.partition("=")
        if not separator or not kind or not variable.isidentifier():
            raise ValueError(f"invalid allocation mapping: {mapping}")
        matches = [
            str(check.get("allocation"))
            for check in checks
            if str(check.get("spec", "")).partition(":")[0] == kind
            and check.get("available")
            and check.get("allocation")
        ]
        if len(matches) != 1:
            raise ValueError(f"allocation mapping {mapping} requires one available {kind}")
        environment[variable] = matches[0]
    return environment


def write_report(report, destination=None):
    rendered = json.dumps(report, indent=2, sort_keys=True) + "\n"
    if destination:
        path = Path(destination)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(rendered, encoding="utf-8")
    print(rendered, end="")


def run_managed(args):
    started = time.monotonic()
    primary, owner = acquire(args.resource, args.stale_after)
    if primary is None:
        write_report(
            {
                "status": "infrastructure_unavailable",
                "category": "resource_busy",
                "resource": args.resource,
                "owner": owner or {},
                "exit_code": INFRASTRUCTURE_UNAVAILABLE,
            },
            args.result,
        )
        return INFRASTRUCTURE_UNAVAILABLE
    locks = [primary]
    try:
        checks = health_report(args.health)
        if any(not check["available"] for check in checks):
            write_report(
                {
                    "status": "infrastructure_unavailable",
                    "category": "preflight",
                    "resource": args.resource,
                    "health": checks,
                    "exit_code": INFRASTRUCTURE_UNAVAILABLE,
                },
                args.result,
            )
            return INFRASTRUCTURE_UNAVAILABLE
        resources = sorted({item for check in checks if (item := health_resource(check))})
        for resource in resources:
            lock, owner = acquire(resource, args.stale_after)
            if lock is None:
                write_report(
                    {
                        "status": "infrastructure_unavailable",
                        "category": "resource_busy",
                        "resource": args.resource,
                        "busy_resource": resource,
                        "owner": owner or {},
                        "exit_code": INFRASTRUCTURE_UNAVAILABLE,
                    },
                    args.result,
                )
                return INFRASTRUCTURE_UNAVAILABLE
            locks.append(lock)
        try:
            environment = child_environment(checks, args.allocation_env)
        except ValueError as error:
            write_report(
                {
                    "status": "product_failure",
                    "category": "configuration",
                    "detail": str(error),
                    "exit_code": 2,
                },
                args.result,
            )
            return 2
        try:
            completed = subprocess.run(
                args.command[1:],
                check=False,
                timeout=args.timeout or None,
                env=environment,
            )
            code = completed.returncode
            status = "passed" if code == 0 else "infrastructure_unavailable" if code == 75 else "product_failure"
            category = "test_environment" if code == 75 else "verification"
        except subprocess.TimeoutExpired:
            code, status, category = 124, "product_failure", "verification_timeout"
        write_report(
            {
                "status": status,
                "category": category,
                "resource": args.resource,
                "reserved_resources": resources,
                "duration_seconds": round(time.monotonic() - started, 3),
                "exit_code": code,
            },
            args.result,
        )
        return code if code >= 0 else 1
    finally:
        for lock in reversed(locks):
            shutil.rmtree(lock, ignore_errors=True)


def parser():
    root = argparse.ArgumentParser(description=__doc__)
    commands = root.add_subparsers(dest="subcommand", required=True)
    health = commands.add_parser("health")
    health.add_argument("--health", action="append", required=True)
    health.add_argument("--result")
    run = commands.add_parser("run")
    run.add_argument("--resource", required=True)
    run.add_argument("--health", action="append", default=[])
    run.add_argument("--allocation-env", action="append", default=[])
    run.add_argument("--result")
    run.add_argument("--timeout", type=int, default=0)
    run.add_argument("--stale-after", type=int, default=21600)
    run.add_argument("command", nargs=argparse.REMAINDER)
    return root


def main():
    args = parser().parse_args()
    if args.subcommand == "health":
        checks = health_report(args.health)
        available = all(check["available"] for check in checks)
        write_report(
            {
                "status": "available" if available else "infrastructure_unavailable",
                "category": "preflight",
                "health": checks,
                "exit_code": 0 if available else INFRASTRUCTURE_UNAVAILABLE,
            },
            args.result,
        )
        return 0 if available else INFRASTRUCTURE_UNAVAILABLE
    if not args.command or args.command[0] != "--" or len(args.command) == 1:
        print("run requires -- <command>", file=sys.stderr)
        return 2
    return run_managed(args)


if __name__ == "__main__":
    raise SystemExit(main())
