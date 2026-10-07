#!/usr/bin/python3
# Outer launch for lease k23-keypad-sim-20261007: the recipe's exact outer code.
# argv[1]: NEW run suffix; argv[2]: "on" (acceptance) or "off" (default-off negotiation).
import importlib.util
import os
from pathlib import Path
import subprocess
import sys

SUFFIX = sys.argv[1]
ENABLED = {"on": "true", "off": "false"}[sys.argv[2]]
SOURCE = Path("/home/blin/Documents/source/kdenlive/.automation-curves")
BUILD = SOURCE / "build-private-20261004"
PREFIX = Path("/home/blin/Documents/source/kdenlive/.mlt-automation-runtime-audio-phase-20261005")
FILES = Path("/home/blin/.copilot/session-state/3eb20fbc-66e2-4394-8296-a849fb978fed/files")
RUN = Path(f"/home/blin/.copilot/session-state/9147f090-d696-4b70-af0f-aa5a935f364f/files/k23-real-daemon-20261007-{SUFFIX}")
OWNER_DRIVER = RUN.parent / "k23_real_daemon_driver.py"
assert OWNER_DRIVER.is_file()
RUN.mkdir(mode=0o700)  # intentionally fail if it already exists
os.environ["KDENLIVE_WORKFLOW_BUILD"] = str(BUILD)
os.environ["KDENLIVE_WORKFLOW_PREFIX"] = str(PREFIX)
os.environ["KDENLIVE_WORKFLOW_CONTRACT"] = "248f8738c3fc597ad86256d7d88967ec8acb146c8ee89a34556ba02ec8f6c255"
spec = importlib.util.spec_from_file_location("qualified_gui", FILES / "production-gui/run_gui.py")
helper = importlib.util.module_from_spec(spec)
spec.loader.exec_module(helper)
env = helper.private_environment(RUN)  # fresh allowlist, not os.environ.copy()
env["K23_LEASE_ROOT"] = str(RUN)
env["K23_DRIVER_MODE"] = "on" if ENABLED == "true" else "off"
(RUN / "config/kdenliverc").write_text(
    "[version]\nfixture=k23-real-daemon-20261007\n"
    f"[misc]\nenableControlSurfaceInterface={ENABLED}\n"
)
subprocess.run([
    "/usr/bin/xvfb-run", "-a", "-s", "-screen 0 1440x1000x24 -nolisten tcp",
    "/usr/bin/dbus-run-session", "--",
    "/usr/bin/python3", "-B", str(OWNER_DRIVER), str(RUN)
], env=env, cwd=RUN / "work", check=True)
