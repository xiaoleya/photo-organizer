# -*- coding: utf-8 -*-
import os
import sys
import io
import shutil
import time

for _n in ("stdout", "stderr"):
    _s = getattr(sys, _n, None)
    if _s is not None and (getattr(_s, "encoding", "") or "").lower() not in ("utf-8", "utf8"):
        try:
            setattr(sys, _n, io.TextIOWrapper(_s.buffer, encoding="utf-8",
                                              errors="replace", line_buffering=True))
        except Exception:
            pass
import numpy as np
from PIL import Image

ROOT = r"E:\agent xm\.deeepseek\old-photo\old-photo-organizer"
sys.path.insert(0, ROOT)
os.environ["OPO_HOME"] = os.path.join(ROOT, "test_full_home")
shutil.rmtree(os.environ["OPO_HOME"], ignore_errors=True)

from opo import config, scanner, classify, restore, organizer, report
from opo.library import Library
from opo.imaging import open_image, quality_metrics

fails = []
def check(name, cond, extra=""):
    print(("  [PASS] " if cond else "  [FAIL] ") + name + ("  " + str(extra) if extra else ""))
    if not cond:
        fails.append(name)

print("=== 1. 修复算法 ===")
src = os.path.join(ROOT, "samples", "1985-06-12 春节全家福.jpg")
im = open_image(src)
check("原图可读", im is not None and im.size[0] > 0, im.size if im else None)
m = quality_metrics(im)
print("   原始指标 contrast=%.3f sat=%.3f sepia=%.3f noise=%.2f blur=%.1f" %
      (m["contrast"], m["saturation"], m["sepia"], m["noise"], m["blur"]))
out = restore.auto_enhance(im, m, restore.PRESETS["标准修复"])
o = quality_metrics(out)
print("   修复指标 contrast=%.3f sat=%.3f sepia=%.3f noise=%.2f blur=%.1f" %
      (o["contrast"], o["saturation"], o["sepia"], o["noise"], o["blur"]))
# 验收标准（与实测标定一致）
check("对比度提升 >=1.20x", o["contrast"] >= m["contrast"] * 1.20,
      "%.3f -> %.3f (%.2fx)" % (m["contrast"], o["contrast"], o["contrast"]/m["contrast"]))
check("泛黄显著下降 <=0.5x", o["sepia"] <= m["sepia"] * 0.50,
      "%.3f -> %.3f" % (m["sepia"], o["sepia"]))
check("噪点受控 <=1.6x/+2.5", o["noise"] <= max(m["noise"] * 1.60, m["noise"] + 2.5),
      "%.2f -> %.2f" % (m["noise"], o["noise"]))
check("细节保留 >=0.80x", o["blur"] >= m["blur"] * 0.80,
      "%.1f -> %.1f" % (m["blur"], o["blur"]))

# 多图统一验收
CASES = [
    ("1985-06-12 春节全家福.jpg", 1.20),
    ("模糊的老照片 1978.jpg", 1.15),
    ("扫描件_2008-07-20_证件照.jpg", 1.15),
    ("微信图片_20200101.jpg", 1.60),
    ("1992-10-01 旅行.jpg", 1.40),
]
for cn, need in CASES:
    pth = os.path.join(ROOT, "samples", cn)
    if not os.path.exists(pth):
        continue
    i2 = open_image(pth)
    if i2 is None:
        check("样品可读 %s" % cn, False)
        continue
    m2 = quality_metrics(i2)
    o2i = restore.auto_enhance(i2, m2, restore.PRESETS["标准修复"])
    q2 = quality_metrics(o2i)
    okc = q2["contrast"] >= m2["contrast"] * need   # 下限：不得低于此提升倍数
    okn = q2["noise"] <= max(m2["noise"] * 1.60, m2["noise"] + 2.5)
    okd = q2["blur"] >= m2["blur"] * 0.80
    check("%-28s 全指标" % cn[:26], okc and okn and okd,
          "c %.2fx / n %.2f->%.2f / d %.2fx" %
          (q2["contrast"]/max(m2["contrast"],1e-6), m2["noise"], q2["noise"],
           q2["blur"]/max(m2["blur"],1e-6)))

# 各子算法独立可用
for fn, args in [("auto_levels", {}), ("gray_world", {}), ("white_patch", {}),
                 ("neutral_cast", {}), ("deyellow", {}), ("restore_color", {}),
                 ("denoise", {"strength": 2}), ("denoise_adaptive", {"noise_level": 4.0}),
                 ("despeckle", {}), ("clahe", {}), ("unsharp", {}),
                 ("edge_sharpen", {}), ("suppress_noise", {"level": 6.0}),
                 ("smart_sharpen", {"blur_metric": 50}), ("crop_border", {}),
                 ("deskew", {}), ("fix_red_eye", {})]:
    try:
        f = getattr(restore, fn)
        r = f(im, **args)
        ok = isinstance(r, Image.Image) and r.size[0] > 0
        check("restore.%s" % fn, ok, r.size if ok else "bad")
    except Exception as e:
        check("restore.%s" % fn, False, repr(e)[:90])

