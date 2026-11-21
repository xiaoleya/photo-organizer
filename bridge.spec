# -*- mode: python ; coding: utf-8 -*-
"""把 Python 引擎打包成独立后端 exe（无控制台）。"""
import os
from PyInstaller.utils.hooks import collect_submodules

hiddenimports = (collect_submodules("PIL") + [
    "PIL._tkinter_finder", "sqlite3", "multiprocessing",
    "email", "http.server", "urllib.parse",
])

a = Analysis(
    ["bridge_launcher.py"],
    pathex=[os.path.abspath(".")],
    binaries=[],
    datas=[],
    hiddenimports=hiddenimports,
    excludes=["tkinter", "numpy.testing", "matplotlib", "scipy", "pandas"],
    noarchive=False,
)
pyz = PYZ(a.pure, a.zipped_data)
exe = EXE(
    pyz, a.scripts, a.binaries, a.zipfiles, a.datas, [],
    name="photo-backend",
    console=False,
    debug=False,
    strip=False,
    upx=True,
    icon="app.ico" if os.path.exists("app.ico") else None,
)
