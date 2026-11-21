# -*- coding: utf-8 -*-
import os, sys, io, subprocess
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
root.geometry("1400x900")
root.update()
root.after(1200, lambda: None)
root.update()
out = r"E:\agent xm\old-photo-organizer\demo_out\gui_shot.ps1"
# 用 PowerShell 截屏
ps = '''
Add-Type -AssemblyName System.Windows.Forms,System.Drawing
$b = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
$bmp = New-Object System.Drawing.Bitmap $b.Width, $b.Height
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($b.Location, [System.Drawing.Point]::Empty, $b.Size)
$bmp.Save("%s", [System.Drawing.Imaging.ImageFormat]::Png)
''' % r"E:\agent xm\old-photo-organizer\demo_out\gui_shot.png"
open(out, "w", encoding="utf-8").write(ps)
for i in range(30):
    root.update()
    import time; time.sleep(0.05)
subprocess.run(["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", out], capture_output=True)
print("shot saved")
root.destroy()