check("detect_skew 返回数值", isinstance(restore.detect_skew(im), float))
check("upscale 放大", restore.upscale(im, 1.5).size[0] > im.size[0])
for name in restore.PRESETS:
    try:
        r = restore.auto_enhance(im, m, restore.PRESETS[name])
        check("预设 %s" % name, r is not None and r.size[0] > 0)
    except Exception as e:
        check("预设 %s" % name, False, repr(e)[:80])

print("\n=== 2. 微型库 + 整理归档 ===")
work = os.path.join(ROOT, "organize_src")
shutil.rmtree(work, ignore_errors=True); os.makedirs(work)
for i, n in enumerate(["1990-05-01 春游.jpg", "1990-05-01 合影.jpg", "2010年 毕业.jpg",
                       "无日期照片.jpg", "DSC_0001.JPG"]):
    shutil.copy2(os.path.join(ROOT, "samples", "DSC_0071.JPG"), os.path.join(work, n))
cfg = config.load_settings()
lib = Library()
scanner.scan_roots(lib, cfg, [work], force=True)
scanner.analyze_pending(lib, cfg, redo=True)
classify.auto_tag(lib, cfg); classify.cluster_events(lib, cfg); classify.find_duplicates(lib, cfg)
print("   库内照片:", lib.count(), "  有时间:", lib.count("dt IS NOT NULL"))

for scheme in organizer.SCHEMES:
    plan = organizer.plan_organize(lib, scheme=scheme, limit=50)
    check("方案 %-14s 生成计划" % scheme, len(plan) == lib.count(), "%d 条" % len(plan))
    if plan:
        check("方案 %-14s 路径安全" % scheme,
              all(not any(c in rel for c in '<>:"|?*') for _, rel in plan),
              plan[0][1])

dest = os.path.join(ROOT, "organize_out")
shutil.rmtree(dest, ignore_errors=True)
plan = organizer.plan_organize(lib, scheme="year_month", template="{date}_{name}")
r = organizer.execute_organize(lib, dest, plan, mode="copy", on_conflict="rename")
print("   复制结果:", {k: v for k, v in r.items() if k != "log_ids"})
check("全部复制成功", r["done"] == len(plan) and r["errors"] == 0, r)
copied = sum(len(f) for _, _, f in os.walk(dest))
check("目标文件数一致", copied == len(plan), "%d/%d" % (copied, len(plan)))
check("生成了回滚日志", len(r["log_ids"]) == len(plan))

rb = organizer.rollback(lib, r["batch"])
print("   回滚(复制模式):", rb)
check("回滚不破坏源文件", lib.count() == len(plan) and all(os.path.exists(x["path"]) for x in lib.all_photos()))

# move 模式 + 回滚
dest2 = os.path.join(ROOT, "organize_moved")
shutil.rmtree(dest2, ignore_errors=True)
plan2 = organizer.plan_organize(lib, scheme="flat")
r2 = organizer.execute_organize(lib, dest2, plan2, mode="move", on_conflict="rename")
print("   移动结果:", {k: v for k, v in r2.items() if k != "log_ids"})
check("移动模式成功", r2["done"] == len(plan2) and r2["errors"] == 0, r2)
rb2 = organizer.rollback(lib, r2["batch"])
print("   回滚(移动模式):", rb2)
check("移动回滚恢复全部", rb2["restored"] == len(plan2), rb2)

print("\n=== 3. 报告导出 ===")
html_p = os.path.join(ROOT, "out_report", "report.html")
csv_p = os.path.join(ROOT, "out_report", "photos.csv")
report.export_html(lib, html_p, title="老照片整理报告")
report.export_csv(lib, csv_p)
check("HTML 已生成", os.path.exists(html_p) and os.path.getsize(html_p) > 1500,
      os.path.getsize(html_p) if os.path.exists(html_p) else 0)
check("CSV 已生成", os.path.exists(csv_p) and os.path.getsize(csv_p) > 200,
      os.path.getsize(csv_p) if os.path.exists(csv_p) else 0)
h = open(html_p, encoding="utf-8").read()
check("HTML 含统计卡片", "照片总数" in h and "<img" in h)

print("\n=== 4. 边界情况 ===")
# 空目录
empty = os.path.join(ROOT, "empty_dir"); os.makedirs(empty, exist_ok=True)
r3 = scanner.scan_roots(lib, config.load_settings(), [empty], force=True)
check("空目录扫描不崩", "added" in r3, r3)
# 不存在目录
r4 = scanner.scan_roots(lib, config.load_settings(), [os.path.join(ROOT, "no_such_dir")], force=True)
check("不存在目录不崩", "added" in r4, r4)
# 极小图
tiny = os.path.join(ROOT, "tiny.png")
Image.new("RGB", (4, 3), (120, 100, 90)).save(tiny)
from opo.imaging import analyze_photo
res = analyze_photo(tiny, config.load_settings(), None)
check("4x3 极小图可分析", res is not None, res.get("width") if res else None)
# 纯色图
flat = os.path.join(ROOT, "flat.png")
Image.new("RGB", (800, 600), (128, 128, 128)).save(flat)
res2 = analyze_photo(flat, config.load_settings(), None)
check("纯色图不崩", res2 is not None, (res2["flags"] if res2 else None))

print("\n" + "=" * 50)
print("失败项:", fails if fails else "无")
sys.exit(0 if not fails else 1)