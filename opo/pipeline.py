# -*- coding: utf-8 -*-
"""一体化流水线：扫描 -> 分析 -> 分类 -> 去重 -> 报告。"""
from __future__ import annotations

import os
import threading
import time

from . import classify, config, scanner
from .library import Library
from .util import fmt_seconds, human_size


class Progress:
    """线程安全的进度汇聚器，供 GUI 轮询。"""

    def __init__(self):
        self.lock = threading.Lock()
        self.phase = ""
        self.done = 0
        self.total = 0
        self.detail = ""
        self.started = 0.0
        self.finished = 0.0
        self.running = False
        self.log = []
        self.cancel = threading.Event()

    def start(self, phase, total=0):
        with self.lock:
            self.phase = phase
            self.done = 0
            self.total = total
            self.detail = ""
            self.started = time.time()
            self.finished = 0.0
            self.running = True
            self.cancel.clear()

    def update(self, phase, done, total, detail=""):
        with self.lock:
            self.phase = phase
            self.done = done
            if total:
                self.total = total
            self.detail = detail
            line = "[%s] %s %d/%d %s" % (time.strftime("%H:%M:%S"), phase, done, total, detail)
            self.log.append(line)
            if len(self.log) > 400:
                self.log = self.log[-400:]

    def finish(self):
        with self.lock:
            self.running = False
            self.finished = time.time()

    def snapshot(self):
        with self.lock:
            el = (self.finished or time.time()) - (self.started or time.time())
            pct = (100.0 * self.done / self.total) if self.total else 0.0
            return {
                "phase": self.phase, "done": self.done, "total": self.total,
                "detail": self.detail, "percent": round(pct, 1),
                "running": self.running, "elapsed": el,
                "elapsed_h": fmt_seconds(el),
            }


def run_full(lib: Library, cfg: dict, roots=None, prog: Progress = None,
             do_scan=True, do_analyze=True, do_tag=True, do_events=True,
             do_dup=True, redo=False, on_progress=None,
             reanalyze_all=False) -> dict:
    prog = prog or Progress()

    def report(phase, done=0, total=0, detail=""):
        """同时喂给 Progress 与外部回调（HTTP 进度接口）。"""
        prog.update(phase, done, total, detail)
        if on_progress:
            try:
                on_progress(phase, done, total, detail)
            except Exception:
                pass

    t0 = time.time()
    report_data = {}
    try:
        # 实验室开关只影响质量待修与照片修复；
        # 去重、地点识别等已是常驻功能，不再受开关限制。
        lab_places = True

        if do_scan:
            prog.start("scan")
            roots = roots if roots is not None else lib.roots()
            if not roots:
                report("scan", 0, 0, "没有设置任何照片目录")
                return {"error": "没有设置任何照片目录"}
            report("扫描目录", 0, 0, "正在枚举文件…")

            def scan_progress(phase, done, total, detail=""):
                report("扫描目录", done, total, detail)

            r = scanner.scan_roots(lib, cfg, roots, scan_progress, prog.cancel,
                                   force=redo)
            report_data["scan"] = r
            report("扫描目录", r["added"], max(1, r["added"]),
                   "新登记 %d 个文件" % r["added"])
        if do_analyze:
            prog.start("analyze")

            def analyze_progress(phase, done, total, detail=""):
                report("分析照片", done, total, detail)

            # 实验室开关变更后需要把已分析的照片也重算一遍
            r = scanner.analyze_pending(lib, cfg, progress=analyze_progress,
                                        cancel=prog.cancel,
                                        redo=redo or reanalyze_all)
            report_data["analyze"] = r

        if do_tag:
            prog.start("tag")

            def tag_progress(phase, done, total, detail=""):
                report("生成标签", done, total, detail)

            r = classify.auto_tag(lib, cfg, tag_progress, prog.cancel)
            report_data["tag"] = r

            # 地点识别属于实验室功能，未开启时不写地点
            if lab_places:
                report("识别地点", 0, 0, "按 GPS 与目录名推断地点…")
                report_data["place"] = classify.backfill_places(lib, report)

        if do_events:
            prog.start("events")
            report("聚类事件", 0, 1, "按时间与地点分组…")
            r = classify.cluster_events(lib, cfg)
            report_data["events"] = r

        if do_dup:
            prog.start("dedup")
            report("查找重复", 0, 0, "计算感知哈希相似度…")

            def dup_progress(phase, done, total, detail=""):
                report("查找重复", done, total, detail)

            r = classify.find_duplicates(lib, cfg, dup_progress, prog.cancel)
            report_data["dup"] = r

        report_data["elapsed"] = time.time() - t0
        report_data["stats"] = lib.stats()
        report("完成", 1, 1, "完成，用时 %s" % fmt_seconds(report_data["elapsed"]))
    except Exception as e:
        import traceback
        report_data["error"] = "%s: %s" % (type(e).__name__, e)
        report_data["traceback"] = traceback.format_exc()
        report("出错", 0, 0, str(e))
    finally:
        prog.finish()
        scanner.reset_pool()
    return report_data


def quick_add(lib: Library, cfg: dict, folder: str, prog: Progress = None) -> dict:
    """加一个目录并立刻跑完整流水线。"""
    lib.add_root(folder)
    return run_full(lib, cfg, roots=[folder], prog=prog)
