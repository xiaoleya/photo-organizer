# -*- coding: utf-8 -*-
"""报告导出：HTML 相册清单、CSV 元数据表。"""
from __future__ import annotations

import csv
import html
import os
import time

from .util import human_size, parse_iso


def export_csv(lib, out_path: str, where="", args=()) -> str:
    rows = lib.all_photos(where, args)
    cols = ["id", "path", "name", "ext", "size", "dt", "dt_source", "year", "month",
            "width", "height", "megapixels", "quality_score", "age_score", "blur",
            "brightness", "contrast", "saturation", "sepia", "noise", "flags",
            "camera_make", "camera_model", "lat", "lon", "dup_group", "is_best", "thumb"]
    os.makedirs(os.path.dirname(out_path) or ".", exist_ok=True)
    with open(out_path, "w", newline="", encoding="utf-8-sig") as f:
        w = csv.writer(f)
        w.writerow(cols + ["tags", "size_h"])
        for r in rows:
            tags = "|".join(lib.tags_of(r["id"]))
            w.writerow([r[c] if c in r.keys() else "" for c in cols]
                       + [tags, human_size(r["size"])])
    return out_path


_CSS = """
body{font-family:"Microsoft YaHei","PingFang SC",system-ui,sans-serif;background:#14161a;color:#e8e8ea;margin:0;padding:24px}
h1{font-size:22px;margin:0 0 6px}
.sub{color:#8b93a1;font-size:13px;margin-bottom:18px}
.stats{display:flex;flex-wrap:wrap;gap:10px;margin-bottom:22px}
.card{background:#1d2026;border:1px solid #2a2f38;border-radius:10px;padding:10px 14px;min-width:110px}
.card b{display:block;font-size:19px;color:#7fd1c1}
.card span{font-size:12px;color:#8b93a1}
h2{font-size:16px;margin:26px 0 10px;border-left:3px solid #7fd1c1;padding-left:8px}
.grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(168px,1fr));gap:12px}
.item{background:#1d2026;border:1px solid #2a2f38;border-radius:8px;overflow:hidden}
.item img{width:100%;height:128px;object-fit:cover;display:block;background:#0e1013}
.meta{padding:6px 8px;font-size:11px;color:#98a0ac;line-height:1.5}
.meta b{color:#d6dae2;font-weight:500;display:block;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.tag{display:inline-block;background:#26303a;color:#8fd4c6;border-radius:4px;padding:0 4px;margin:1px 2px 0 0;font-size:10px}
table{border-collapse:collapse;width:100%;font-size:12px}
td,th{border-bottom:1px solid #262b33;padding:6px 8px;text-align:left}
th{color:#8b93a1;font-weight:500}
"""


def export_html(lib, out_path: str, where="", args=(), title="老照片整理报告",
                max_items=600, use_thumbs=True, group="year") -> str:
    rows = lib.all_photos(where, args)
    st = lib.stats()
    tdir = os.path.dirname(os.path.abspath(out_path))
    parts = ["<!doctype html><html lang='zh-CN'><head><meta charset='utf-8'>",
             "<title>%s</title><style>%s</style></head><body>" % (html.escape(title), _CSS),
             "<h1>%s</h1>" % html.escape(title),
             "<div class='sub'>生成时间 %s · 资源库 %s</div>"
             % (time.strftime("%Y-%m-%d %H:%M:%S"), html.escape(os.path.basename(lib.path)))]
    parts.append("<div class='stats'>")
    for label, val in (("照片总数", st["total"]), ("已分析", st["analyzed"]),
                       ("有时间", st["with_dt"]), ("缺时间", st["no_dt"]),
                       ("重复照片", st["dups"]), ("占用空间", human_size(st["bytes"])),
                       ("标签数", st["tags"])):
        parts.append("<div class='card'><b>%s</b><span>%s</span></div>"
                     % (html.escape(str(val)), label))
    parts.append("</div>")
    evs = lib.q("SELECT title,count,start_dt FROM events ORDER BY start_dt DESC LIMIT 30")
    if evs:
        parts.append("<h2>事件分组（最近 30 个）</h2><table><tr><th>事件</th><th>照片数</th><th>开始时间</th></tr>")
        for e in evs:
            parts.append("<tr><td>%s</td><td>%d</td><td>%s</td></tr>"
                         % (html.escape(e["title"] or ""), e["count"] or 0,
                            html.escape((e["start_dt"] or "")[:10])))
        parts.append("</table>")

    dups = lib.q("SELECT id,kind,count,saved_bytes FROM dup_groups ORDER BY saved_bytes DESC LIMIT 20")
    if dups:
        parts.append("<h2>重复照片组</h2><table><tr><th>组</th><th>类型</th><th>张数</th><th>可节省</th></tr>")
        for d in dups:
            parts.append("<tr><td>#%d</td><td>%s</td><td>%d</td><td>%s</td></tr>"
                         % (d["id"], "完全相同" if d["kind"] == "exact" else "近似",
                            d["count"] or 0, human_size(d["saved_bytes"] or 0)))
        parts.append("</table>")

    if st["years"]:
        parts.append("<h2>年份分布</h2><table><tr><th>年份</th><th>数量</th></tr>")
        for y, c in st["years"]:
            parts.append("<tr><td>%d</td><td>%d</td></tr>" % (y, c))
        parts.append("</table>")
    parts.append("<h2>照片清单（前 %d 张）</h2><div class='grid'>" % max_items)
    for r in rows[:max_items]:
        thumb = r["thumb"]
        src = ""
        if use_thumbs and thumb and os.path.exists(thumb):
            try:
                src = os.path.relpath(thumb, tdir).replace("\\", "/")
            except Exception:
                src = "file:///" + thumb.replace("\\", "/")
        elif r["path"]:
            src = "file:///" + r["path"].replace("\\", "/")
        tags = lib.tags_of(r["id"])[:6]
        taghtml = "".join("<span class='tag'>%s</span>" % html.escape(t) for t in tags)
        parts.append(
            "<div class='item'><img loading='lazy' src='%s' alt=''>"
            "<div class='meta'><b>%s</b>%s<br>%s · %s<br>%s</div></div>"
            % (html.escape(src), html.escape(os.path.basename(r["path"] or "")),
               html.escape(r["dt"] or "无日期"),
               html.escape("%dx%d" % (r["width"] or 0, r["height"] or 0)),
               human_size(r["size"]),
               taghtml))
    parts.append("</div></body></html>")
    os.makedirs(os.path.dirname(os.path.abspath(out_path)) or ".", exist_ok=True)
    with open(out_path, "w", encoding="utf-8") as f:
        f.write("\n".join(parts))
    return out_path