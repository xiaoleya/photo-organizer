# -*- coding: utf-8 -*-
import os
import sys
import io

for _n in ("stdout", "stderr"):
    _s = getattr(sys, _n, None)
    if _s is not None and (getattr(_s, "encoding", "") or "").lower() not in ("utf-8", "utf8"):
        try:
            setattr(sys, _n, io.TextIOWrapper(_s.buffer, encoding="utf-8",
                                              errors="replace", line_buffering=True))
        except Exception:
            pass

sys.path.insert(0, r"E:\agent xm\.deeepseek\old-photo\old-photo-organizer")
os.environ["OPO_HOME"] = r"E:\agent xm\.deeepseek\old-photo\old-photo-organizer\test_home"
from opo.imaging import open_image, quality_metrics
from opo import restore

S = r"E:\agent xm\.deeepseek\old-photo\old-photo-organizer\samples"
cases = ["1985-06-12 春节全家福.jpg", "模糊的老照片 1978.jpg", "扫描件_2008-07-20_证件照.jpg",
         "微信图片_20200101.jpg", "1992-10-01 旅行.jpg"]
print("%-30s %-6s %-8s %-7s %-7s %-6s %-7s" % ("文件","阶段","contrast","sat","sepia","noise","blur"))
for c in cases:
    im = open_image(os.path.join(S, c))
    if im is None: continue
    m = quality_metrics(im)
    out = restore.auto_enhance(im, m, restore.PRESETS["标准修复"])
    o = quality_metrics(out)
    print("%-30s %-6s %8.3f %7.3f %7.3f %6.2f %7.1f" % (c[:28], "原", m["contrast"], m["saturation"], m["sepia"], m["noise"], m["blur"]))
    print("%-30s %-6s %8.3f %7.3f %7.3f %6.2f %7.1f" % ("", "修", o["contrast"], o["saturation"], o["sepia"], o["noise"], o["blur"]))
    verdict = []
    if o["contrast"] > m["contrast"]*1.10: verdict.append("对比+")
    if o["sepia"] < m["sepia"]*0.85: verdict.append("去黄+")
    if o["noise"] <= max(m["noise"]*1.25, m["noise"]+2.0): verdict.append("噪点-Ok")
    else: verdict.append("噪点↑%.1f" % (o["noise"]-m["noise"]))
    if o["blur"] >= m["blur"]*0.92: verdict.append("细节保留")
    else: verdict.append("细节↓")
    print("%-30s -> %s\n" % ("", " ".join(verdict)))