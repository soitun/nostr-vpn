#!/usr/bin/env python3

import hashlib
import json
import os
import platform
import socket
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


LAB = Path(__file__).resolve().with_name("native-lab.py")


class NativeLabTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.state_dir = str(Path(self.temp.name) / "state")

    def run_lab(self, *args: str) -> subprocess.CompletedProcess:
        environment = os.environ.copy()
        environment["NVPN_NATIVE_LAB_STATE_DIR"] = self.state_dir
        return subprocess.run(
            [sys.executable, str(LAB), *args],
            check=False,
            capture_output=True,
            text=True,
            env=environment,
        )

    def test_health_distinguishes_available_and_missing_commands(self) -> None:
        available = self.run_lab("health", "--health", f"command:{Path(sys.executable).name}")
        self.assertEqual(available.returncode, 0, available.stderr)
        self.assertEqual(json.loads(available.stdout)["status"], "available")
        missing = self.run_lab("health", "--health", "command:not-a-real-native-lab-tool")
        self.assertEqual(missing.returncode, 75)

    def test_run_classifies_product_and_infrastructure_failures(self) -> None:
        product = self.run_lab(
            "run", "--resource", "matrix", "--", sys.executable, "-c", "raise SystemExit(7)"
        )
        self.assertEqual(product.returncode, 7)
        self.assertEqual(json.loads(product.stdout)["status"], "product_failure")
        infrastructure = self.run_lab(
            "run", "--resource", "matrix", "--", sys.executable, "-c", "raise SystemExit(75)"
        )
        self.assertEqual(infrastructure.returncode, 75)
        self.assertEqual(json.loads(infrastructure.stdout)["status"], "infrastructure_unavailable")

    def test_run_exports_the_selected_allocation(self) -> None:
        variable = "NVPN_NATIVE_LAB_TEST_DEVICE"
        previous = os.environ.get(variable)
        os.environ[variable] = "known-device"
        self.addCleanup(
            lambda: os.environ.pop(variable, None)
            if previous is None
            else os.environ.__setitem__(variable, previous)
        )
        completed = self.run_lab(
            "run",
            "--resource",
            "matrix",
            "--health",
            f"env:{variable}",
            "--allocation-env",
            "env=ALLOCATED_DEVICE",
            "--",
            sys.executable,
            "-c",
            "import os; raise SystemExit(0 if os.environ.get('ALLOCATED_DEVICE') == 'known-device' else 9)",
        )
        self.assertEqual(completed.returncode, 0, completed.stderr)

    def test_run_rejects_a_busy_named_host(self) -> None:
        system = platform.system().lower()
        label = {"darwin": "macos", "windows": "windows", "linux": "linux"}[system]
        resource = f"host:local:{socket.gethostname()}"
        digest = hashlib.sha256(resource.encode()).hexdigest()[:12]
        readable = "".join(char if char.isalnum() else "-" for char in resource)[:40]
        lock = Path(self.state_dir) / "locks" / f"{readable}-{digest}"
        lock.mkdir(parents=True)
        (lock / "owner.json").write_text(
            json.dumps({"resource": resource, "pid": os.getpid(), "host": socket.gethostname()}),
            encoding="utf-8",
        )
        completed = self.run_lab(
            "run",
            "--resource",
            "other-matrix",
            "--health",
            f"local:{label}",
            "--",
            sys.executable,
            "-c",
            "raise SystemExit(0)",
        )
        report = json.loads(completed.stdout)
        self.assertEqual(completed.returncode, 75)
        self.assertEqual(report["busy_resource"], resource)


if __name__ == "__main__":
    unittest.main()
