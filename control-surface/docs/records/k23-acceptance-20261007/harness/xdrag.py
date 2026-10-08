#!/usr/bin/python3
"""Drag with the left button on the PRIVATE display: x1 y1 x2 y2."""
import importlib.util, os, sys
from pathlib import Path
FILES = Path("/home/blin/.copilot/session-state/3eb20fbc-66e2-4394-8296-a849fb978fed/files")
spec = importlib.util.spec_from_file_location("qualified_gui", FILES / "production-gui/run_gui.py")
helper = importlib.util.module_from_spec(spec); spec.loader.exec_module(helper)
x1, y1, x2, y2 = map(float, sys.argv[1:5])
helper.XInput(Path(os.environ["K23_LEASE_ROOT"])).drag((x1, y1), (x2, y2))
print("dragged")
