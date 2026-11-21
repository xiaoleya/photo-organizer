# -*- coding: utf-8 -*-
"""命令行入口：python -m opo.cli <命令> [参数]"""
from __future__ import annotations

import argparse
import json
import os
import sys
import time

from . import config
from .library import Library
from .util import fmt_seconds, human_size


def _force_utf8_stdout():
    """Windows 控制台默认 GBK，中文路径/标签会变乱码，这里切到 UTF-8。"""
    import io
    for name in ("stdout", "stderr"):
        s = getattr(sys, name, None)
        enc = (getattr(s, "encoding", "") or "").lower()
        if s is not None and enc not in ("utf-8", "utf8"):
            try:
                setattr(sys, name, io.TextIOWrapper(
                    s.buffer, encoding="utf-8", errors="replace", line_buffering=True))
            except Exception:
                pass


def _prog_printer():
    def f(phase, done, total, detail=""):
        pct = ("%5.1f%%" % (100.0 * done / total)) if total else "     "
        sys.stdout.write("\r  %-10s %s %d/%d %s" % (phase, pct, done, total, detail[:40]))
        sys.stdout.flush()
    return f


def cmd_scan(a):
    from . import scanner
    lib, cfg = _lib(a)
    r = scanner.scan_roots(lib, cfg, a.dirs or None, force=a.force)
    print("扫描完成：新登记 %(added)d，跳过 %(skipped)d，错误 %(errors)d" % r)
    return 0


def cmd_analyze(a):
    from . import scanner
    lib, cfg = _lib(a)
    if a.workers:
        cfg["analyze_workers"] = a.workers
    r = scanner.analyze_pending(lib, cfg, limit=a.limit, redo=a.redo)
    print("\n分析完成：%(done)d / %(total)d，失败 %(failed)d" % r)
    return 0


def cmd_classify(a):
    from . import classify
    lib, cfg = _lib(a)
    r1 = classify.auto_tag(lib, cfg)
    print("自动标签：%d 张照片，%d 条标签关系" % (r1["photos"], r1["tags"]))
    r2 = classify.cluster_events(lib, cfg)
    print("事件聚类：%d 个事件，覆盖 %d 张" % (r2["events"], r2["photos"]))
    return 0


def cmd_dup(a):
    from . import classify
    lib, cfg = _lib(a)
    r = classify.find_duplicates(lib, cfg)
    print("重复检测：%d 组，%d 张，可省 %s" % (r["groups"], r["photos"], human_size(r["saved_bytes"])))
    return 0


def cmd_run(a):
    from . import pipeline
    lib, cfg = _lib(a)
    prog = pipeline.Progress()
    print("开始整理流程…")
    r = pipeline.run_full(lib, cfg, roots=a.dirs or None, prog=prog,
                          redo=a.redo)
    print("\n" + json.dumps({k: v for k, v in r.items() if k != "traceback"},
                            ensure_ascii=False, indent=2, default=str))
    return 0 if "error" not in r else 1


def cmd_tags(a):
    lib, _ = _lib(a)
    for name, kind, cnt in lib.all_tags():
        print("%-24s %-8s %5d" % (name, kind, cnt))
    return 0


def cmd_list(a):
    lib, _ = _lib(a)
    rows = lib.all_photos(a.where or "", ())
    print("%-6s %-19s %-12s %-9s %s" % ("ID", "拍摄时间", "尺寸", "评分", "路径"))
    for r in rows[: a.limit or 200]:
        print("%-6d %-19s %-12s %-9s %s" % (
            r["id"], r["dt"] or "-", "%dx%d" % (r["width"] or 0, r["height"] or 0),
            "%.0f" % (r["quality_score"] or 0), r["path"]))
    return 0


def cmd_organize(a):
    from . import organizer
    lib, _ = _lib(a)
    if not a.dest:
        print("需要 --dest 目标目录")
        return 2
    plan = organizer.plan_organize(lib, scheme=a.scheme, template=a.template,
                                   limit=a.limit)
    print("计划整理 %d 张 -> %s（模式 %s）" % (len(plan), a.dest, a.mode))
    for pid, rel in plan[:10]:
        print("   ", rel)
    if a.dry_run:
        print("（预览模式，未执行）")
        return 0
    r = organizer.execute_organize(lib, a.dest, plan, mode=a.mode,
                                   on_conflict=a.on_conflict)
    print("完成：%(done)d 张，跳过 %(skipped)d，错误 %(errors)d，批次 %(batch)s" % r)
    return 0


def cmd_rollback(a):
    from . import organizer
    lib, _ = _lib(a)
    r = organizer.rollback(lib, a.batch)
    print("回滚：恢复 %(restored)d，错误 %(errors)d" % r)
    return 0


