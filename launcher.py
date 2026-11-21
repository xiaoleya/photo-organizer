# -*- coding: utf-8 -*-
"""打包入口：冻结后 sys._MEIPASS 才是资源根，需先把自身目录加入 sys.path。"""
import os
import sys

if getattr(sys, "frozen", False):
    base = os.path.dirname(sys.executable)
    if base not in sys.path:
        sys.path.insert(0, base)

from opo.gui import main

if __name__ == "__main__":
    sys.exit(main())
