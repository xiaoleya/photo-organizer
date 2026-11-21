# -*- coding: utf-8 -*-
import os, sys, io, subprocess, time
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
root.geometry("1400x880+0+0")
app = g.App(root)
root.update()
# 找到 restore 视图对象并选中一张照片
app.show_view("restore")
root.update()
rv = None
for attr in dir(app):
    o = getattr(app, attr, None)
    if type(o).__name__ == "RestoreView":
        rv = o; print("found RestoreView:", attr)
        break
print("RestoreView methods:", [m for m in dir(rv) if not m.startswith("_")][:40] if rv else "none")
if rv:
    for cand in ("set_photo","select_photo","load_photo","on_pick","show_photo"):
        if hasattr(rv, cand):
            row = app.lib.q1("SELECT * FROM photos WHERE analyzed_at IS NOT NULL ORDER BY age_score DESC LIMIT 1")
            try:
                getattr(rv, cand)(row["id"] if hasattr(rv, cand).__name__ else row)
                print("called", cand)
            except Exception as e:
                print("call", cand, "err", repr(e)[:120])
            break
for _ in range(80):
    root.update(); time.sleep(0.05)
ps = '''
Add-Type -AssemblyName System.Windows.Forms,System.Drawing
$b = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
$bmp = New-Object System.Drawing.Bitmap $b.Width, $b.Height
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($b.Location, [System.Drawing.Point]::Empty, $b.Size)
$bmp.Save("%s", [System.Drawing.Imaging.ImageFormat]::Png)
''' % r"E:\agent xm\old-photo-organizer\demo_out\gui_restore2.png"
p = r"E:\agent xm\old-photo-organizer\demo_out\shot3.ps1"
open(p,"w",encoding="utf-8").write(ps)
subprocess.run(["powershell","-NoProfile","-ExecutionPolicy","Bypass","-File",p], capture_output=True)
print("saved")
root.destroy()
