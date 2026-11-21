# -*- coding: utf-8 -*-
import os, sys
sys.path.insert(0, r"E:\agent xm\.deeepseek\old-photo\old-photo-organizer")
os.environ["OPO_HOME"] = r"E:\agent xm\.deeepseek\old-photo\old-photo-organizer\test_home"
import numpy as np
from PIL import Image, ImageDraw
from opo import restore
from opo.imaging import save_thumb

# 造一张干净图 + 画 10 条划痕（4 白 3 黑 3 灰），检查修补率
base = Image.new("RGB", (600, 400), (140, 150, 160))
d = ImageDraw.Draw(base)
d.ellipse([200, 120, 400, 320], fill=(210, 180, 150))
d.rectangle([60, 300, 540, 360], fill=(70, 80, 90))
clean = base.copy()
d2 = ImageDraw.Draw(base)
import random
random.seed(3)
lines = []
for i in range(10):
    x = 50 + i * 50
    y0 = random.randint(10, 80); y1 = y0 + random.randint(150, 300)
    c = 250 if i % 3 == 0 else (20 if i % 3 == 1 else 120)
    d2.line([(x, y0), (x + random.randint(-2, 2), y1)], fill=(c, c, c), width=1)
    lines.append((x, y0, y1))
out = restore.despeckle(base, threshold=22.0, dilate=1)
a = np.asarray(base, np.float32); b = np.asarray(out, np.float32); c0 = np.asarray(clean, np.float32)
# 划痕区域：沿线 ±2 像素
def line_err(img):
    tot = 0.0; n = 0
    for x, y0, y1 in lines:
        seg = img[max(0,y0):y1, max(0,x-2):x+3].mean(axis=(1,2))
        tot += np.abs(seg - 140).mean(); n += 1
    return tot / max(1, n)
print("划痕区域与原底的偏差: 处理前=%.1f  处理后=%.1f  理想(干净图)=%.1f" %
      (line_err(a), line_err(b), line_err(c0)))
# 干净区域是否被破坏
mask = np.ones(a.shape[:2], bool)
for x, y0, y1 in lines:
    mask[max(0,y0-4):y1+4, max(0,x-4):x+5] = False
print("非划痕区域相对原图的平均变化: %.3f" % np.abs(b-a)[mask].mean())
Image.fromarray(np.asarray(base)).save(r"E:\agent xm\.deeepseek\old-photo\old-photo-organizer\demo_out\划痕_处理前.png")
out.save(r"E:\agent xm\.deeepseek\old-photo\old-photo-organizer\demo_out\划痕_处理后.png")