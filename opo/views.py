# -*- coding: utf-8 -*-
"""视图实现（一）：照片库、时间轴、标签、事件。

视觉规范全部取自 design.py（源自 DSH），这里只做排列组合。
"""
from __future__ import annotations

import os
import threading
import tkinter as tk
from tkinter import filedialog, messagebox, ttk

from . import design
from .util import human_size, parse_iso
from .widgets import (Button, Card, Chip, Divider, EmptyState, ListRow,
                      Painter, RadioRow, ScrollFrame, SectionTitle, Select)

CARD_W = 208
CARD_IMG_H = 138
GAP = 16


class ThumbCard(tk.Canvas):
    """照片卡片：MangoDisk 的卡片风格。

    默认：bg_layer1 底 + border_l1 细描边
    悬停：底色抬升 + 描边加亮
    选中：蓝色描边 + accent_bg 底（与侧栏选中态同色系）
    """

    def __init__(self, master, theme, row, thumb_path, on_click, on_double, w=CARD_W):
        self.theme = theme
        self._ready = False
        self.row = row
        self.on_click = on_click
        self.on_double = on_double
        self.selected = False
        self._hover = False
        self._img = None
        h = CARD_IMG_H + 52
        super().__init__(master, width=w, height=h, bg=theme["bg_base"],
                         highlightthickness=0, bd=0)
        self._cw, self._ch = w, h
        self.thumb_path = thumb_path
        self._ready = True
        self.configure(cursor="hand2")
        self.bind("<Configure>", lambda e: self._draw())
        self.bind("<Enter>", self._enter)
        self.bind("<Leave>", self._leave)
        self.bind("<Button-1>", self._click)
        self.bind("<Double-Button-1>", self._dbl)
        self._draw()

    def set_thumb(self, img):
        self._img = img
        self._draw()

    def set_selected(self, v):
        self.selected = bool(v)
        self._draw()

    def _enter(self, _=None):
        self._hover = True
        self._draw()

    def _leave(self, _=None):
        self._hover = False
        self._draw()

    def _click(self, _=None):
        if self.on_click:
            self.on_click(self)

    def _dbl(self, _=None):
        if self.on_double:
            self.on_double(self)

    def _draw(self):
        if not self._ready:
            return
        c = self.theme
        self.delete("all")
        w = self.winfo_width() or self._cw
        h = self._ch
        r = design.RADIUS["lg"]

        if self.selected:
            fill, outline, ow = c["accent_bg"], c["accent"], 2
        elif self._hover:
            fill, outline, ow = c["bg_layer2"], c["border_l2"], 1
        else:
            fill, outline, ow = c["bg_layer1"], c["border_l1"], 1
        Painter.rounded_rect(self, 1, 1, w - 2, h - 2, r, fill=fill,
                             outline=outline, width=ow)

        pad = 4
        if self._img is not None:
            self.create_image(pad, pad, image=self._img, anchor="nw")
        else:
            Painter.rounded_rect(self, pad, pad, w - pad, CARD_IMG_H - 2,
                                 r - 3, fill=c["bg_base"], outline="")
            self.create_text(w // 2, CARD_IMG_H // 2, text="◌",
                             fill=c["label_dimmed"],
                             font=(c.f_body.cget("family"), 22))

        # 文字区：按像素宽度截断，绝不换行（换行会向上溢出压到图片）
        ty = CARD_IMG_H + 13
        raw = self.row["name"] or ""
        avail = w - 22
        name = raw
        if c.f_body.measure(name) > avail:
            while name and c.f_body.measure(name + "…") > avail:
                name = name[:-1]
            name = name + "…"
        self.create_text(11, ty, text=name, anchor="w", fill=c["label_primary"],
                         font=c.f_body)
        dt = (self.row["dt"] or "")[:10] or "无日期"
        self.create_text(11, ty + 21, text=dt, anchor="w",
                         fill=c["label_tertiary"], font=c.f_meta)

        # 右上角状态点
        flags = (self.row["flags"] or "")
        dot = None
        if self.row["dup_group"]:
            dot = c["accent"]
        elif "模糊" in flags:
            dot = c["warn"]
        elif (self.row["age_score"] or 0) > 55:
            dot = c["error"]
        if dot:
            x, y, rr = w - 15, 15, 4
            self.create_oval(x - rr, y - rr, x + rr, y + rr, fill=dot, outline="")


