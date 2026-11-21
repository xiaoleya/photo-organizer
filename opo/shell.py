# -*- coding: utf-8 -*-
"""应用外壳：结构照抄 MangoDisk。

对照 md-window-titlebar.vue + md-sidebar.vue + md-app-shell.vue：

    ┌──────────────────────────────────────────────────────┐
    │ [icon] 老照片整理            ─   □   ✕   ← 自绘标题栏 │ 40px
    ├────────────┬─────────────────────────────────────────┤
    │ 导航分组    │  页面标题（22px 粗）        [操作按钮]  │ 58px
    │ ● 40px 条目 │  ─────────────────────────────────      │
    │             │  内容区（20px 内边距）                  │
    └────────────┴─────────────────────────────────────────┘

关键点：
  · 窗口用 overrideredirect 去掉系统标题栏，标题栏完全自绘
  · 标题栏可拖动（拖 <Button-1>/<B1-Motion> 调 root.geometry）
  · 最小化/最大化/关闭自己实现，hover 用 foreground 9% 叠加
"""
from __future__ import annotations

import os
import tkinter as tk

from . import design, glyphs
from .fonts import Tokens
from .widgets import Button, Divider, NavItem, Painter, Select

NAV_GROUPS = [
    ("浏览", [
        ("library", "library", "照片库"),
        ("timeline", "timeline", "时间轴"),
        ("tags", "tags", "标签"),
        ("events", "events", "事件"),
    ]),
    ("处理", [
        ("duplicates", "duplicates", "重复照片"),
        ("quality", "quality", "质量待修"),
        ("restore", "restore", "老照片修复"),
    ]),
    ("整理", [
        ("organize", "organize", "整理归档"),
        ("settings", "settings", "设置"),
    ]),
]

META = {
    "library": ("照片库", "浏览、筛选与批量操作"),
    "timeline": ("时间轴", "按年月浏览老照片"),
    "tags": ("标签", "管理自动与自定义标签"),
    "events": ("事件", "按拍摄时间与地点自动分组"),
    "duplicates": ("重复照片", "找出同一张照片的多个副本"),
    "quality": ("质量待修", "模糊、过暗、褪色等问题照片"),
    "restore": ("老照片修复", "去黄、去划痕、增强对比"),
    "organize": ("整理归档", "按规则重命名与分类输出"),
    "settings": ("设置", "阈值与偏好"),
}


