# -*- coding: utf-8 -*-
import os, sys, io, shutil, time
for _n in ("stdout","stderr"):
    _s=getattr(sys,_n,None)
    if _s is not None and (getattr(_s,"encoding","") or "").lower() not in ("utf-8","utf8"):
        try: setattr(sys,_n,io.TextIOWrapper(_s.buffer,encoding="utf-8",errors="replace",line_buffering=True))
        except Exception: pass
sys.path.insert(0, r"E:\agent xm\.deeepseek\old-photo\old-photo-organizer")
HOME = r"E:\agent xm\.deeepseek\old-photo\old-photo-organizer\final_home"
shutil.rmtree(HOME, ignore_errors=True)
os.environ["OPO_HOME"] = HOME
from opo.library import Library
from opo import config, pipeline, report, organizer, restore
from opo.imaging import open_image, quality_metrics
from opo.cli import main as cli_main

print("=" * 60)
print(" 端到端验收：从零开始的完整用户流程")
print("=" * 60)
lib = Library(); cfg = config.load_settings()

# 1 全流程
t0 = time.time()
prog = pipeline.Progress()
r = pipeline.run_full(lib, cfg, roots=[r"E:\agent xm\.deeepseek\old-photo\old-photo-organizer\samples"], prog=prog)
print("\n[1] 全流程 %.2fs" % (time.time()-t0))
print("    扫描: %s" % r["scan"])
print("    分析: %s" % r["analyze"])
print("    去重: %s" % r["dup"])
st = lib.stats()
print("    统计: %d 张 / 标签 %d / 重复 %d" % (st["total"], st["tags"], st["dups"]))

# 2 归档
print("\n[2] 归档")
outdir = r"E:\agent xm\.deeepseek\old-photo\old-photo-organizer\final_out"
shutil.rmtree(outdir, ignore_errors=True)
plan = organizer.plan_organize(lib, scheme="year_month", template="{date}_{name}")
print("    计划 %d 条，示例: %s" % (len(plan), plan[0][1]))
res = organizer.execute_organize(lib, outdir, plan, mode="copy")
print("    执行: %d 成功 / %d 错误" % (res["done"], res["errors"]))
n = sum(len(f) for _,_,f in os.walk(outdir))
print("    落盘文件: %d" % n)

# 3 修复
print("\n[3] 批量修复（前 3 张）")
rows = lib.q("SELECT * FROM photos WHERE analyzed_at IS NOT NULL ORDER BY age_score DESC LIMIT 3")
for i, row in enumerate(rows):
    im = open_image(row["path"]); m = quality_metrics(im)
    fx = restore.auto_enhance(im, m, restore.PRESETS["标准修复"])
    q = quality_metrics(fx)
    print("    %-28s 对比 %.3f->%.3f 泛黄 %.3f->%.3f 噪点 %.2f->%.2f" %
          (os.path.basename(row["path"])[:26], m["contrast"], q["contrast"],
           m["sepia"], q["sepia"], m["noise"], q["noise"]))
    if i == 0:
        im.save(os.path.join(outdir, "_对比_修复前.jpg"), "JPEG", quality=92)
        fx.save(os.path.join(outdir, "_对比_修复后.jpg"), "JPEG", quality=92)

# 4 报告
print("\n[4] 报告")
hp = os.path.join(outdir, "report.html"); cp = os.path.join(outdir, "photos.csv")
report.export_html(lib, hp, title="老照片整理报告")
report.export_csv(lib, cp)
print("    HTML %d 字节 / CSV %d 字节" % (os.path.getsize(hp), os.path.getsize(cp)))

# 5 回滚
print("\n[5] 回滚")
rb = organizer.rollback(lib, res["batch"])
print("    恢复 %d / 错误 %d" % (rb["restored"], rb["errors"]))
print("    源文件仍完整: %s" % all(os.path.exists(x["path"]) for x in lib.all_photos()))
print("\n" + "=" * 60)
print(" 验收完成")
print("=" * 60)