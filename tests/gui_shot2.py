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
app = g.App(root)
root.geometry("1400x880+0+0")
root.update()
app.show_view("restore")
for _ in range(40):
    root.update(); time.sleep(0.05)
# 若有"选中第一张"的入口就调用
for attr in ("select_first", "on_select_first"):
    if hasattr(app, attr):
        getattr(app, attr)()
        break
for _ in range(40):
    root.update(); time.sleep(0.05)
ps = '''
Add-Type -AssemblyName System.Windows.Forms,System.Drawing
$b = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
$bmp = New-Object System.Drawing.Bitmap $b.Width, $b.Height
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($b.Location, [System.Drawing.Point]::Empty, $b.Size)
$bmp.Save("%s", [System.Drawing.Imaging.ImageFormat]::Png)
''' % r"E:\agent xm\old-photo-organizer\demo_out\gui_restore.png"
p = r"E:\agent xm\old-photo-organizer\demo_out\shot2.ps1"
open(p,"w",encoding="utf-8").write(ps)
subprocess.run(["powershell","-NoProfile","-ExecutionPolicy","Bypass","-File",p], capture_output=True)
print("saved")
root.destroy()
