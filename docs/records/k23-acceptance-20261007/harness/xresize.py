#!/usr/bin/python3
"""Resize the leased editor's main window on the PRIVATE display (no WM there)."""
import importlib.util, os, sys
from pathlib import Path
FILES = Path("/home/blin/.copilot/session-state/3eb20fbc-66e2-4394-8296-a849fb978fed/files")
spec = importlib.util.spec_from_file_location("qualified_gui", FILES / "production-gui/run_gui.py")
helper = importlib.util.module_from_spec(spec); spec.loader.exec_module(helper)
run = Path(os.environ["K23_LEASE_ROOT"])
helper.XInput(run).resize_toplevels(int(sys.argv[1]), int(sys.argv[2]))
print("resized")
