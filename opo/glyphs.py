# -*- coding: utf-8 -*-
"""线性图标：用基本图形绘制，不依赖字体字形。

MangoDisk 用的是 24x24、1.7px 描边的 lucide 风格线性图标。
Tk 无法渲染 SVG，这里用 line / arc / oval / polygon 复刻同一套视觉语言：
统一 24px 画布、1.7px 线宽、圆角端点、无填充。
"""
from __future__ import annotations

import math

STROKE = 1.7


def _round_line(cv, x1, y1, x2, y2, color, w=STROKE):
    cv.create_line(x1, y1, x2, y2, fill=color, width=w,
                   capstyle="round", joinstyle="round")


def _rrect(cv, x1, y1, x2, y2, r, color, w=STROKE):
    """圆角矩形描边（用四段线 + 四个弧拼，Tk 无直接圆角矩形描边）。"""
    cv.create_arc(x1, y1, x1 + 2 * r, y1 + 2 * r, start=90, extent=90,
                  style="arc", outline=color, width=w)
    cv.create_arc(x2 - 2 * r, y1, x2, y1 + 2 * r, start=0, extent=90,
                  style="arc", outline=color, width=w)
    cv.create_arc(x1, y2 - 2 * r, x1 + 2 * r, y2, start=180, extent=90,
                  style="arc", outline=color, width=w)
    cv.create_arc(x2 - 2 * r, y2 - 2 * r, x2, y2, start=270, extent=90,
                  style="arc", outline=color, width=w)
    _round_line(cv, x1 + r, y1, x2 - r, y1, color, w)
    _round_line(cv, x1 + r, y2, x2 - r, y2, color, w)
    _round_line(cv, x1, y1 + r, x1, y2 - r, color, w)
    _round_line(cv, x2, y1 + r, x2, y2 - r, color, w)


def draw(cv, name, cx, cy, color, size=24, width=STROKE):
    """在 (cx, cy) 居中绘制 24x24 图标。"""
    s = size / 24.0
    w = max(1, round(width * s, 1))

    def L(x1, y1, x2, y2):
        _round_line(cv, cx + x1 * s, cy + y1 * s, cx + x2 * s, cy + y2 * s, color, w)

    def O(x1, y1, x2, y2, fill=""):
        cv.create_oval(cx + x1 * s, cy + y1 * s, cx + x2 * s, cy + y2 * s,
                       outline=color, width=w, fill=fill)

    def R(x1, y1, x2, y2, r=2):
        _rrect(cv, cx + x1 * s, cy + y1 * s, cx + x2 * s, cy + y2 * s,
               max(1.0, r * s), color, w)

    def P(pts, fill=""):
        flat = []
        for (x, y) in pts:
            flat.extend([cx + x * s, cy + y * s])
        cv.create_polygon(flat, outline=color, fill=fill, width=w,
                          joinstyle="round")

    if name == "library":                 # 图片：圆角框 + 山 + 太阳
        R(-9, -8, 9, 8, 2.5)
        P([(-9, 4), (-3, -3), (2, 3), (5, 0), (9, 4)], fill="")
        O(-4.5, -5, -1.5, -2)
    elif name == "timeline":              # 时钟
        O(-9, -9, 9, 9)
        L(0, -5, 0, 0)
        L(0, 0, 4, 2)
    elif name == "tags":                  # 标签
        P([(-9, 0), (-3, -7), (8, -7), (8, 2), (1, 8), (-9, 0)])
        O(3.5, -3.5, 6.5, -0.5)
    elif name == "events":                # 菱形/星芒
        P([(0, -9), (9, 0), (0, 9), (-9, 0)])
    elif name == "duplicates":            # 两张叠片
        R(-9, -8, 4, 4, 2)
        R(-3, -3, 9, 8, 2)
    elif name == "quality":               # 六边形
        P([(0, -9), (7.8, -4.5), (7.8, 4.5), (0, 9), (-7.8, 4.5), (-7.8, -4.5)])
    elif name == "restore":               # 星芒（修复）
        P([(0, -9), (2.4, -2.4), (9, 0), (2.4, 2.4),
           (0, 9), (-2.4, 2.4), (-9, 0), (-2.4, -2.4)])
    elif name == "organize":              # 归档：托盘 + 箭头
        L(-9, -3, -9, 7); L(-9, 7, 9, 7); L(9, 7, 9, -3)
        L(-4, 0, 4, 0); L(1, -3, 4, 0); L(1, 3, 4, 0)
    elif name == "settings":              # 齿轮
        O(-3.5, -3.5, 3.5, 3.5)
        for a in range(0, 360, 45):
            r1, r2 = 5.5, 9.0
            L(r1 * math.cos(math.radians(a)), r1 * math.sin(math.radians(a)),
              r2 * math.cos(math.radians(a)), r2 * math.sin(math.radians(a)))
    elif name == "search":
        O(-8, -8, 2.5, 2.5)
        L(1.5, 1.5, 8, 8)
    elif name == "plus":
        L(-7, 0, 7, 0); L(0, -7, 0, 7)
    elif name == "camera":                # 品牌：光圈
        R(-9, -6, 9, 7, 2.5)
        O(-4, -4, 4, 4)
    elif name == "chevron_down":
        L(-5, -2, 0, 3); L(0, 3, 5, -2)
    elif name == "x":
        L(-5, -5, 5, 5); L(5, -5, -5, 5)
    elif name == "check":
        L(-6, 0, -2, 4); L(-2, 4, 6, -5)
    elif name == "folder":
        P([(-9, -6), (-2, -6), (0, -3), (9, -3), (9, 7), (-9, 7)])
    else:
        O(-6, -6, 6, 6)
