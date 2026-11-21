# -*- coding: utf-8 -*-
"""字体与主题令牌：一处创建，全局复用。

字号严格按 MangoDisk 的 font-content-* 选型换算，并用
Tk 的 tkfont 显式测量宽度，避免自动换行把排版撑坏。

清晰度要点（Windows）：
  · 优先 Segoe UI Variable Text / Segoe UI，它们有小字号 hinting
  · 中文回退到 Microsoft YaHei UI（UI 版比普通雅黑更紧凑）
  · 不使用任何 italic / 过细字重，暗底细字最容易发虚
"""
from __future__ import annotations

import tkinter as tk
from tkinter import font as tkfont

from . import design


class Tokens:
    """当前主题的实色表 + 一组预先构造的字体对象。"""

    def __init__(self, root, theme="dark"):
        self.theme = theme
        self.c = design.surface(design.palette(theme))

        text = design.FONT_STACK
        disp = design.FONT_DISPLAY
        mono = design.FONT_MONO
        s = design.FONT_SIZE

        def F(size, weight="normal", family=text):
            return tkfont.Font(root, family=family, size=size, weight=weight)

        self.f_meta = F(s["meta"])
        self.f_secondary = F(s["secondary"])
        self.f_body = F(s["body"])
        self.f_body_b = F(s["body"], "bold")
        self.f_primary = F(s["primary"])
        self.f_primary_b = F(s["primary"], "bold")
        self.f_nav = F(s["nav"])
        self.f_nav_b = F(s["nav"], "bold")
        self.f_section = F(s["section"], "bold", disp)
        self.f_empty_title = F(s["empty_title"], "bold", disp)
        self.f_page_title = F(s["page_title"], "bold", disp)
        self.f_mono = F(s["secondary"], family=mono)

        # 兼容旧名
        self.f_caption = self.f_meta
        self.f_small = self.f_secondary
        self.f_title = self.f_section
        self.f_h1 = self.f_page_title
        self.f_h2 = self.f_section

    def __getitem__(self, k):
        return self.c[k]

    def elide(self, text, font, max_px):
        """按像素宽度截断并补省略号（Canvas 文字不换行）。"""
        if not text:
            return ""
        if font.measure(text) <= max_px:
            return text
        ell = "…"
        lo, hi = 0, len(text)
        while lo < hi:
            mid = (lo + hi + 1) // 2
            if font.measure(text[:mid] + ell) <= max_px:
                lo = mid
            else:
                hi = mid - 1
        return text[:lo] + ell