class LibraryView(tk.Frame):
    """照片库：筛选栏 + 响应式缩略图网格 + 右侧详情面板。"""

    def __init__(self, app):
        self.app = app
        self.tk_ = app.tk_
        c = self.tk_
        super().__init__(app.body, bg=c["bg_base"])
        self.lib = app.library
        self.cards = []
        self.sel = []
        self._thumb_queue = []
        self._queue_lock = threading.Lock()
        self._worker = None
        self._where = ""
        self._args = ()
        self._limit = 120
        self._build()
        self.reload()
        self._pump()

    # -------------------------------------------------------- 构建
    def _build(self):
        c, tk_ = self.tk_, self.tk_
        # 筛选条
        bar = tk.Frame(self, bg=c["bg_base"])
        bar.pack(fill="x", padx=24, pady=(12, 6))

        self.filters = {}
        self._selects = {}
        specs = [("tag", "标签"), ("year", "年份"), ("flag", "质量标志"),
                 ("dup", "重复"), ("sort", "排序")]
        defaults = {
            "tag": ["全部"], "year": ["全部"], "flag": ["全部"],
            "dup": ["全部", "仅重复", "排除重复"],
            "sort": ["时间 ↑", "时间 ↓", "质量 ↓", "年代感 ↓"],
        }
        for key, label in specs:
            tk.Label(bar, text=label, bg=c["bg_base"], fg=c["label_tertiary"],
                     font=tk_.f_secondary).pack(side="left", padx=(0, 6))
            sel = Select(bar, tk_, defaults[key], width=None,
                         on_change=lambda v: self.reload())
            sel.pack(side="left", padx=(0, 12))
            self._selects[key] = sel
            self.filters[key] = sel

        self.count_lbl = tk.Label(bar, text="", bg=c["bg_base"],
                                  fg=c["label_tertiary"], font=tk_.f_secondary)
        self.count_lbl.pack(side="right")

        # 主体：网格 + 详情
        body = tk.Frame(self, bg=c["bg_base"])
        body.pack(fill="both", expand=True, padx=24, pady=(0, 12))

        self.grid_scroll = ScrollFrame(body, tk_, c["bg_base"])
        self.grid_scroll.pack(side="left", fill="both", expand=True)
        self.grid = self.grid_scroll.inner
        self._last_cols = None
        self.grid_scroll.canvas.bind("<Configure>",
                                     lambda e: self.after(60, self._on_canvas_resize))

        sep = tk.Frame(body, width=1, bg=c["border_l1"], bd=0, highlightthickness=0)
        sep.pack(side="left", fill="y", padx=(14, 0))
        self.detail = DetailPanel(body, tk_)
        self.detail.pack(side="left", fill="y", padx=(6, 0))

    # -------------------------------------------------------- 数据
    def _build_query(self):
        where = ["missing=0"]
        args = []
        tag = self.filters["tag"].get()
        if tag and tag != "全部":
            where.append("id IN (SELECT pt.photo_id FROM photo_tags pt "
                         "JOIN tags t ON t.id=pt.tag_id WHERE t.name=?)")
            args.append(tag)
        year = self.filters["year"].get()
        if year and year != "全部":
            where.append("year=?")
            args.append(int(year))
        flag = self.filters["flag"].get()
        if flag and flag != "全部":
            where.append("flags LIKE ?")
            args.append("%" + flag + "%")
        dup = self.filters["dup"].get()
        if dup == "仅重复":
            where.append("dup_group IS NOT NULL")
        elif dup == "排除重复":
            where.append("dup_group IS NULL")
        q = getattr(self, "_search", "")
        if q:
            where.append("(name LIKE ? OR path LIKE ?)")
            args.extend(["%" + q + "%"] * 2)
        sort = self.filters["sort"].get()
        order = {
            "时间 ↑": "dt IS NULL, dt ASC, id",
            "时间 ↓": "dt IS NULL, dt DESC, id DESC",
            "质量 ↓": "quality_score DESC",
            "年代感 ↓": "age_score DESC",
        }.get(sort, "dt IS NULL, dt ASC, id")
        return " AND ".join(where), tuple(args), order

    def apply_search(self, text):
        self._search = (text or "").strip()
        self.reload()

    def reload(self):
        where, args, order = self._build_query()
        self._where, self._args = where, args
        rows = self.lib.all_photos(where, args, order=order)
        total = len(rows)
        rows = rows[: self._limit]
        pending = self.lib.count(where, args)

        self.count_lbl.configure(text="共 %d 张%s" %
                                 (pending, "（显示前 %d）" % self._limit if total > self._limit else ""))
        self._refresh_filter_values()
        self._render(rows)
        self.detail.show(None)

    def _refresh_filter_values(self):
        try:
            tags = ["全部"] + [n for n, k, cnt in self.lib.all_tags() if cnt > 0][:80]
            years = ["全部"] + [str(y) for y, cnt in self.lib.stats()["years"]]
            flags = ["全部", "模糊", "过暗", "过亮", "低分辨率",
                     "褪色平淡", "泛黄/棕褐", "噪点重", "高光过曝"]
            self._selects["tag"].set_values(tags)
            self._selects["year"].set_values(years)
            self._selects["flag"].set_values(flags)
        except Exception:
            pass

    def _render(self, rows):
        for w in self.grid.winfo_children():
            w.destroy()
        self.cards = []
        self._rows = rows
        if not rows:
            EmptyState(self.grid, self.tk_, "◌", "没有符合条件的照片",
                       "换个筛选条件，或先在「设置」里添加照片目录").pack(
                fill="both", expand=True, pady=60)
            return
        self._layout_cards()

    def _cols(self):
        w = self.grid_scroll.canvas.winfo_width()
        if w <= 1:
            w = self.winfo_width() - DetailPanel.W - 80
        if w <= 1:
            w = 760
        cols = int((w - 4) // (CARD_W + GAP))
        return max(1, min(cols, 8))

    def _card_w(self):
        """卡片宽度按可用宽度均分，避免右侧留大片空白。"""
        w = self.grid_scroll.canvas.winfo_width()
        if w <= 1:
            return CARD_W
        cols = self._cols()
        return max(150, min(240, int((w - cols * GAP) / cols) - 2))

    def _layout_cards(self):
        rows = getattr(self, "_rows", [])
        if not rows:
            return
        cols = self._cols()
        cw = self._card_w()
        for ci in range(max(cols, 12)):
            self.grid.grid_columnconfigure(ci, minsize=0, weight=0)
        for i, row in enumerate(rows):
            if i < len(self.cards):
                card = self.cards[i]
                if getattr(card, "_cw_target", None) != cw:
                    card._cw_target = cw
                    card.configure(width=cw)
            else:
                card = ThumbCard(self.grid, self.tk_, row, row["thumb"],
                                 self._on_select, self._on_open, w=cw)
                card._cw_target = cw
                self.cards.append(card)
                self._enqueue(card, row)
            card.grid(row=i // cols, column=i % cols,
                      padx=GAP // 2, pady=GAP // 2, sticky="nw")
        for ci in range(cols):
            self.grid.grid_columnconfigure(ci, minsize=cw + GAP, weight=0)
        self.grid.update_idletasks()
        self.grid_scroll._on_inner()

    def _on_canvas_resize(self, e=None):
        """窗口变宽/变窄时重排列，不要重建卡片（保留已加载的缩略图）。"""
        if not getattr(self, "_rows", None):
            return
        cols_now = self._cols()
        if cols_now != getattr(self, "_last_cols", None):
            self._last_cols = cols_now
            self._layout_cards()

    # -------------------------------------------------------- 缩略图后台加载
    def _enqueue(self, card, row):
        with self._queue_lock:
            self._thumb_queue.append((card, row))

    def _pump(self):
        """主线程定时消费队列，避免解码阻塞 UI。"""
        batch = []
        with self._queue_lock:
            while self._thumb_queue and len(batch) < 6:
                batch.append(self._thumb_queue.pop(0))
        for card, row in batch:
            try:
                self._load_thumb(card, row)
            except Exception:
                pass
        try:
            self.after(30, self._pump)
        except Exception:
            pass

    def _load_thumb(self, card, row):
        from PIL import Image, ImageTk
        path = row["thumb"]
        if not path or not os.path.exists(path):
            path = row["path"]
        if not path or not os.path.exists(path):
            return
        try:
            im = Image.open(path)
            im.load()
        except Exception:
            return
        # 裁成 16:11 的卡片比例
        tw = CARD_W - 6
        th = CARD_IMG_H - 4
        try:
            im = im.convert("RGB")
            sw, sh = im.size
            scale = max(tw / sw, th / sh)
            nw, nh = max(1, int(sw * scale)), max(1, int(sh * scale))
            im = im.resize((nw, nh), Image.LANCZOS)
            left = (nw - tw) // 2
            top = (nh - th) // 2
            im = im.crop((left, top, left + tw, top + th))
            mask = Image.new("L", (tw, th), 0)
            from PIL import ImageDraw
            ImageDraw.Draw(mask).rounded_rectangle([0, 0, tw - 1, th - 1],
                                                   radius=12, fill=255)
            photo = ImageTk.PhotoImage(im)
            card._photo = photo
            card.set_thumb(photo)
        except Exception:
            pass

    # -------------------------------------------------------- 交互
    def _on_select(self, card):
        multi = bool(self.winfo_toplevel().tk.call("tk::Priv", "shouldTraverse") is None)
        self.sel = [card]
        for x in self.cards:
            x.set_selected(x is card)
        self.detail.show(card.row)

    def _on_open(self, card):
        try:
            os.startfile(card.row["path"])
            self.app.set_status("已用系统程序打开：" + os.path.basename(card.row["path"]))
        except Exception as e:
            self.app.set_status("打开失败：%s" % e)


class DetailPanel(tk.Frame):
    """右侧详情：缩略大图 + 元数据 + 标签。宽度固定，风格同 DSH 的 hovercard。"""

    W = 240

    def __init__(self, master, tk_):
        c = tk_
        super().__init__(master, bg=c["bg_base"], width=self.W)
        self.tk_ = c
        self.pack_propagate(False)
        self.body = tk.Frame(self, bg=c["bg_base"])
        self.body.pack(fill="both", expand=True)
        self._photo = None
        self._show_empty()

    def _clear(self):
        for w in self.body.winfo_children():
            w.destroy()

    def _show_empty(self):
        self._clear()
        c = self.tk_
        tk.Label(self.body, text="详情", bg=c["bg_base"], fg=c["label_tertiary"],
                 font=c.f_meta, anchor="w").pack(fill="x", pady=(0, 14))
        tk.Label(self.body, text="◌", bg=c["bg_base"], fg=c["label_dimmed"],
                 font=(c.f_page_title.cget("family"), 26)).pack(anchor="w", padx=4)
        tk.Label(self.body, text="选中一张照片", bg=c["bg_base"],
                 fg=c["label_secondary"], font=c.f_primary_b,
                 anchor="w").pack(fill="x", padx=2, pady=(10, 4))
        tk.Label(self.body, text="查看时间、尺寸、评分与标签",
                 bg=c["bg_base"], fg=c["label_tertiary"], font=c.f_secondary,
                 anchor="w", wraplength=200, justify="left").pack(fill="x", padx=2)

    def show(self, row):
        self._clear()
        c = self.tk_
        if row is None:
            self._show_empty()
            return
        tk.Label(self.body, text="详情", bg=c["bg_base"], fg=c["label_tertiary"],
                 font=c.f_meta, anchor="w").pack(fill="x", pady=(0, 8))

        # 大图
        cv = tk.Canvas(self.body, width=self.W, height=180, bg=c["bg_layer1"],
                       highlightthickness=0, bd=0)
        cv.pack(fill="x", pady=(0, 10))
        try:
            from PIL import Image, ImageTk
            im = Image.open(row["path"])
            im.load()
            im = im.convert("RGB")
            im.thumbnail((self.W - 4, 180), Image.LANCZOS)
            self._photo = ImageTk.PhotoImage(im)
            cv.create_image(self.W // 2, 90, image=self._photo)
        except Exception:
            cv.create_text(self.W // 2, 90, text="◌", fill=c["label_dimmed"],
                           font=(c.f_body.cget("family"), 24))

        tk.Label(self.body, text=row["name"], bg=c["bg_base"], fg=c["label_primary"],
                 font=c.f_body_b, anchor="w", wraplength=self.W - 4,
                 justify="left").pack(fill="x", pady=(0, 8))

        # 元数据表
        dt = row["dt"] or "未知"
        items = [
            ("拍摄时间", dt),
            ("来源", {"exif": "EXIF", "filename": "文件名", "folder": "目录名",
                     "fs": "文件时间"}.get(row["dt_source"] or "", "-")),
            ("尺寸", "%d × %d" % (row["width"] or 0, row["height"] or 0)),
            ("文件大小", human_size(row["size"])),
            ("质量评分", "%.0f / 100" % (row["quality_score"] or 0)),
            ("年代感", "%.0f / 100" % (row["age_score"] or 0)),
            ("相机", (row["camera_model"] or "-")[:20]),
        ]
        for k, v in items:
            line = tk.Frame(self.body, bg=c["bg_base"])
            line.pack(fill="x", pady=1)
            tk.Label(line, text=k, bg=c["bg_base"], fg=c["label_tertiary"],
                     font=c.f_meta, width=9, anchor="w").pack(side="left")
            tk.Label(line, text=str(v), bg=c["bg_base"], fg=c["label_secondary"],
                     font=c.f_meta, anchor="w",
                     wraplength=self.W - 80, justify="left").pack(side="left")

        # 标签
        tags = None
        try:
            from .library import Library
            tags = self._lib_tags(row)
        except Exception:
            tags = []
        if tags:
            Divider(self.body, c).pack(fill="x", pady=8)
            tk.Label(self.body, text="标签", bg=c["bg_base"], fg=c["label_tertiary"],
                     font=c.f_meta, anchor="w").pack(fill="x", pady=(0, 4))
            wrap = tk.Frame(self.body, bg=c["bg_base"])
            wrap.pack(fill="x")
            for i, t in enumerate(tags[:12]):
                tone = "neutral"
                if t in ("重复",):
                    tone = "info"
                elif t in ("模糊", "过暗", "低分辨率"):
                    tone = "warn"
                elif t.startswith("事件:"):
                    tone = "accent"
                chip = Chip(wrap, c, t[:14], tone)
                chip.pack(side="left", padx=(0, 4), pady=2)

    def _lib_tags(self, row):
        return self.master.master.lib.tags_of(row["id"]) if hasattr(
            self.master.master, "lib") else []


class TimelineView(tk.Frame):
    """时间轴：按年份分组的横向条带，DSH 的 section 分隔风格。"""

    def __init__(self, app):
        self.app = app
        self.tk_ = app.tk_
        c = self.tk_
        super().__init__(app.body, bg=c["bg_base"])
        self.lib = app.library
        self._build()

    def _build(self):
        c, tk_ = self.tk_, self.tk_
        sf = ScrollFrame(self, tk_, c["bg_base"])
        sf.pack(fill="both", expand=True, padx=24, pady=16)
        inner = sf.inner
        years = self.lib.stats()["years"]
        if not years:
            EmptyState(inner, tk_, "◷", "还没有带日期的照片",
                       "在「设置」里添加目录并运行一次整理").pack(pady=60)
            return
        for y, cnt in reversed(years):
            head = tk.Frame(inner, bg=c["bg_base"])
            head.pack(fill="x", pady=(14, 6))
            tk.Label(head, text=str(y), bg=c["bg_base"], fg=c["label_primary"],
                     font=tk_.f_section).pack(side="left")
            tk.Label(head, text="  %d 张" % cnt, bg=c["bg_base"],
                     fg=c["label_tertiary"], font=tk_.f_secondary).pack(side="left")
            rowf = tk.Frame(inner, bg=c["bg_base"])
            rowf.pack(fill="x")
            rows = self.lib.all_photos("year=? AND missing=0", (y,),
                                       order="month, day")[:14]
            for i, r in enumerate(rows):
                card = ThumbCard(rowf, tk_, r, r["thumb"],
                                 lambda cd: None, lambda cd: os.startfile(cd.row["path"]),
                                 w=132)
                card.grid(row=0, column=i, padx=(0, 8), pady=4, sticky="n")
                card._cw = 132
                card.configure(width=132, height=CARD_IMG_H + 40)
                self._thumb(card, r)
            if cnt > 14:
                more = tk.Label(rowf, text="还有 %d 张…" % (cnt - 14),
                                bg=c["bg_base"], fg=c["label_tertiary"],
                                font=tk_.f_secondary)
                more.grid(row=0, column=15, padx=8)
            Divider(inner, tk_).pack(fill="x", pady=(10, 0))

    def _thumb(self, card, row):
        try:
            from PIL import Image, ImageTk
            p = row["thumb"] or row["path"]
            im = Image.open(p)
            im.load()
            im = im.convert("RGB")
            tw, th = 126, CARD_IMG_H - 4
            sw, sh = im.size
            sc = max(tw / sw, th / sh)
            im = im.resize((max(1, int(sw * sc)), max(1, int(sh * sc))), Image.LANCZOS)
            l = (im.width - tw) // 2
            t = (im.height - th) // 2
            im = im.crop((l, t, l + tw, t + th))
            card._photo = ImageTk.PhotoImage(im)
            card.set_thumb(card._photo)
        except Exception:
            pass


class TagsView(tk.Frame):
    """标签：左侧标签列表（按类型分组），右侧该标签下的照片计数。"""

    def __init__(self, app):
        self.app = app
        self.tk_ = app.tk_
        c = self.tk_
        super().__init__(app.body, bg=c["bg_base"])
        self.lib = app.library
        self._build()

    def _build(self):
        c, tk_ = self.tk_, self.tk_
        wrap = tk.Frame(self, bg=c["bg_base"])
        wrap.pack(fill="both", expand=True, padx=24, pady=16)
        sf = ScrollFrame(wrap, tk_, c["bg_base"])
        sf.pack(fill="both", expand=True)
        inner = sf.inner
        groups = {}
        for name, kind, cnt in self.lib.all_tags():
            if cnt <= 0:
                continue
            groups.setdefault(kind, []).append((name, cnt))
        labels = {"time": "时间", "event": "事件", "auto": "自动识别",
                  "user": "自定义"}
        if not groups:
            EmptyState(inner, tk_, "#", "还没有标签",
                       "运行一次整理后会自动生成时间、事件与特征标签").pack(pady=60)
            return
        for kind in ("event", "time", "auto", "user"):
            items = groups.get(kind)
            if not items:
                continue
            head = tk.Frame(inner, bg=c["bg_base"])
            head.pack(fill="x", pady=(16, 8))
            tk.Label(head, text=labels.get(kind, kind), bg=c["bg_base"],
                     fg=c["label_primary"], font=tk_.f_section).pack(side="left")
            tk.Label(head, text="  %d 个" % len(items), bg=c["bg_base"],
                     fg=c["label_tertiary"], font=tk_.f_secondary).pack(side="left")
            grid = tk.Frame(inner, bg=c["bg_base"])
            grid.pack(fill="x")
            for i, (name, cnt) in enumerate(sorted(items, key=lambda x: -x[1])):
                tone = {"event": "accent", "time": "neutral",
                        "auto": "neutral", "user": "info"}.get(kind, "neutral")
                box = tk.Frame(grid, bg=c["bg_base"])
                box.grid(row=i // 5, column=i % 5, sticky="w", padx=(0, 12), pady=3)
                Chip(box, tk_, "%s  %d" % (name[:16], cnt), tone).pack(side="left")
            Divider(inner, tk_).pack(fill="x", pady=(12, 0))


class EventsView(tk.Frame):
    """事件：每个事件一张卡，显示封面、时间范围、张数。"""

    def __init__(self, app):
        self.app = app
        self.tk_ = app.tk_
        c = self.tk_
        super().__init__(app.body, bg=c["bg_base"])
        self.lib = app.library
        self._build()

    def _build(self):
        c, tk_ = self.tk_, self.tk_
        sf = ScrollFrame(self, tk_, c["bg_base"])
        sf.pack(fill="both", expand=True, padx=24, pady=16)
        inner = sf.inner
        evs = self.lib.q("SELECT * FROM events ORDER BY start_dt DESC")
        if not evs:
            EmptyState(inner, tk_, "◈", "还没有识别出事件",
                       "事件由拍摄时间与地点自动聚类，先运行一次整理").pack(pady=60)
            return
        tk.Label(inner, text="共 %d 个事件" % len(evs), bg=c["bg_base"],
                 fg=c["label_tertiary"], font=tk_.f_secondary, anchor="w").pack(
            fill="x", pady=(0, 10))
        gridw = tk.Frame(inner, bg=c["bg_base"])
        gridw.pack(fill="x")
        cols = 4
        for i, e in enumerate(evs):
            card = tk.Frame(gridw, bg=c["bg_layer1"], highlightthickness=1,
                            highlightbackground=c["border_l1"])
            card.grid(row=i // cols, column=i % cols, padx=6, pady=6, sticky="nsew")
            cv = tk.Canvas(card, width=200, height=112, bg=c["bg_layer2"],
                           highlightthickness=0, bd=0)
            cv.pack(fill="x", padx=1, pady=(1, 0))
            cover = self.lib.photo(e["cover_id"]) if e["cover_id"] else None
            if cover:
                try:
                    from PIL import Image, ImageTk
                    im = Image.open(cover["thumb"] or cover["path"])
                    im.load()
                    im = im.convert("RGB")
                    im.thumbnail((198, 110), Image.LANCZOS)
                    ph = ImageTk.PhotoImage(im)
                    cv._ph = ph
                    cv.create_image(100, 56, image=ph)
                except Exception:
                    pass
            box = tk.Frame(card, bg=c["bg_layer1"])
            box.pack(fill="x", padx=10, pady=8)
            tk.Label(box, text=(e["title"] or "")[:22], bg=c["bg_layer1"],
                     fg=c["label_primary"], font=tk_.f_body_b, anchor="w").pack(fill="x")
            span = "%s ~ %s" % ((e["start_dt"] or "")[:10], (e["end_dt"] or "")[:10])
            tk.Label(box, text="%s · %d 张" % (span, e["count"] or 0),
                     bg=c["bg_layer1"], fg=c["label_tertiary"],
                     font=tk_.f_meta, anchor="w").pack(fill="x")
        # 事件卡保留等宽，视觉更整齐

# ====================================================================== 修复
class RestoreView(tk.Frame):
    """老照片修复：左侧选图列表，右侧并排"修复前 / 修复后"。

    布局对照 DSH 的双栏：左列表用 12px 圆角行，右侧是大面积预览区。
    """

    def __init__(self, app):
        self.app = app
        self.tk_ = app.tk_
        c = self.tk_
        super().__init__(app.body, bg=c["bg_base"])
        self.lib = app.library
        self.current_row = None
        self._before = None
        self._after = None
        self._after_img = None
        self._build()

    def _build(self):
        c, tk_ = self.tk_, self.tk_
        # 顶部工具条
        bar = tk.Frame(self, bg=c["bg_base"])
        bar.pack(fill="x", padx=24, pady=(12, 8))
        tk.Label(bar, text="预设", bg=c["bg_base"], fg=c["label_tertiary"],
                 font=tk_.f_meta).pack(side="left", padx=(0, 6))
        from .restore import PRESETS
        self.preset_sel = Select(bar, tk_, list(PRESETS.keys()), "标准修复",
                                 width=132, on_change=lambda v: self.preview())
        self.preset_sel.pack(side="left", padx=(0, 12))

        Button(bar, tk_, "应用修复", self.preview, "primary").pack(side="left", padx=4)
        Button(bar, tk_, "另存为…", self.save_as, "secondary").pack(side="left", padx=4)
        Button(bar, tk_, "批量应用到选中", self.batch_apply, "secondary").pack(side="left", padx=4)
        self.info = tk.Label(bar, text="", bg=c["bg_base"], fg=c["label_tertiary"],
                             font=tk_.f_secondary)
        self.info.pack(side="right")

        # 双栏
        body = tk.Frame(self, bg=c["bg_base"])
        body.pack(fill="both", expand=True, padx=24, pady=(0, 16))

        left = tk.Frame(body, bg=c["bg_base"], width=300)
        left.pack(side="left", fill="y")
        left.pack_propagate(False)
        self.list_scroll = ScrollFrame(left, tk_, c["bg_base"])
        self.list_scroll.pack(fill="both", expand=True)
        self._fill_list()

        Divider(body, tk_, vertical=True).pack(side="left", fill="y", padx=14)

        right = tk.Frame(body, bg=c["bg_base"])
        right.pack(side="left", fill="both", expand=True)
        heads = tk.Frame(right, bg=c["bg_base"])
        heads.pack(fill="x")
        for txt in ("修复前", "修复后"):
            tk.Label(heads, text=txt, bg=c["bg_base"], fg=c["label_tertiary"],
                     font=tk_.f_meta).pack(side="left", expand=True, anchor="w")
        self.cv_before = tk.Canvas(right, bg=c["bg_layer1"], highlightthickness=0, bd=0)
        self.cv_before.pack(side="left", fill="both", expand=True, padx=(0, 6), pady=(4, 0))
        self.cv_after = tk.Canvas(right, bg=c["bg_layer1"], highlightthickness=0, bd=0)
        self.cv_after.pack(side="left", fill="both", expand=True, padx=(6, 0), pady=(4, 0))
        for cv in (self.cv_before, self.cv_after):
            cv.bind("<Configure>", lambda e: self._paint_previews())

    def _fill_list(self):
        c, tk_ = self.tk_, self.tk_
        rows = self.lib.all_photos("analyzed_at IS NOT NULL",
                                   order="age_score DESC")[:80]
        self.pick_rows = rows
        self._rows_widgets = []
        if not rows:
            EmptyState(self.list_scroll.inner, tk_, "✦", "还没有可修复的照片",
                       "先添加目录并运行一次整理").pack(pady=40)
            return
        for r in rows:
            name = (r["name"] or "")[:26]
            sub = (r["dt"] or "")[:10] or "无日期"
            lr = ListRow(self.list_scroll.inner, tk_, name, sub,
                         width=250,
                         command=lambda row=r, w=None: None)
            lr.command = (lambda row=r, lr=lr: self.pick(row, lr))
            lr.pack(fill="x", pady=1, padx=2)
            self._rows_widgets.append(lr)

    def _mark(self, widget, on):
        c = self.tk_
        widget._sel = on
        widget.configure(bg=c["accent_bg"] if on else c["bg_base"],
                         fg=c["accent_bg_fg"] if on else c["label_secondary"])

    def pick(self, row, widget=None):
        for lr in getattr(self, "_rows_widgets", []):
            lr.set_selected(False)
        if widget is not None:
            widget.set_selected(True)
        self.current_row = row
        self._after_img = None
        self.preview()

    def preview(self):
        if not self.current_row:
            return
        row = self.current_row
        try:
            from .imaging import open_image, quality_metrics
            from . import restore
            im = open_image(row["path"])
            if im is None:
                self.info.configure(text="无法解码该文件")
                return
            self._before = im
            m = quality_metrics(im)
            preset = restore.PRESETS.get(self.preset_sel.get(), {})
            self._after_img = restore.auto_enhance(im, m, preset)
            q = quality_metrics(self._after_img)
            self.info.configure(text="对比 %.2f→%.2f · 泛黄 %.2f→%.2f · 细节 +%.0f%%" % (
                m["contrast"], q["contrast"], m["sepia"], q["sepia"],
                max(0, (q["blur"] / max(m["blur"], 1e-6) - 1) * 100)))
            self._paint_previews()
            self.app.set_status("已生成修复预览：" + os.path.basename(row["path"]))
        except Exception as e:
            self.info.configure(text="修复失败：%s" % e)

    def _paint_previews(self):
        c = self.tk_
        for cv, img, tag in ((self.cv_before, self._before, "before"),
                             (self.cv_after, self._after_img, "after")):
            cv.delete("all")
            w = cv.winfo_width() or 300
            h = cv.winfo_height() or 300
            if img is None:
                txt = "← 左侧选一张照片" if tag == "before" else "应用修复后在此预览"
                cv.create_text(w // 2, h // 2, text=txt, fill=c["label_dimmed"],
                               font=c.f_secondary)
                continue
            try:
                from PIL import Image, ImageTk
                disp = img.copy()
                disp.thumbnail((max(40, w - 8), max(40, h - 8)), Image.LANCZOS)
                ph = ImageTk.PhotoImage(disp)
                setattr(cv, "_ph_" + tag, ph)
                cv.create_image(w // 2, h // 2, image=ph)
            except Exception:
                pass

    def save_as(self):
        if self._after_img is None:
            messagebox.showinfo("提示", "请先选择照片并应用修复")
            return
        p = filedialog.asksaveasfilename(defaultextension=".jpg",
                                         filetypes=[("JPEG", "*.jpg"), ("PNG", "*.png")])
        if not p:
            return
        try:
            self._after_img.save(p, quality=95)
            self.app.set_status("已保存：" + p)
        except Exception as e:
            messagebox.showerror("保存失败", str(e))

    def batch_apply(self):
        c = self.tk_
        sel = self.lib.all_photos("analyzed_at IS NOT NULL",
                                   order="age_score DESC")[:20]
        if not sel:
            return
        d = filedialog.askdirectory(title="选择修复结果输出目录")
        if not d:
            return
        from . import restore
        from .imaging import open_image, quality_metrics
        preset = restore.PRESETS.get(self.preset_sel.get(), {})
        done = 0

        def work():
            nonlocal done
            for i, row in enumerate(sel):
                try:
                    im = open_image(row["path"])
                    if im is None:
                        continue
                    m = quality_metrics(im)
                    out = restore.auto_enhance(im, m, preset)
                    name = os.path.splitext(os.path.basename(row["path"]))[0]
                    out.save(os.path.join(d, name + "_修复.jpg"), quality=95)
                    done += 1
                except Exception:
                    continue
                self.app.set_status("批量修复 %d/%d" % (done, len(sel)),
                                    done / len(sel))
            self.app.set_status("批量修复完成：%d 张 → %s" % (done, d))

        self.app.set_status("开始批量修复…", 0)
        threading.Thread(target=work, daemon=True).start()


# ====================================================================== 重复
class DuplicatesView(tk.Frame):
    """重复照片：按组展示，每组标出"最佳"，可一键把其余移到回收目录。"""

    def __init__(self, app):
        self.app = app
        self.tk_ = app.tk_
        c = self.tk_
        super().__init__(app.body, bg=c["bg_base"])
        self.lib = app.library
        self._build()

    def _build(self):
        c, tk_ = self.tk_, self.tk_
        bar = tk.Frame(self, bg=c["bg_base"])
        bar.pack(fill="x", padx=24, pady=(12, 8))
        groups = self.lib.q("SELECT * FROM dup_groups ORDER BY saved_bytes DESC")
        total_saved = sum(g["saved_bytes"] or 0 for g in groups)
        tk.Label(bar, text="%d 组重复，可释放约 %s" %
                 (len(groups), human_size(total_saved)), bg=c["bg_base"],
                 fg=c["label_secondary"], font=tk_.f_body_b).pack(side="left")
        Button(bar, tk_, "清理非最佳副本", self.cleanup, "secondary").pack(side="right")

        sf = ScrollFrame(self, tk_, c["bg_base"])
        sf.pack(fill="both", expand=True, padx=24, pady=(0, 16))
        inner = sf.inner
        if not groups:
            EmptyState(inner, tk_, "⧉", "没有发现重复照片",
                       "去重基于感知哈希，可识别缩放、调亮、裁边后的副本").pack(pady=60)
            return
        for g in groups[:60]:
            head = tk.Frame(inner, bg=c["bg_base"])
            head.pack(fill="x", pady=(12, 6))
            kind = "完全相同" if g["kind"] == "exact" else "相似"
            tk.Label(head, text="组 #%d" % g["id"], bg=c["bg_base"],
                     fg=c["label_primary"], font=tk_.f_body_b).pack(side="left")
            Chip(head, tk_, kind, "info").pack(side="left", padx=8)
            tk.Label(head, text="%d 张 · 可省 %s" %
                     (g["count"] or 0, human_size(g["saved_bytes"] or 0)),
                     bg=c["bg_base"], fg=c["label_tertiary"],
                     font=tk_.f_secondary).pack(side="left")
            rowf = tk.Frame(inner, bg=c["bg_base"])
            rowf.pack(fill="x")
            rows = self.lib.q("SELECT * FROM photos WHERE dup_group=? ORDER BY is_best DESC",
                              (g["id"],))
            for i, r in enumerate(rows):
                card = ThumbCard(rowf, tk_, r, r["thumb"], lambda cd: None,
                                 lambda cd: os.startfile(cd.row["path"]), w=132)
                card.grid(row=0, column=i, padx=(0, 8), pady=4)
                card._cw = 132
                self._thumb(card, r)
                if r["is_best"]:
                    lbl = tk.Label(rowf, text="最佳", bg=c["bg_layer2"],
                                   fg=c["success"], font=tk_.f_meta)
                    lbl.grid(row=1, column=i, sticky="w")
            Divider(inner, tk_).pack(fill="x", pady=(8, 0))

    def _thumb(self, card, row):
        try:
            from PIL import Image, ImageTk
            p = row["thumb"] or row["path"]
            im = Image.open(p)
            im.load()
            im = im.convert("RGB")
            tw, th = 126, CARD_IMG_H - 4
            sw, sh = im.size
            sc = max(tw / sw, th / sh)
            im = im.resize((max(1, int(sw * sc)), max(1, int(sh * sc))), Image.LANCZOS)
            l = (im.width - tw) // 2
            t = (im.height - th) // 2
            im = im.crop((l, t, l + tw, t + th))
            card._photo = ImageTk.PhotoImage(im)
            card.set_thumb(card._photo)
        except Exception:
            pass

    def cleanup(self):
        d = filedialog.askdirectory(title="选择存放非最佳副本的目录")
        if not d:
            return
        n = 0
        rows = self.lib.q("SELECT * FROM photos WHERE dup_group IS NOT NULL AND is_best=0")
        import shutil
        for r in rows:
            try:
                src = r["path"]
                if not os.path.exists(src):
                    continue
                dst = os.path.join(d, os.path.basename(src))
                base, ext = os.path.splitext(dst)
                k = 1
                while os.path.exists(dst):
                    dst = "%s_%d%s" % (base, k, ext)
                    k += 1
                shutil.move(src, dst)
                self.lib.log_op("dedup_move", src, dst, "移动到重复回收目录")
                self.lib.ex("UPDATE photos SET path=?, folder=? WHERE id=?",
                            (dst, os.path.dirname(dst), r["id"]))
                n += 1
            except Exception:
                continue
        messagebox.showinfo("完成", "已移动 %d 个副本到：\n%s" % (n, d))
        self.app.set_status("重复清理完成：%d 个副本 → %s" % (n, d))
        self.app.show_view("library")


# ====================================================================== 质量
class QualityView(tk.Frame):
    """质量待修：按问题类型分组的筛选视图。"""

    FLAGS = ["模糊", "过暗", "过亮", "低分辨率", "褪色平淡", "泛黄/棕褐", "噪点重"]

    def __init__(self, app):
        self.app = app
        self.tk_ = app.tk_
        c = self.tk_
        super().__init__(app.body, bg=c["bg_base"])
        self.lib = app.library
        self.active = "模糊"
        self._build()

    def _build(self):
        c, tk_ = self.tk_, self.tk_
        bar = tk.Frame(self, bg=c["bg_base"])
        bar.pack(fill="x", padx=24, pady=(12, 8))
        self.chips = {}
        for f in self.FLAGS:
            cnt = self.lib.count("flags LIKE ?", ("%" + f + "%",))
            box = tk.Frame(bar, bg=c["bg_base"])
            box.pack(side="left", padx=(0, 6))
            chip = Chip(box, tk_, "%s %d" % (f, cnt),
                        "accent" if f == self.active else "neutral")
            chip.pack()
            chip.configure(cursor="hand2")
            chip.bind("<Button-1>", lambda e, ff=f: self.select(ff))
            self.chips[f] = chip
        self.sf = ScrollFrame(self, tk_, c["bg_base"])
        self.sf.pack(fill="both", expand=True, padx=24, pady=(0, 16))
        self.render()

    def select(self, flag):
        self.active = flag
        for f, chip in self.chips.items():
            chip.tone = "accent" if f == self.active else "neutral"
            chip._draw()
        self.render()

    def render(self):
        c, tk_ = self.tk_, self.tk_
        for w in self.sf.inner.winfo_children():
            w.destroy()
        rows = self.lib.all_photos("flags LIKE ? AND missing=0",
                                   ("%" + self.active + "%",),
                                   order="quality_score ASC")[:60]
        tk.Label(self.sf.inner, text="%s：%d 张" % (self.active, len(rows)),
                 bg=c["bg_base"], fg=c["label_tertiary"],
                 font=tk_.f_secondary, anchor="w").pack(fill="x", pady=(0, 8))
        if not rows:
            EmptyState(self.sf.inner, tk_, "◇", "这个类别下没有照片",
                       "换一个类别看看").pack(pady=40)
            return
        gridw = tk.Frame(self.sf.inner, bg=c["bg_base"])
        gridw.pack(fill="x")
        cols = 5
        for i, r in enumerate(rows):
            card = ThumbCard(gridw, tk_, r, r["thumb"], lambda cd: None,
                             lambda cd: os.startfile(cd.row["path"]))
            card.grid(row=i // cols, column=i % cols, padx=6, pady=6)
            self._thumb(card, r)
        # 不做列均分，保持左对齐自然排版

    def _thumb(self, card, row):
        try:
            from PIL import Image, ImageTk
            p = row["thumb"] or row["path"]
            im = Image.open(p)
            im.load()
            im = im.convert("RGB")
            tw, th = CARD_W - 6, CARD_IMG_H - 4
            sw, sh = im.size
            sc = max(tw / sw, th / sh)
            im = im.resize((max(1, int(sw * sc)), max(1, int(sh * sc))), Image.LANCZOS)
            l = (im.width - tw) // 2
            t = (im.height - th) // 2
            im = im.crop((l, t, l + tw, t + th))
            card._photo = ImageTk.PhotoImage(im)
            card.set_thumb(card._photo)
        except Exception:
            pass


# ====================================================================== 整理
class OrganizeView(tk.Frame):
    """整理归档：方案选择 + 命名模板 + 预览 + 执行 + 回滚。"""

    SCHEME_LABELS = {
        "按年 / 月": "year_month", "按年 / 月 / 日": "year_month_day",
        "按年": "year", "按事件": "event", "按标签": "tag",
        "按质量": "quality", "按是否重复": "dup", "全部平铺": "flat",
    }

    def __init__(self, app):
        self.app = app
        self.tk_ = app.tk_
        c = self.tk_
        super().__init__(app.body, bg=c["bg_base"])
        self.lib = app.library
        self._build()

    def _build(self):
        c, tk_ = self.tk_, self.tk_
        box = tk.Frame(self, bg=c["bg_layer1"], highlightthickness=1,
                       highlightbackground=c["border_l1"])
        box.pack(fill="x", padx=24, pady=(14, 10))

        inner = tk.Frame(box, bg=c["bg_layer1"])
        inner.pack(fill="x", padx=16, pady=14)

        r1 = tk.Frame(inner, bg=c["bg_layer1"])
        r1.pack(fill="x", pady=(0, 10))
        tk.Label(r1, text="分类方案", bg=c["bg_layer1"], fg=c["label_tertiary"],
                 font=tk_.f_meta, width=10, anchor="w").pack(side="left")
        self.scheme_sel = Select(r1, tk_, list(self.SCHEME_LABELS), "按年 / 月",
                                 width=150, on_change=lambda v: self.preview(),
                                 bg_key="bg_layer1")
        self.scheme_sel.pack(side="left", padx=(0, 20))

        tk.Label(r1, text="命名模板", bg=c["bg_layer1"], fg=c["label_tertiary"],
                 font=tk_.f_meta).pack(side="left", padx=(0, 6))
        self.template = tk.StringVar(value="{date}_{name}")
        ent = tk.Entry(r1, textvariable=self.template, width=28, bg=c["bg_base"],
                       fg=c["label_primary"], insertbackground=c["label_primary"],
                       relief="flat", font=tk_.f_secondary, highlightthickness=1,
                       highlightbackground=c["border_l1"],
                       highlightcolor=c["border_l2"])
        ent.pack(side="left", ipady=3)
        tk.Label(r1, text="  可用：{date} {time} {name} {tag} {event} {quality} {ext}",
                 bg=c["bg_layer1"], fg=c["label_dimmed"],
                 font=tk_.f_meta).pack(side="left")

        r2 = tk.Frame(inner, bg=c["bg_layer1"])
        r2.pack(fill="x", pady=(0, 10))
        tk.Label(r2, text="目标目录", bg=c["bg_layer1"], fg=c["label_tertiary"],
                 font=tk_.f_meta, width=10, anchor="w").pack(side="left")
        self.dest = tk.StringVar(value="")
        ent2 = tk.Entry(r2, textvariable=self.dest, width=54, bg=c["bg_base"],
                        fg=c["label_primary"], insertbackground=c["label_primary"],
                        relief="flat", font=tk_.f_secondary, highlightthickness=1,
                        highlightbackground=c["border_l1"],
                        highlightcolor=c["border_l2"])
        ent2.pack(side="left", ipady=3)
        Button(r2, tk_, "浏览…", self.browse, "secondary").pack(side="left", padx=8)

        r3 = tk.Frame(inner, bg=c["bg_layer1"])
        r3.pack(fill="x")
        self.mode = RadioRow(r3, tk_, [("复制（保留原文件）", "copy"),
                                       ("移动（改变原位置）", "move"),
                                       ("硬链接（不占额外空间）", "link")],
                             value="copy", bg=c["bg_layer1"])
        self.mode.pack(side="left")

        act = tk.Frame(self, bg=c["bg_base"])
        act.pack(fill="x", padx=24, pady=(0, 8))
        Button(act, tk_, "预览计划", self.preview, "primary").pack(side="left", padx=(0, 6))
        Button(act, tk_, "执行整理", self.execute, "secondary").pack(side="left", padx=6)
        Button(act, tk_, "回滚最近批次", self.rollback, "secondary").pack(side="left", padx=6)
        self.hint = tk.Label(act, text="", bg=c["bg_base"], fg=c["label_tertiary"],
                             font=tk_.f_secondary)
        self.hint.pack(side="right")

        self.sf = ScrollFrame(self, tk_, c["bg_base"])
        self.sf.pack(fill="both", expand=True, padx=24, pady=(0, 16))

        self.table_holder = tk.Frame(self.sf.inner, bg=c["bg_base"])
        self.table_holder.pack(fill="both", expand=True)
        self.preview()

    def browse(self):
        d = filedialog.askdirectory(title="选择整理输出目录")
        if d:
            self.dest.set(d)

    def _plan(self):
        from .organizer import plan_organize
        scheme = self.SCHEME_LABELS.get(self.scheme_sel.get(), "year_month")
        return plan_organize(self.lib, scheme=scheme,
                             template=self.template.get() or "{date}_{name}")

    def preview(self):
        c, tk_ = self.tk_, self.tk_
        for w in self.table_holder.winfo_children():
            w.destroy()
        try:
            plan = self._plan()
        except Exception as e:
            tk.Label(self.table_holder, text="生成计划失败：%s" % e, bg=c["bg_base"],
                     fg=c["error"], font=tk_.f_secondary).pack(anchor="w")
            return
        self.hint.configure(text="%d 个文件待整理" % len(plan))
        head = tk.Frame(self.table_holder, bg=c["bg_base"])
        head.pack(fill="x", pady=(0, 4))
        tk.Label(head, text="ID", bg=c["bg_base"], fg=c["label_tertiary"],
                 font=tk_.f_meta, width=6, anchor="w").pack(side="left")
        tk.Label(head, text="整理后相对路径", bg=c["bg_base"], fg=c["label_tertiary"],
                 font=tk_.f_meta, anchor="w").pack(side="left")
        Divider(self.table_holder, tk_).pack(fill="x")
        for pid, rel in plan[:200]:
            line = tk.Frame(self.table_holder, bg=c["bg_base"])
            line.pack(fill="x")
            tk.Label(line, text=str(pid), bg=c["bg_base"], fg=c["label_dimmed"],
                     font=tk_.f_mono, width=6, anchor="w").pack(side="left")
            tk.Label(line, text=rel, bg=c["bg_base"], fg=c["label_secondary"],
                     font=tk_.f_mono, anchor="w").pack(side="left")
        if len(plan) > 200:
            tk.Label(self.table_holder, text="… 还有 %d 条" % (len(plan) - 200),
                     bg=c["bg_base"], fg=c["label_tertiary"],
                     font=tk_.f_meta).pack(anchor="w", pady=4)

    def execute(self):
        dest = self.dest.get().strip()
        if not dest:
            messagebox.showinfo("提示", "请先选择目标目录")
            return
        from .organizer import execute_organize
        try:
            plan = self._plan()
        except Exception as e:
            messagebox.showerror("失败", str(e))
            return
        if not messagebox.askyesno("确认整理",
                                   "将要处理 %d 个文件到：\n%s\n\n模式：%s" %
                                   (len(plan), dest, self.mode.get())):
            return
        try:
            r = execute_organize(self.lib, dest, plan, mode=self.mode.get())
        except Exception as e:
            messagebox.showerror("执行失败", str(e))
            return
        self.app.set_status("整理完成：%d 张，跳过 %d，错误 %d（批次 %s）" %
                            (r["done"], r["skipped"], r["errors"], r["batch"]))
        messagebox.showinfo("完成", "已完成 %d 张\n批次号：%s\n可用「回滚最近批次」撤销" %
                            (r["done"], r["batch"]))
        self.app.refresh_stats()

    def rollback(self):
        from .organizer import rollback
        if not messagebox.askyesno("确认回滚", "将撤销最近一次整理操作，是否继续？"):
            return
        r = rollback(self.lib)
        messagebox.showinfo("回滚完成", "恢复 %d 个文件，错误 %d" %
                            (r["restored"], r["errors"]))
        self.app.set_status("回滚完成：恢复 %d" % r["restored"])
        self.app.refresh_stats()
        self.preview()


# ====================================================================== 设置
class SettingsView(tk.Frame):
    """设置：目录管理与阈值调整，分区块，风格同 DSH 设置页。"""

    FIELDS = [
        ("blur_threshold", "清晰度阈值", "低于此值判为模糊（会按对比度自适应下调）"),
        ("dark_threshold", "过暗阈值", "平均亮度低于此值判为过暗"),
        ("bright_threshold", "过亮阈值", "平均亮度高于此值判为过亮"),
        ("lowres_mp", "低分辨率阈值", "低于此百万像素判为低分辨率"),
        ("faded_contrast", "褪色对比度阈值", "对比度低于此值判为褪色平淡"),
        ("event_gap_hours", "事件间隔（小时）", "相邻照片超过此间隔则分到新事件"),
        ("phash_distance", "去重哈希距离", "感知哈希汉明距离上限"),
    ]

    def __init__(self, app):
        self.app = app
        self.tk_ = app.tk_
        c = self.tk_
        super().__init__(app.body, bg=c["bg_base"])
        from . import config
        self.cfg = config.load_settings()
        self._build()

    def _build(self):
        c, tk_ = self.tk_, self.tk_
        sf = ScrollFrame(self, tk_, c["bg_base"])
        sf.pack(fill="both", expand=True, padx=24, pady=16)
        inner = sf.inner

        # ---- 照片目录
        tk.Label(inner, text="照片目录", bg=c["bg_base"], fg=c["label_primary"],
                 font=tk_.f_section, anchor="w").pack(fill="x")
        tk.Label(inner, text="添加后运行一次全流程整理，即可建立索引",
                 bg=c["bg_base"], fg=c["label_tertiary"],
                 font=tk_.f_secondary, anchor="w").pack(fill="x", pady=(2, 10))
        self.roots_box = tk.Frame(inner, bg=c["bg_base"])
        self.roots_box.pack(fill="x")
        self._fill_roots()
        Button(inner, tk_, "添加目录…", self.add_root, "secondary").pack(
            anchor="w", pady=10)

        Divider(inner, tk_).pack(fill="x", pady=16)

        # ---- 识别阈值
        tk.Label(inner, text="识别阈值", bg=c["bg_base"], fg=c["label_primary"],
                 font=tk_.f_section, anchor="w").pack(fill="x")
        tk.Label(inner, text="调整这些数值可以改变自动分类的松紧程度",
                 bg=c["bg_base"], fg=c["label_tertiary"],
                 font=tk_.f_secondary, anchor="w").pack(fill="x", pady=(2, 12))
        self.vars = {}
        grid = tk.Frame(inner, bg=c["bg_base"])
        grid.pack(fill="x")
        for i, (key, label, hint) in enumerate(self.FIELDS):
            row = tk.Frame(grid, bg=c["bg_base"])
            row.pack(fill="x", pady=3)
            tk.Label(row, text=label, bg=c["bg_base"], fg=c["label_secondary"],
                     font=tk_.f_body, width=16, anchor="w").pack(side="left")
            var = tk.StringVar(value=str(self.cfg.get(key, "")))
            self.vars[key] = var
            # 用 1px 描边的容器包住 Entry，模拟 MangoDisk 的 input 样式
            box = tk.Frame(row, bg=c["bg_layer1"], highlightthickness=1,
                           highlightbackground=c["border_l1"],
                           highlightcolor=c["accent"])
            box.pack(side="left", padx=(0, 12))
            ent = tk.Entry(box, textvariable=var, width=9, bg=c["bg_layer1"],
                           fg=c["label_primary"], insertbackground=c["accent"],
                           relief="flat", bd=0, font=tk_.f_mono,
                           highlightthickness=0)
            ent.pack(padx=8, pady=4)
            tk.Label(row, text=hint, bg=c["bg_base"], fg=c["label_tertiary"],
                     font=tk_.f_secondary).pack(side="left")

        Button(inner, tk_, "保存设置", self.save, "primary").pack(anchor="w", pady=14)

        Divider(inner, tk_).pack(fill="x", pady=16)

        # ---- 重新分析
        tk.Label(inner, text="维护", bg=c["bg_base"], fg=c["label_primary"],
                 font=tk_.f_section, anchor="w").pack(fill="x")
        tk.Label(inner, text="重新分析会清空已有指标并重建缩略图",
                 bg=c["bg_base"], fg=c["label_tertiary"],
                 font=tk_.f_secondary, anchor="w").pack(fill="x", pady=(2, 10))
        row = tk.Frame(inner, bg=c["bg_base"])
        row.pack(fill="x")
        Button(row, tk_, "重新分析全部", self.reanalyze, "secondary").pack(side="left")
        self.msg = tk.Label(row, text="", bg=c["bg_base"], fg=c["label_tertiary"],
                            font=tk_.f_secondary)
        self.msg.pack(side="left", padx=12)

    def _fill_roots(self):
        c, tk_ = self.tk_, self.tk_
        for w in self.roots_box.winfo_children():
            w.destroy()
        roots = self.app.library.roots()
        if not roots:
            tk.Label(self.roots_box, text="（还没有添加任何目录）", bg=c["bg_base"],
                     fg=c["label_dimmed"], font=tk_.f_secondary, anchor="w").pack(fill="x")
            return
        for r in roots:
            row = tk.Frame(self.roots_box, bg=c["bg_layer1"])
            row.pack(fill="x", pady=2)
            tk.Label(row, text=r, bg=c["bg_layer1"], fg=c["label_secondary"],
                     font=tk_.f_mono, anchor="w", padx=10, pady=6).pack(
                side="left", fill="x", expand=True)
            Button(row, tk_, "移除", lambda p=r: self.remove_root(p),
                   "ghost").pack(side="right", padx=6)

    def add_root(self):
        d = filedialog.askdirectory(title="选择照片目录")
        if not d:
            return
        self.app.library.add_root(d)
        self._fill_roots()
        self.msg.configure(text="已添加，可点「全流程整理」建立索引")

    def remove_root(self, path):
        self.app.library.remove_root(path)
        self._fill_roots()

    def save(self):
        from . import config
        for key, var in self.vars.items():
            try:
                old = self.cfg.get(key)
                if isinstance(old, int):
                    self.cfg[key] = int(float(var.get()))
                else:
                    self.cfg[key] = float(var.get())
            except Exception:
                pass
        config.save_settings(self.cfg)
        self.msg.configure(text="已保存到 %s" % config.app_dir())
        self.app.set_status("设置已保存")

    def reanalyze(self):
        if not messagebox.askyesno("确认", "将清空现有分析结果并重新分析全部照片，可能耗时较长。"):
            return
        from . import pipeline
        self.app.set_status("正在重新分析…", 0)
        prog = pipeline.Progress()

        def work():
            pipeline.run_full(self.app.library, self.cfg, prog=prog, do_scan=False,
                              do_analyze=True, do_tag=True, do_events=True,
                              do_dup=True, redo=True)
            self.app.set_status("重新分析完成")

        threading.Thread(target=work, daemon=True).start()


# ====================================================================== 工厂
def build_view(app, key):
    mapping = {
        "library": LibraryView,
        "timeline": TimelineView,
        "tags": TagsView,
        "events": EventsView,
        "duplicates": DuplicatesView,
        "quality": QualityView,
        "restore": RestoreView,
        "organize": OrganizeView,
        "settings": SettingsView,
    }
    cls = mapping.get(key)
    return cls(app) if cls else None