# -*- mode: python ; coding: utf-8 -*-
"""方案A：目录模式后端。启动 ~1-2 秒，无需每次解压。
发布时是一个 backend/ 文件夹，主程序从里面启动 python 运行时。"""
import os
from PyInstaller.utils.hooks import collect_submodules

hiddenimports = (collect_submodules("PIL") + [
    "PIL._tkinter_finder", "sqlite3", "multiprocessing",
    "email", "http.server", "urllib.parse",
])

a = Analysis(
    ["bridge_launcher.py"],
    pathex=[os.path.abspath(".")],
    binaries=[], datas=[],
    hiddenimports=hiddenimports,
    excludes=["tkinter", "numpy.testing", "matplotlib", "scipy", "pandas"],
    noarchive=False,
)
pyz = PYZ(a.pure, a.zipped_data)
exe = EXE(
    pyz, a.scripts, [],
    exclude_binaries=True,
    name="photo-backend",
    console=False,
    debug=False,
    strip=False,
    upx=False,
    icon="app.ico" if os.path.exists("app.ico") else None,
)
coll = COLLECT(
    exe, a.binaries, a.zipfiles, a.datas,
    strip=False, upx=False, upx_exclude=[],
    name="backend",
)
