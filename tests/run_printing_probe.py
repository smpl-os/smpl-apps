#!/usr/bin/env python3
"""Exercise the production Printing page with synthetic queues on private Xvfb."""
from run_mouse_editor_probe import run_private_settings_probe


if __name__ == "__main__":
    run_private_settings_probe(
        ["printing::ui::native_tests::printing_native_queue_workflow"],
        name="printing",
    )
