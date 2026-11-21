# -*- coding: utf-8 -*-
import os, sys
sys.path.insert(0, r"E:\agent xm\.deeepseek\old-photo\old-photo-organizer")
os.environ["OPO_HOME"] = r"E:\agent xm\.deeepseek\old-photo\old-photo-organizer\test_home"
from opo.imaging import open_image, quality_metrics
from opo import restore

im = open_image(r"E:\agent xm\.deeepseek\old-photo\old-photo-organizer\samples\1985-06-12 春节全家福.jpg")
def show(tag, x):
    m = quality_metrics(x)
    print("%-18s size=%-12s contrast=%.4f sat=%.4f sepia=%.3f noise=%.2f blur=%.1f sharp=%.1f mean=%.3f" %
          (tag, "%dx%d"%x.size, m["contrast"], m["saturation"], m["sepia"], m["noise"], m["blur"], m["sharpness"], m["brightness"]))
    return m

o = im
show("0 原图", o)
steps = [
    ("crop_border", lambda x: restore.crop_border(x)),
    ("deskew", lambda x: restore.deskew(x)),
    ("denoise_adaptive", lambda x: restore.denoise_adaptive(x, 4.66)),
    ("despeckle(26)", lambda x: restore.despeckle(x, threshold=26.0)),
    ("neutral_cast", lambda x: restore.neutral_cast(x)),
    ("deyellow", lambda x: restore.deyellow(x, 1.0)),
    ("auto_levels", lambda x: restore.auto_levels(x, clip=0.35)),
    ("clahe(2.2)", lambda x: restore.clahe(x, clip_limit=2.2)),
    ("saturate_by_fade", lambda x: restore.saturate_by_fade(x, 0.20)),
    ("smart_sharpen", lambda x: restore.smart_sharpen(x, 568.3)),
]
for name, fn in steps:
    o = fn(o)
    show(name, o)