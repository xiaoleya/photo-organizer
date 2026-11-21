# -*- coding: utf-8 -*-
import os, sys, shutil
sys.path.insert(0, r"E:\agent xm\.deeepseek\old-photo\old-photo-organizer")
os.environ["OPO_HOME"] = r"E:\agent xm\.deeepseek\old-photo\old-photo-organizer\demo_home"
shutil.rmtree(os.environ["OPO_HOME"], ignore_errors=True)
from opo.library import Library
from opo import config, pipeline, report, organizer
from opo.imaging import open_image, quality_metrics
from opo import restore

lib = Library(); cfg = config.load_settings()
prog = pipeline.Progress()
r = pipeline.run_full(lib, cfg, roots=[r"E:\agent xm\.deeepseek\old-photo\old-photo-organizer\samples"], prog=prog)
print("scan:", r.get("scan"), "analyze:", r.get("analyze"))
print("dup:", r.get("dup"))
out = r"E:\agent xm\.deeepseek\old-photo\old-photo-organizer\demo_out"
shutil.rmtree(out, ignore_errors=True); os.makedirs(out, exist_ok=True)
# 修复最有年代感的 3 张
rows = lib.q("SELECT * FROM photos WHERE analyzed_at IS NOT NULL ORDER BY age_score DESC LIMIT 3")
for i, row in enumerate(rows):
    im = open_image(row["path"])
    m = quality_metrics(im)
    fixed = restore.auto_enhance(im, m, restore.PRESETS["标准修复"])
    name = "修复%02d_原图.jpg" % (i+1)
    im.save(os.path.join(out, name), "JPEG", quality=92)
    fixed.save(os.path.join(out, "修复%02d_增强.jpg" % (i+1)), "JPEG", quality=92)
    print("修复:", os.path.basename(row["path"]), "age=%.1f" % row["age_score"])
report.export_html(lib, os.path.join(out, "report.html"), title="老照片整理报告")
print("报告:", os.path.join(out, "report.html"))
print("库统计:", lib.stats()["total"], "张,", "标签", lib.stats()["tags"], "个")