# -*- coding: utf-8 -*-
import os, sys, io
for _n in ("stdout","stderr"):
    _s=getattr(sys,_n,None)
    if _s is not None and (getattr(_s,"encoding","") or "").lower() not in ("utf-8","utf8"):
        try: setattr(sys,_n,io.TextIOWrapper(_s.buffer,encoding="utf-8",errors="replace",line_buffering=True))
        except Exception: pass
sys.path.insert(0, r"E:\agent xm\old-photo-organizer")
os.environ["OPO_HOME"] = r"E:\agent xm\old-photo-organizer\demo_home"
import tkinter as tk
import opo.gui as g

root = tk.Tk()
root.withdraw()
app = g.App(root)
print("App created ok")
# 逐个切换视图，确认全部能构建
errs = []
for label in getattr(g, "NAV_ITEMS", []):
    try:
        key = label[0] if isinstance(label, (tuple, list)) else label
        app.show_view(key) if hasattr(app, "show_view") else None
        root.update()
        print("  view ok:", key)
    except Exception as e:
        errs.append((label, repr(e)[:160]))
        print("  view FAIL:", label, repr(e)[:160])
print("errors:", errs if errs else "none")
root.update()
root.destroy()
print("DONE")
