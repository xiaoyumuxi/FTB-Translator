from __future__ import annotations

import os
from pathlib import Path


APP_DIR_NAME = "FTB-Translater"


def user_config_dir() -> Path:
    override = os.getenv("FTB_TRANSLATER_CONFIG_DIR")
    if override:
        return Path(override).expanduser()

    if os.name == "nt":
        base = os.getenv("LOCALAPPDATA") or os.getenv("APPDATA")
        if base:
            return Path(base) / APP_DIR_NAME

    xdg_config_home = os.getenv("XDG_CONFIG_HOME")
    if xdg_config_home:
        return Path(xdg_config_home) / "ftb-translater"

    return Path.home() / ".config" / "ftb-translater"
