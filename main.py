# -*- coding: utf-8 -*-
"""双击/直接运行入口：python main.py [子命令]

不带参数时启动图形界面；带参数时等价于 python -m opo <参数>。
"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from opo.cli import main

if __name__ == "__main__":
    argv = sys.argv[1:]
    if not argv:
        argv = ["gui"]
    sys.exit(main(argv))