def cmd_restore(a):
    from . import restore
    from .imaging import open_image, quality_metrics
    lib, _ = _lib(a)
    os.makedirs(a.out, exist_ok=True)
    rows = lib.all_photos(a.where or "", ())
    if a.limit:
        rows = rows[: a.limit]
    preset = restore.PRESETS.get(a.preset, {})
    n = 0
    for r in rows:
        im = open_image(r["path"])
        if im is None:
            continue
        m = {"noise": r["noise"], "sepia": r["sepia"], "contrast": r["contrast"],
             "saturation": r["saturation"], "blur": r["blur"]}
        out = restore.auto_enhance(im, m, preset)
        dst = os.path.join(a.out, os.path.splitext(os.path.basename(r["path"]))[0] + "_修复.jpg")
        out.save(dst, "JPEG", quality=a.quality, optimize=True)
        n += 1
        print("\r  修复 %d/%d" % (n, len(rows)), end="")
    print("\n修复完成 %d 张 -> %s" % (n, a.out))
    return 0


def cmd_report(a):
    from . import report
    lib, _ = _lib(a)
    if a.csv:
        p = report.export_csv(lib, a.csv)
        print("CSV:", p)
    p = report.export_html(lib, a.html or os.path.join(os.getcwd(), "report.html"),
                           title=a.title or "老照片整理报告")
    print("HTML:", p)
    return 0


def cmd_gui(a):
    from . import gui
    return gui.main()


def _lib(a):
    cfg = config.load_settings()
    if a.db:
        return Library(a.db), cfg
    return Library(), cfg


def build_parser():
    p = argparse.ArgumentParser(prog="opo", description="老照片整理与分类工具")
    p.add_argument("--db", help="指定资源库文件（默认用户目录）")
    sub = p.add_subparsers(dest="cmd")

    s = sub.add_parser("scan", help="扫描目录登记照片")
    s.add_argument("dirs", nargs="*")
    s.add_argument("--force", action="store_true")
    s.set_defaults(func=cmd_scan)

    s = sub.add_parser("analyze", help="分析照片（EXIF/质量/哈希/缩略图）")
    s.add_argument("--limit", type=int, default=0)
    s.add_argument("--redo", action="store_true")
    s.add_argument("--workers", type=int, default=0)
    s.set_defaults(func=cmd_analyze)

    s = sub.add_parser("classify", help="自动标签 + 事件聚类")
    s.set_defaults(func=cmd_classify)

    s = sub.add_parser("dup", help="查找重复照片")
    s.set_defaults(func=cmd_dup)

    s = sub.add_parser("run", help="一键全流程")
    s.add_argument("dirs", nargs="*")
    s.add_argument("--redo", action="store_true")
    s.set_defaults(func=cmd_run)

    s = sub.add_parser("tags", help="列出标签")
    s.set_defaults(func=cmd_tags)

    s = sub.add_parser("list", help="列出照片")
    s.add_argument("--where", default="")
    s.add_argument("--limit", type=int, default=200)
    s.set_defaults(func=cmd_list)

    s = sub.add_parser("organize", help="按分类整理归档")
    s.add_argument("--dest", required=False)
    s.add_argument("--scheme", default="year_month")
    s.add_argument("--template", default="{date}_{time}_{name}")
    s.add_argument("--mode", default="copy", choices=["copy", "move", "link"])
    s.add_argument("--on-conflict", default="rename",
                   choices=["rename", "skip", "overwrite"])
    s.add_argument("--limit", type=int, default=0)
    s.add_argument("--dry-run", action="store_true")
    s.set_defaults(func=cmd_organize)

    s = sub.add_parser("rollback", help="撤销整理")
    s.add_argument("--batch")
    s.set_defaults(func=cmd_rollback)

    s = sub.add_parser("restore", help="批量修复老照片")
    s.add_argument("--out", required=True)
    s.add_argument("--where", default="")
    s.add_argument("--preset", default="标准修复")
    s.add_argument("--quality", type=int, default=95)
    s.add_argument("--limit", type=int, default=0)
    s.set_defaults(func=cmd_restore)

    s = sub.add_parser("report", help="导出 HTML/CSV 报告")
    s.add_argument("--html")
    s.add_argument("--csv")
    s.add_argument("--title")
    s.set_defaults(func=cmd_report)

    s = sub.add_parser("gui", help="启动图形界面")
    s.set_defaults(func=cmd_gui)
    return p


def main(argv=None):
    _force_utf8_stdout()
    p = build_parser()
    a = p.parse_args(argv)
    if not getattr(a, "cmd", None):
        p.print_help()
        return 0
    try:
        return a.func(a) or 0
    except KeyboardInterrupt:
        print("\n已中断")
        return 130


if __name__ == "__main__":
    sys.exit(main())