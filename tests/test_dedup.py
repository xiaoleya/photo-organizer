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
from PIL import Image, ImageEnhance
ROOT = r"E:\agent xm\.deeepseek\old-photo\old-photo-organizer"
sys.path.insert(0, ROOT)
os.environ["OPO_HOME"] = os.path.join(ROOT, "test_dedup_home")
shutil.rmtree(os.environ["OPO_HOME"], ignore_errors=True)

from opo.library import Library
from opo import scanner, classify, config


def main():
    from opo.library import Library
    from opo import scanner, classify, config

    src_dir = os.path.join(ROOT, "samples")
    out = os.path.join(ROOT, "dedup_cases")
    shutil.rmtree(out, ignore_errors=True); os.makedirs(out)
    base = Image.open(os.path.join(src_dir, "1985-06-12 春节全家福.jpg")).convert("RGB")

    # A组：同一张的多种变体
    base.save(os.path.join(out, "A_原始.jpg"), quality=92)
    base.resize((320, 240), Image.LANCZOS).save(os.path.join(out, "A_缩到320.jpg"), quality=70)
    ImageEnhance.Brightness(base).enhance(1.18).save(os.path.join(out, "A_变亮.jpg"), quality=85)
    ImageEnhance.Contrast(base).enhance(0.85).save(os.path.join(out, "A_降对比.jpg"), quality=80)
    base.rotate(2.0, expand=True, fillcolor=(255,255,255)).save(os.path.join(out, "A_微旋转2度.jpg"), quality=85)
    base.crop((40, 30, 1240, 930)).save(os.path.join(out, "A_裁掉边缘.jpg"), quality=88)
    # B组：另一张不同照片的变体（不应与 A 组混淆）
    other = Image.open(os.path.join(src_dir, "1992-10-01 旅行.jpg")).convert("RGB")
    other.save(os.path.join(out, "B_原始.jpg"), quality=92)
    other.resize((400, 300), Image.LANCZOS).save(os.path.join(out, "B_缩小.jpg"), quality=70)
    # C组：完全不同的独立照片
    Image.open(os.path.join(src_dir, "DSC_0071.JPG")).convert("RGB").save(os.path.join(out, "C_独立.jpg"), quality=90)

    cfg = config.load_settings()
    lib = Library()
    scanner.scan_roots(lib, cfg, [out], force=True)
    print("analyze:", scanner.analyze_pending(lib, cfg, redo=True))
    r = classify.find_duplicates(lib, cfg)
    print("dup:", r)

    print("\n--- 分组结果 ---")
    for g in lib.q("SELECT * FROM dup_groups ORDER BY id"):
        names = [x["name"] for x in lib.q("SELECT name FROM photos WHERE dup_group=?", (g["id"],))]
        print("组%d %-8s %d张: %s" % (g["id"], g["kind"], g["count"], ", ".join(sorted(names))))
    n_dup = lib.count("dup_group IS NOT NULL")
    print("\n被判重复的总数:", n_dup, "/", lib.count())

    # 断言
    groups = [sorted(x["name"] for x in lib.q("SELECT name FROM photos WHERE dup_group=?", (g["id"],)))
              for g in lib.q("SELECT id FROM dup_groups")]
    ok_a = any(sum(1 for n in g if n.startswith("A_")) >= 5 for g in groups)
    ok_c = not any("C_独立.jpg" in g for g in groups)
    ok_b = any(sum(1 for n in g if n.startswith("B_")) == 2 for g in groups)
    print("\nA组变体召回>=5 :", "PASS" if ok_a else "FAIL")
    print("B组两张合并    :", "PASS" if ok_b else "FAIL")
    print("C独立图未误杀  :", "PASS" if ok_c else "FAIL")
    sys.exit(0 if (ok_a and ok_b and ok_c) else 1)


if __name__ == "__main__":
    main()