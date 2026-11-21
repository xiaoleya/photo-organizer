# -*- coding: utf-8 -*-
"""图形界面入口。

两个关键点（之前两个 bug 都出在这里）：

1) DPI 感知必须在 import tkinter 之前声明，否则窗口被系统当位图拉伸，字发虚。
2) 声明 DPI 感知之后，Tk 的 geometry() / winfo_* 全部以**物理像素**为单位。
   在 200% 缩放下写 geometry("1440x920") 实际只有 720x460 逻辑尺寸，
   界面会被挤成一半、文字互相重叠。所以窗口尺寸必须按缩放比换算。
"""
from __future__ import annotations

import sys


def _enable_dpi_awareness():
    """在 import tkinter 之前调用，声明 PER_MONITOR_AWARE_V2。"""
    if sys.platform != "win32":
        return
    try:
        import ctypes
        from ctypes import wintypes

        user32 = ctypes.windll.user32
        # 必须显式声明 argtypes，否则 64 位下参数被截断、调用静默失败
        user32.SetProcessDpiAwarenessContext.argtypes = [wintypes.HANDLE]
        user32.SetProcessDpiAwarenessContext.restype = wintypes.BOOL
        if user32.SetProcessDpiAwarenessContext(wintypes.HANDLE(-4)):
            return
        try:
            if ctypes.windll.shcore.SetProcessDpiAwareness(2) == 0:
                return
        except Exception:
            pass
        user32.SetProcessDPIAware()
    except Exception:
        pass


_enable_dpi_awareness()

import tkinter as tk

from . import design
from .library import Library


def ui_scale(root):
    """当前窗口的缩放比（96dpi = 1.0）。"""
    try:
        import ctypes
        dpi = ctypes.windll.user32.GetDpiForWindow(root.winfo_id())
    except Exception:
        dpi = 96
    if not dpi or dpi < 72:
        dpi = 96
    return dpi / 96.0


def main():
    from .shell import App

    root = tk.Tk()
    root.title("照片整理")

    # 先用默认尺寸映射出窗口，拿到真实 DPI，再按逻辑尺寸重设
    root.geometry("+80+40")
    root.update_idletasks()

    s = ui_scale(root)
    # 设计稿的逻辑尺寸（MangoDisk 是 1280x800 基准）
    want_w, want_h = 1360, 860
    sw, sh = root.winfo_screenwidth(), root.winfo_screenheight()
    phys_w = min(int(want_w * s), sw - 60)
    phys_h = min(int(want_h * s), sh - 100)
    root.geometry("%dx%d+%d+%d" % (phys_w, phys_h, 60, 40))
    root.minsize(int(1120 * s), int(740 * s))

    try:
        root.tk.call("tk", "scaling", 96 * s / 72.0)
    except Exception:
        pass
    root.option_add("*tearOff", 0)
    root.configure(bg=design.surface(design.DARK)["bg_base"])

    lib = Library()
    app = App(root, library=lib)
    app.ui_scale = s
    root.protocol("WM_DELETE_WINDOW", app.on_close)
    root.mainloop()
    return 0


if __name__ == "__main__":
    sys.exit(main())
