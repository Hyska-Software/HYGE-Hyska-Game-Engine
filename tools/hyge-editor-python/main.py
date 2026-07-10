"""Executable wrapper for ``python main.py`` and pyside6-deploy."""

from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).parent / "src"))

from hyge_editor.main import main

raise SystemExit(main())