class TitleBar(tk.Canvas):
    """自绘标题栏（对应 md-window-titlebar.vue）。

    左：应用图标 28x28 + 名称（14px 600）
    右：三个 46px 宽的控制按钮，hover 时整块叠加 foreground 9%
    整条可拖动窗口。
    """

    H = design.TITLEBAR_H
    CTRL_W = 52

    def __init__(self, master, theme, app):
        self.theme = theme
        self.app = app
        c = theme
        super().__init__(master, height=self.H, bg=c["sidebar"],
                         highlightthickness=0, bd=0)
        self._drag = None
        self._hover = None
        self.bind("<Configure>", lambda e: self._draw())
        self.bind("<Button-1>", self._press)
        self.bind("<B1-Motion>", self._move)
        self.bind("<ButtonRelease-1>", lambda e: setattr(self, "_drag", None))
        self.bind("<Double-Button-1>", lambda e: self.app.toggle_maximize())
        self.bind("<Motion>", self._motion)
        self.bind("<Leave>", lambda e: self._set_hover(None))
        self._draw()

    # ---------------------------------------------------------- 拖动
    def _press(self, e):
        w = self.winfo_width()
        # 右侧控制区不参与拖动
        if e.x > w - self.CTRL_W * 3:
            which = int((e.x - (w - self.CTRL_W * 3)) // self.CTRL_W)
            if which == 0:
                self.app.minimize()
            elif which == 1:
                self.app.toggle_maximize()
            else:
                self.app.on_close()
            return
        self._drag = (e.x_root, e.y_root,
                      self.app.root.winfo_x(), self.app.root.winfo_y())

    def _move(self, e):
        if not self._drag:
            return
        x0, y0, wx, wy = self._drag
        self.app.root.geometry("+%d+%d" % (wx + e.x_root - x0, wy + e.y_root - y0))

    def _motion(self, e):
        w = self.winfo_width()
        if e.x > w - self.CTRL_W * 3:
            which = int((e.x - (w - self.CTRL_W * 3)) // self.CTRL_W)
        else:
            which = None
        if which != self._hover:
            self._hover = which
            self._draw()

    def _set_hover(self, v):
        self._hover = v
        self._draw()

    # ---------------------------------------------------------- 绘制
    def _draw(self):
        c = self.theme
        self.delete("all")
        w = self.winfo_width() or 800
        h = self.H

        # 应用图标
        glyphs.draw(self, "camera", 30, h // 2, c["accent"], size=30)
        self.create_text(52, h // 2, text="照片整理", anchor="w",
                         fill=c["label_primary"], font=c.f_nav_b)

        # 右侧控制按钮
        x0 = w - self.CTRL_W * 3
        if self._hover is not None:
            hx = x0 + self._hover * self.CTRL_W
            fill = self.theme["error"] if self._hover == 2 else design.mix(
                c["label_primary"], c["sidebar"], 0.09)
            self.create_rectangle(hx, 0, hx + self.CTRL_W, h,
                                  fill=fill, outline="")
        ink = c["label_secondary"]
        cy = h // 2
        # 最小化
        cx1 = x0 + self.CTRL_W // 2
        self.create_line(cx1 - 5, cy, cx1 + 5, cy, fill=ink, width=1.3)
        # 最大化/还原
        cx2 = x0 + self.CTRL_W + self.CTRL_W // 2
        if self.app.is_maximized:
            self.create_rectangle(cx2 - 4, cy - 4, cx2 + 4, cy + 4,
                                  outline=ink, width=1.3)
            self.create_rectangle(cx2 - 6, cy - 6, cx2 + 2, cy + 2,
                                  outline=ink, width=1.3)
        else:
            self.create_rectangle(cx2 - 5, cy - 5, cx2 + 5, cy + 5,
                                  outline=ink, width=1.3)
        # 关闭
        cx3 = x0 + self.CTRL_W * 2 + self.CTRL_W // 2
        fg = "#FFFFFF" if self._hover == 2 else ink
        self.create_line(cx3 - 5, cy - 5, cx3 + 5, cy + 5, fill=fg, width=1.3)
        self.create_line(cx3 + 5, cy - 5, cx3 - 5, cy + 5, fill=fg, width=1.3)


class App(tk.Frame):
    def __init__(self, root, library=None):
        self.root = root
        if library is None:
            from .library import Library
            library = Library()
        self.library = library
        self.tk_ = Tokens(root, "dark")
        c = self.tk_

        self.is_maximized = False
        self._normal_geom = None

        # 去掉系统标题栏，标题栏自绘
        try:
            root.overrideredirect(True)
        except Exception:
            pass
        root.configure(bg=c["bg_base"])
        super().__init__(root, bg=c["bg_base"])
        self.pack(fill="both", expand=True)

        self.views = {}
        self.nav_items = {}
        self.current = None
        self._build_titlebar()
        self._build_body()
        self.show_view("library")

    # ------------------------------------------------------------ 标题栏
    def _build_titlebar(self):
        self.titlebar = TitleBar(self, self.tk_, self)
        self.titlebar.pack(fill="x", side="top")
        Divider(self, self.tk_).pack(fill="x", side="top")

    # ------------------------------------------------------------ 主体
    def _build_body(self):
        c = self.tk_
        body = tk.Frame(self, bg=c["sidebar"])
        body.pack(fill="both", expand=True)
        self.body_frame = body

        # ---- 侧栏（240px，对应 --layout-sidebar-expanded-width）
        sb = tk.Frame(body, bg=c["sidebar"], width=design.SIDEBAR_W)
        sb.pack(side="left", fill="y")
        sb.pack_propagate(False)
        self.sidebar = sb

        nav = tk.Frame(sb, bg=c["sidebar"])
        nav.pack(fill="x", padx=10, pady=(12, 6))
        for group_label, items in NAV_GROUPS:
            tk.Label(nav, text=group_label, bg=c["sidebar"],
                     fg=c["label_dimmed"], font=c.f_meta,
                     anchor="w").pack(fill="x", padx=12, pady=(10, 4))
            for key, icon, label in items:
                it = NavItem(nav, c, icon, label,
                             command=lambda k=key: self.show_view(k),
                             width=design.SIDEBAR_W - 20)
                it.pack(fill="x", pady=1)
                self.nav_items[key] = it

        # 底部统计
        foot = tk.Frame(sb, bg=c["sidebar"])
        foot.pack(side="bottom", fill="x", pady=(0, 14))
        Divider(foot, c).pack(fill="x", padx=16, pady=(0, 10))
        self.stat_lbl = tk.Label(foot, text="", bg=c["sidebar"],
                                 fg=c["label_tertiary"], font=c.f_secondary,
                                 justify="left", anchor="w")
        self.stat_lbl.pack(fill="x", padx=18)

        Divider(body, c, vertical=True).pack(side="left", fill="y")

        # ---- 右侧内容区（--workspace）
        main = tk.Frame(body, bg=c["bg_base"])
        main.pack(side="left", fill="both", expand=True)
        self.main = main

        # 页头 58px（--layout-page-header-height）
        head = tk.Frame(main, bg=c["bg_base"], height=design.PAGE_HEADER_H)
        head.pack(fill="x")
        head.pack_propagate(False)

        tbox = tk.Frame(head, bg=c["bg_base"])
        tbox.pack(side="left", padx=(design.PAGE_PAD_X, 0))
        self.title_lbl = tk.Label(tbox, text="照片库", bg=c["bg_base"],
                                  fg=c["label_primary"], font=c.f_page_title,
                                  anchor="w")
        self.title_lbl.pack(anchor="w")
        self.sub_lbl = tk.Label(tbox, text="", bg=c["bg_base"],
                                fg=c["label_tertiary"], font=c.f_body, anchor="w")
        self.sub_lbl.pack(anchor="w")

        right = tk.Frame(head, bg=c["bg_base"])
        right.pack(side="right", padx=(0, design.PAGE_PAD_X))

        # 搜索框（高 30px，对齐 --layout-workspace-control-height）
        sbox = tk.Frame(right, bg=c["bg_layer1"], highlightthickness=1,
                        highlightbackground=c["border_l1"],
                        highlightcolor=c["accent"])
        sbox.pack(side="left", padx=(0, 10))
        sc = tk.Canvas(sbox, width=20, height=20, bg=c["bg_layer1"],
                       highlightthickness=0, bd=0)
        sc.pack(side="left", padx=(12, 5), pady=7)
        glyphs.draw(sc, "search", 10, 10, c["label_tertiary"], size=18)
        self.search_var = tk.StringVar()
        self.search_entry = tk.Entry(sbox, textvariable=self.search_var, width=22,
                                     bg=c["bg_layer1"], fg=c["label_primary"],
                                     insertbackground=c["accent"], relief="flat",
                                     bd=0, font=c.f_primary, highlightthickness=0)
        self.search_entry.pack(side="left", padx=(0, 12), pady=7)
        self.search_entry.bind("<Return>", lambda e: self._on_search())

        Button(right, c, "添加目录", self._add_folder, "outline",
               icon="plus").pack(side="left", padx=4)
        Button(right, c, "全流程整理", self._run_pipeline, "primary").pack(
            side="left", padx=4)

        # 内容区
        wrap = tk.Frame(main, bg=c["bg_base"])
        wrap.pack(fill="both", expand=True,
                  padx=(design.PAGE_PAD_X, design.PAGE_PAD_X), pady=(0, 12))
        self.body = tk.Frame(wrap, bg=c["bg_base"])
        self.body.pack(fill="both", expand=True)

        self.status_lbl = tk.Label(main, text="就绪", bg=c["bg_base"],
                                   fg=c["label_tertiary"], font=c.f_meta,
                                   anchor="w")
        self.status_lbl.pack(side="bottom", fill="x",
                             padx=design.PAGE_PAD_X, pady=(0, 6))

    # ------------------------------------------------------------ 视图
    def show_view(self, key):
        from .views import build_view
        # 未知视图回退到照片库，避免出现空白页
        if key not in META:
            key = "library"
        if self.current == key:
            return
        for k, item in self.nav_items.items():
            item.set_selected(k == key)
        for w in self.body.winfo_children():
            w.destroy()
        self.current = key
        title, sub = META.get(key, (key, ""))
        self.title_lbl.configure(text=title)
        self.sub_lbl.configure(text=sub)
        view = build_view(self, key)
        if view is not None:
            view.pack(fill="both", expand=True)
            self.views[key] = view
        self.refresh_stats()

    # ------------------------------------------------------------ 窗口控制
    def minimize(self):
        try:
            self.root.overrideredirect(False)
            self.root.iconify()
        except Exception:
            pass

    def toggle_maximize(self):
        try:
            if self.is_maximized:
                self.root.geometry(self._normal_geom or "1360x860+80+40")
                self.is_maximized = False
            else:
                self._normal_geom = self.root.geometry()
                # 物理像素：DPI 感知后 winfo_screenwidth 返回的就是物理尺寸
                sw = self.root.winfo_screenwidth()
                sh = self.root.winfo_screenheight()
                self.root.geometry("%dx%d+0+0" % (sw, sh))
                self.is_maximized = True
            self.after(10, self.titlebar._draw)
        except Exception:
            pass

    def on_close(self):
        try:
            from . import scanner
            scanner.reset_pool()
        except Exception:
            pass
        try:
            self.library.close()
        except Exception:
            pass
        try:
            self.root.destroy()
        except Exception:
            pass

    # ------------------------------------------------------------ 状态
    def set_status(self, text, progress=None):
        try:
            self.status_lbl.configure(text=text)
        except Exception:
            pass

    def refresh_stats(self):
        try:
            st = self.library.stats()
            self.stat_lbl.configure(
                text="%d 张照片\n%d 个标签\n%d 张重复" %
                     (st["total"], st["tags"], st["dups"]))
        except Exception:
            pass

    def _add_folder(self):
        from tkinter import filedialog
        d = filedialog.askdirectory(title="选择照片目录")
        if not d:
            return
        self.library.add_root(d)
        self.set_status("已添加目录：" + d + "（点「全流程整理」建立索引）")

    def _run_pipeline(self):
        self.show_view("organize")

    def _on_search(self):
        v = self.views.get(self.current)
        if v is not None and hasattr(v, "apply_search"):
            v.apply_search(self.search_var.get())