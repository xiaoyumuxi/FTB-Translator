from __future__ import annotations

import os
import sys
from datetime import datetime
from pathlib import Path
import unittest


def main() -> int:
    command_name = os.path.basename(sys.argv[0]).lower()
    default_group = "e2e" if "e2e" in command_name or "full" in command_name else "unit"
    group = sys.argv[1] if len(sys.argv) > 1 else default_group
    if group in {"unit", "normal"}:
        suite = unittest.defaultTestLoader.discover("tests", pattern="test_*.py")
    elif group in {"e2e", "full"}:
        os.environ.setdefault("FTB_TRANSLATER_LIVE_TEST", "1")
        os.environ.setdefault("FTB_TRANSLATER_LIVE_DEEPSEEK", "1")
        output_dir = Path(".ftb-translater") / "e2e-runs" / datetime.now().strftime("%Y%m%d-%H%M%S")
        os.environ.setdefault("FTB_TRANSLATER_LIVE_OUTPUT_DIR", str(output_dir))
        print(f"Full e2e output dir: {Path(os.environ['FTB_TRANSLATER_LIVE_OUTPUT_DIR']).resolve()}")
        suite = unittest.TestSuite(
            [
                unittest.defaultTestLoader.loadTestsFromName("tests.test_live_curseforge"),
                unittest.defaultTestLoader.loadTestsFromName("tests.test_live_deepseek_e2e"),
            ]
        )
    else:
        print("Usage: python -m tests.run_groups [unit|e2e]", file=sys.stderr)
        return 2

    result = unittest.TextTestRunner(verbosity=2).run(suite)
    return 0 if result.wasSuccessful() else 1


if __name__ == "__main__":
    raise SystemExit(main())
