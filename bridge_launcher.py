# -*- coding: utf-8 -*-
"""后端桥接 exe 入口：启动本地 HTTP 服务供 Tauri 前端调用。"""
import os
import sys

if getattr(sys, "frozen", False):
    base = os.path.dirname(sys.executable)
    if base not in sys.path:
        sys.path.insert(0, base)

from opo.bridge import main

if __name__ == "__main__":
    main()
