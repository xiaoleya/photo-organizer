# -*- coding: utf-8 -*-
"""自绘控件库：规格逐条对齐 MangoDisk 源码。

对照来源：
    md-sidebar.vue          nav-item 40px/8px圆角/14px字/24px图标/3px选中条
    button-variants.ts      h-10 px-4 rounded-md text-sm font-medium
    main.css                --layout-* 密度变量
    mango.css               sidebar / card / accent 等语义色
"""
from __future__ import annotations

import tkinter as tk

from . import design, glyphs
from .fonts import Tokens


class Painter:
    @staticmethod
    def rounded_rect(cv, x1, y1, x2, y2, r, **kw):
        r = max(0, min(r, (x2 - x1) // 2, (y2 - y1) // 2))
        if r <= 0:
            return cv.create_rectangle(x1, y1, x2, y2, **kw)
        pts = [x1 + r, y1, x2 - r, y1, x2, y1, x2, y1 + r,
               x2, y2 - r, x2, y2, x2 - r, y2, x1 + r, y2,
               x1, y2, x1, y2 - r, x1, y1 + r, x1, y1]
        return cv.create_polygon(pts, smooth=True, **kw)


class NavItem(tk.Canvas):
    """侧栏导航项，逐条照 md-sidebar.vue 的 .nav-item：

        height: 40px          (--layout-sidebar-item-height)
        border-radius: 8px
        padding: 0 14px       展开后 0 12px，gap 12px
        font-size: 14px
        .nav-icon: 24x24
        .active: bg sidebar-accent + font-weight 600
        .active::before: left 0, 3x24px, radius 999px, bg primary
        :hover:not(.active): color-mix(sidebar-accent 52%, transparent)
    """

    def __init__(self, master, theme, icon, label, command=None, width=None,
                 expanded=True):
        self.theme = theme
        self._ready = False
        self.expanded = expanded
        w = width or design.SIDEBAR_W
        h = design.NAV_ITEM_H
        super().__init__(master, width=w, height=h, bg=theme["sidebar"],
                         highlightthickness=0, bd=0)
        self.command = command
        self.icon = icon
        self.label = label
        self.selected = False
        self._hover = False
        self._cw, self._ch = w, h
        self._ready = True
        self.configure(cursor="hand2")
        self.bind("<Configure>", lambda e: self._draw())
        self.bind("<Enter>", self._enter)
        self.bind("<Leave>", self._leave)
        self.bind("<Button-1>", lambda e: self.command() if self.command else None)
        self._draw()

    def _enter(self, _=None):
        self._hover = True
        self._draw()

    def _leave(self, _=None):
        self._hover = False
        self._draw()

    def set_selected(self, v):
        self.selected = bool(v)
        self._draw()

    def _draw(self):
        if not self._ready:
            return
        c = self.theme
        self.delete("all")
        w = self.winfo_width() or self._cw
        h = self.winfo_height() or self._ch
        x1, y1, x2, y2 = 8, 0, w - 8, h

        if self.selected:
            Painter.rounded_rect(self, x1, y1, x2, y2, design.RADIUS["md"],
                                 fill=c["sidebar_accent"], outline="")
            # .active::before —— 3x24 胶囊，贴左边缘
            bar_h = 24
            by = (h - bar_h) // 2
            Painter.rounded_rect(self, x1 - 8, by, x1 - 8 + 3, by + bar_h,
                                 design.RADIUS["pill"],
                                 fill=c["accent"], outline="")
            ink = c["sidebar_accent_fg"]
            fnt = c.f_nav_b
        elif self._hover:
            hover_bg = design.mix(c["sidebar_accent"], c["sidebar"], 0.52)
            Painter.rounded_rect(self, x1, y1, x2, y2, design.RADIUS["md"],
                                 fill=hover_bg, outline="")
            ink = c["label_primary"]
            fnt = c.f_nav
        else:
            ink = c["label_secondary"]
            fnt = c.f_nav

        cy = h // 2
        ico = design.NAV_ICON
        pad = 14 if self.expanded else ((w - ico) // 2)
        if self.expanded:
            glyphs.draw(self, self.icon, pad + ico // 2, cy, ink, size=ico)
            self.create_text(pad + ico + 12, cy, text=self.label,
                             anchor="w", fill=ink, font=fnt)
        else:
            glyphs.draw(self, self.icon, w // 2, cy, ink, size=ico)


class Button(tk.Canvas):
    """按钮，按 button-variants.ts：

        default h-10 px-4 rounded-md text-sm font-medium gap-2
        primary  : bg-primary text-primary-foreground
        outline  : border-input bg-background, hover:bg-accent
        ghost    : hover:bg-accent
        destructive: bg-destructive
    """

    HEIGHT = design.BTN_H

    def __init__(self, master, theme, text, command=None, variant="outline",
                 width=None, height=None, icon=""):
        self.theme = theme
        self._ready = False
        self.text = text
        self.icon = icon
        self.command = command
        self.variant = variant
        self._hover = False
        self._down = False
        self._enabled = True
        f = theme.f_primary
        label = text
        tw = f.measure(label)
        icon_w = 26 if icon else 0
        h = height or self.HEIGHT
        w = width or max(80, tw + icon_w + 28)
        try:
            pbg = master["bg"]
        except Exception:
            pbg = theme["bg_base"]
        super().__init__(master, width=w, height=h, bg=pbg,
                         highlightthickness=0, bd=0)
        self._cw, self._ch = w, h
        self._ready = True
        self.configure(cursor="hand2")
        self.bind("<Configure>", lambda e: self._draw())
        self.bind("<Enter>", lambda e: self._hover_set(True))
        self.bind("<Leave>", lambda e: self._hover_set(False))
        self.bind("<Button-1>", self._press)
        self.bind("<ButtonRelease-1>", self._release)
        self._draw()

    def _hover_set(self, v):
        self._hover = v
        self._draw()

    def _press(self, _=None):
        if self._enabled:
            self._down = True
            self._draw()

    def _release(self, _=None):
        if not self._enabled:
            return
        was = self._down
        self._down = False
        self._draw()
        if was and self.command:
            self.command()

    def _colors(self):
        c = self.theme
        if not self._enabled:
            return c["bg_layer2"], c["label_dimmed"], ""
        v = self.variant
        if v == "primary":
            if self._down:
                return c["accent_press"], c["accent_text"], ""
            if self._hover:
                return c["accent_hover"], c["accent_text"], ""
            return c["accent"], c["accent_text"], ""
        if v == "danger":
            return (c["error"] if self._hover else c["bg_layer2"]), \
                   (c["label_inverted"] if self._hover else c["error"]), \
                   ("" if self._hover else c["border_l2"])
        if v == "ghost":
            return (c["accent_bg"] if self._hover else c["bg_base"]), \
                   (c["accent_bg_fg"] if self._hover else c["label_secondary"]), ""
        if self._hover:
            return c["accent_bg"], c["accent_bg_fg"], c["accent"]
        return c["bg_base"], c["label_primary"], c["border_l2"]

    def _draw(self):
        if not self._ready:
            return
        self.delete("all")
        w = self.winfo_width() or self._cw
        h = self.winfo_height() or self._ch
        bg, fg, outline = self._colors()
        Painter.rounded_rect(self, 0, 0, w - 1, h - 1, design.RADIUS["md"],
                             fill=bg, outline=outline or bg,
                             width=1 if outline else 0)
        f = self.theme.f_primary
        tw = f.measure(self.text)
        total = tw + (26 if self.icon else 0)
        x0 = (w - total) // 2
        if self.icon:
            glyphs.draw(self, self.icon, x0 + 9, h // 2, fg, size=16)
            x0 += 26
        self.create_text(x0, h // 2, text=self.text, anchor="w", fill=fg, font=f)


class Select(tk.Canvas):
    """下拉选择：对齐 MangoDisk 的 select（工作区控件高 30px）。"""

    def __init__(self, master, theme, values, value=None, width=None,
                 on_change=None, height=None, bg_key="bg_base"):
        self.theme = theme
        self._ready = False
        self.values = list(values) if values else ["全部"]
        self.value = value if value is not None else self.values[0]
        self.on_change = on_change
        self._hover = False
        self._open = False
        self._bg_key = bg_key
        self._font = theme.f_primary
        w = width or (max(self._font.measure(str(v)) for v in self.values) + 54)
        h = height or design.BTN_H
        super().__init__(master, width=w, height=h, bg=theme[bg_key],
                         highlightthickness=0, bd=0)
        self._cw, self._ch = w, h
        self._popup = None
        self._ready = True
        self.configure(cursor="hand2")
        self.bind("<Configure>", lambda e: self._draw())
        self.bind("<Enter>", lambda e: self._set(True))
        self.bind("<Leave>", lambda e: self._set(False))
        self.bind("<Button-1>", lambda e: self._toggle())
        self._draw()

    def get(self):
        return self.value

    def set(self, v):
        if v in self.values:
            self.value = v
            self._draw()

    def set_values(self, vals, keep=True):
        self.values = list(vals) if vals else ["全部"]
        if self.value not in self.values:
            self.value = self.values[0]
        self._draw()

    def _set(self, v):
        self._hover = v
        self._draw()

    def _draw(self):
        if not self._ready:
            return
        c = self.theme
        self.delete("all")
        w = self.winfo_width() or self._cw
        h = self._ch
        bg = c["bg_layer2"] if (self._hover or self._open) else c["bg_layer1"]
        outline = c["accent"] if self._open else (
            c["border_l2"] if self._hover else c["border_l1"])
        Painter.rounded_rect(self, 0, 0, w - 1, h - 1, design.RADIUS["md"],
                             fill=bg, outline=outline, width=1)
        label = self.theme.elide(str(self.value), self._font, w - 44)
        self.create_text(12, h // 2, text=label, anchor="w",
                         fill=c["label_primary"], font=self._font)
        glyphs.draw(self, "chevron_down", w - 16, h // 2, c["label_tertiary"],
                    size=16, width=2.0)

    def _toggle(self):
        if self._open:
            self._close()
            return
        self._open = True
        self._draw()
        self._popup = tk.Toplevel(self)
        self._popup.overrideredirect(True)
        self._popup.configure(bg=self.theme["border_l1"])
        x = self.winfo_rootx()
        y = self.winfo_rooty() + self._ch + 3
        w = max(self.winfo_width() or self._cw, 150)
        rows = min(len(self.values), 12)
        self._popup.geometry("%dx%d+%d+%d" % (w, rows * 32 + 6, x, y))
        try:
            self._popup.attributes("-topmost", True)
        except Exception:
            pass
        holder = tk.Frame(self._popup, bg=self.theme["bg_layer2"])
        holder.pack(fill="both", expand=True, padx=1, pady=1)
        for v in self.values[:12]:
            item = tk.Label(holder, text=str(v), bg=self.theme["bg_layer2"],
                            fg=self.theme["label_primary"],
                            font=self.theme.f_primary, anchor="w",
                            padx=12, height=1, cursor="hand2")
            item.pack(fill="x", pady=1, padx=2)
            item.bind("<Enter>", lambda e, w_=item: w_.configure(
                bg=self.theme["accent_bg"], fg=self.theme["accent_bg_fg"]))
            item.bind("<Leave>", lambda e, w_=item: w_.configure(
                bg=self.theme["bg_layer2"], fg=self.theme["label_primary"]))
            item.bind("<Button-1>", lambda e, vv=v: self._choose(vv))
        self.after(60, lambda: self._popup and self._popup.focus_set())

    def _choose(self, v):
        self.value = v
        self._close()
        if self.on_change:
            self.on_change(v)

    def _close(self):
        self._open = False
        if self._popup is not None:
            try:
                self._popup.destroy()
            except Exception:
                pass
            self._popup = None
        self._draw()


class Chip(tk.Canvas):
    def __init__(self, master, theme, text, tone="neutral", on_close=None):
        self.theme = theme
        self._ready = False
        self.text = text
        self.tone = tone
        self.on_close = on_close
        f = theme.f_secondary
        w = f.measure(text) + (36 if on_close else 28)
        h = 26
        super().__init__(master, width=w, height=h, bg=theme["bg_base"],
                         highlightthickness=0, bd=0)
        self._cw, self._ch = w, h
        self._ready = True
        self.bind("<Configure>", lambda e: self._draw())
        self._draw()
        if on_close:
            self.configure(cursor="hand2")
            self.bind("<Button-1>", lambda e: self.on_close(self))

    def _tone_colors(self):
        c = self.theme
        return {
            "neutral": (c["bg_layer2"], c["label_secondary"]),
            "accent": (c["accent_bg"], c["accent_bg_fg"]),
            "error": (c["bg_layer2"], c["error"]),
            "success": (c["bg_layer2"], c["success"]),
            "warn": (c["bg_layer2"], c["warn"]),
            "info": (c["bg_layer2"], c["info"]),
        }.get(self.tone, (c["bg_layer2"], c["label_secondary"]))

    def _draw(self):
        if not self._ready:
            return
        self.delete("all")
        w = self.winfo_width() or self._cw
        bg, fg = self._tone_colors()
        Painter.rounded_rect(self, 0, 0, w - 1, self._ch - 1, design.RADIUS["sm"],
                             fill=bg, outline="")
        self.create_text(w // 2, self._ch // 2, text=self.text, fill=fg,
                         font=self.theme.f_secondary)


class ListRow(tk.Canvas):
    """列表行：--layout-result-row-height 44px。"""

    H = design.ROW_H

    def __init__(self, master, theme, text, sub="", command=None, width=260):
        self.theme = theme
        self._ready = False
        super().__init__(master, width=width, height=self.H,
                         bg=theme["bg_base"], highlightthickness=0, bd=0)
        self.text = text
        self.sub = sub
        self.command = command
        self.selected = False
        self._hover = False
        self._cw = width
        self._ready = True
        self.configure(cursor="hand2")
        self.bind("<Configure>", lambda e: self._draw())
        self.bind("<Enter>", lambda e: self._set(True))
        self.bind("<Leave>", lambda e: self._set(False))
        self.bind("<Button-1>", lambda e: self.command() if self.command else None)
        self._draw()

    def _set(self, v):
        self._hover = v
        self._draw()

    def set_selected(self, v):
        self.selected = bool(v)
        self._draw()

    def _draw(self):
        if not self._ready:
            return
        c = self.theme
        self.delete("all")
        w = self.winfo_width() or self._cw
        h = self.H
        if self.selected:
            fill, fg = c["accent_bg"], c["accent_bg_fg"]
        elif self._hover:
            fill, fg = c["hover"], c["label_primary"]
        else:
            fill, fg = None, c["label_secondary"]
        if fill:
            Painter.rounded_rect(self, 4, 2, w - 4, h - 2, design.RADIUS["md"],
                                 fill=fill, outline="")
        ty = h // 2 - (7 if self.sub else 0)
        name = self.theme.elide(self.text, self.theme.f_body, w - 28)
        self.create_text(14, ty, text=name, anchor="w", fill=fg,
                         font=self.theme.f_body)
        if self.sub:
            self.create_text(14, ty + 16, text=self.sub, anchor="w",
                             fill=c["label_tertiary"], font=self.theme.f_meta)


class RadioRow(tk.Frame):
    """单选组：18px 圆点 + 13px 文字。"""

    def __init__(self, master, theme, options, value=None, on_change=None, bg=None):
        bg = bg or theme["bg_layer1"]
        super().__init__(master, bg=bg, bd=0, highlightthickness=0)
        self.theme = theme
        self.options = list(options)
        self.value = value or (self.options[0][1] if self.options else None)
        self.on_change = on_change
        self._dots = {}
        for label, val in self.options:
            row = tk.Frame(self, bg=bg)
            row.pack(side="left", padx=(0, 20))
            cv = tk.Canvas(row, width=18, height=18, bg=bg,
                           highlightthickness=0, bd=0)
            cv.pack(side="left", padx=(0, 8))
            lbl = tk.Label(row, text=label, bg=bg, fg=theme["label_secondary"],
                           font=theme.f_body, cursor="hand2")
            lbl.pack(side="left")
            self._dots[val] = cv
            for w_ in (row, cv, lbl):
                w_.configure(cursor="hand2")
                w_.bind("<Button-1>", lambda e, v=val: self.select(v))
        self._draw_all()

    def get(self):
        return self.value

    def set(self, v):
        self.value = v
        self._draw_all()

    def select(self, v):
        self.value = v
        self._draw_all()
        if self.on_change:
            self.on_change(v)

    def _draw_all(self):
        c = self.theme
        for val, cv in self._dots.items():
            cv.delete("all")
            on = (val == self.value)
            cv.create_oval(1, 1, 17, 17, outline=c["accent"] if on else c["border_l2"],
                           width=2 if on else 1, fill="")
            if on:
                cv.create_oval(6, 6, 12, 12, fill=c["accent"], outline="")


class Divider(tk.Frame):
    def __init__(self, master, theme, vertical=False):
        kw = dict(bg=theme["border_l1"], bd=0, highlightthickness=0)
        if vertical:
            super().__init__(master, width=1, **kw)
        else:
            super().__init__(master, height=1, **kw)


class _ThinScrollbar(tk.Canvas):
    """极细滚动条：10px 轨道内 4px 圆角滑条（--layout-scrollbar-width）。"""

    W = design.SCROLLBAR

    def __init__(self, master, theme, command, bg):
        super().__init__(master, width=self.W, bg=bg, highlightthickness=0,
                         bd=0, takefocus=0)
        self.theme = theme
        self.command = command
        self._first = 0.0
        self._last = 1.0
        self._hover = False
        self._drag_y = None
        self._drag_first = 0.0
        self.bind("<Enter>", lambda e: self._hv(True))
        self.bind("<Leave>", lambda e: self._hv(False))
        self.bind("<Configure>", lambda e: self._draw())
        self.bind("<Button-1>", self._press)
        self.bind("<B1-Motion>", self._drag)
        self.bind("<ButtonRelease-1>", lambda e: setattr(self, "_drag_y", None))

    def _hv(self, v):
        self._hover = v
        self._draw()

    def set(self, first, last):
        try:
            self._first, self._last = float(first), float(last)
        except Exception:
            return
        self._draw()

    def _box(self):
        h = self.winfo_height() or 1
        span = max(0.04, self._last - self._first)
        top = self._first * h
        bh = max(36, span * h)
        if top + bh > h:
            top = h - bh
        return top, top + bh

    def _draw(self):
        self.delete("all")
        h = self.winfo_height()
        if h < 2 or (self._first <= 0.0 and self._last >= 1.0):
            return
        color = self.theme["border_l3"] if self._hover else self.theme["border_l2"]
        t, b = self._box()
        cx = self.W // 2
        Painter.rounded_rect(self, cx - 2, t, cx + 2, b, 2, fill=color, outline="")

    def _press(self, e):
        t, b = self._box()
        if t <= e.y <= b:
            self._drag_y = e.y
            self._drag_first = self._first
        else:
            self.command("moveto", max(0.0, min(1.0, e.y / max(1, self.winfo_height()))))

    def _drag(self, e):
        if self._drag_y is None:
            return
        h = max(1, self.winfo_height())
        self.command("moveto",
                     max(0.0, min(1.0, self._drag_first + (e.y - self._drag_y) / h)))


class ScrollFrame(tk.Frame):
    """滚动容器。canvas 先建并 pack，inner 后建，最后 create_window。"""

    def __init__(self, master, theme, bg=None):
        bg = bg or theme["bg_base"]
        super().__init__(master, bg=bg, bd=0, highlightthickness=0)
        self.theme = theme
        self._bg = bg
        self.canvas = tk.Canvas(self, bg=bg, bd=0, highlightthickness=0,
                                takefocus=0, relief="flat")
        self.vsb = _ThinScrollbar(self, theme, self.canvas.yview, bg)
        self.canvas.configure(yscrollcommand=self.vsb.set)
        self.vsb.pack(side="right", fill="y")
        self.canvas.pack(side="left", fill="both", expand=True)
        self.inner = tk.Frame(self, bg=bg, bd=0, highlightthickness=0)
        self._win = self.canvas.create_window((0, 0), window=self.inner,
                                              anchor="nw", tags="inner")
        self.inner.bind("<Configure>", self._on_inner)
        self.canvas.bind("<Configure>", self._on_canvas)
        self.canvas.bind("<Enter>", lambda e: self._wheel(True))
        self.canvas.bind("<Leave>", lambda e: self._wheel(False))

    def _wheel(self, on):
        if on:
            self.canvas.bind_all("<MouseWheel>", self._on_wheel)
        else:
            self.canvas.unbind_all("<MouseWheel>")

    def _on_wheel(self, e):
        try:
            self.canvas.yview_scroll(int(-1 * (e.delta / 120)), "units")
        except Exception:
            pass

    def _on_inner(self, _=None):
        try:
            self.canvas.configure(scrollregion=self.canvas.bbox("all"))
        except Exception:
            pass

    def _on_canvas(self, e):
        try:
            self.canvas.itemconfigure("inner", width=e.width)
        except Exception:
            pass

    def scroll_top(self):
        try:
            self.canvas.yview_moveto(0.0)
        except Exception:
            pass


class EmptyState(tk.Frame):
    def __init__(self, master, theme, icon="library", title="暂无内容", hint="", action=None):
        bg = theme["bg_base"]
        super().__init__(master, bg=bg)
        box = tk.Frame(self, bg=bg)
        box.place(relx=0.5, rely=0.42, anchor="center")
        cv = tk.Canvas(box, width=48, height=48, bg=bg, highlightthickness=0, bd=0)
        cv.pack()
        glyphs.draw(cv, icon, 24, 24, theme["label_dimmed"], size=40, width=1.4)
        tk.Label(box, text=title, bg=bg, fg=theme["label_secondary"],
                 font=theme.f_empty_title).pack(pady=(14, 6))
        if hint:
            tk.Label(box, text=hint, bg=bg, fg=theme["label_tertiary"],
                     font=theme.f_body, wraplength=360,
                     justify="center").pack()
        if action:
            txt, cmd = action
            Button(box, theme, txt, cmd, variant="primary").pack(pady=(18, 0))


def token_bundle(root):
    return Tokens(root, "dark")


class Card(tk.Frame):
    """内容卡：1px 描边 + bg-layer-1 底（MangoDisk 的 card）。"""

    def __init__(self, master, theme, pad=16, bg_key="bg_layer1"):
        self.theme = theme
        wrap = tk.Frame(master, bg=theme["border_l1"])
        self._wrap = wrap
        super().__init__(wrap, bg=theme[bg_key], bd=0, highlightthickness=0)
        tk.Frame.pack(self, fill="both", expand=True, padx=1, pady=1)
        self.pad = pad
        self.inner = tk.Frame(self, bg=theme[bg_key])
        self.inner.pack(fill="both", expand=True, padx=pad, pady=pad)

    def pack(self, **kw):
        self._wrap.pack(**kw)
        return self

    def grid(self, **kw):
        self._wrap.grid(**kw)
        return self


class SectionTitle(tk.Frame):
    """区块标题：15px 粗 + 可选的说明行。"""

    def __init__(self, master, theme, text, sub=""):
        try:
            bg = master["bg"]
        except Exception:
            bg = theme["bg_base"]
        super().__init__(master, bg=bg)
        tk.Label(self, text=text, bg=bg, fg=theme["label_primary"],
                 font=theme.f_section, anchor="w").pack(anchor="w")
        if sub:
            tk.Label(self, text=sub, bg=bg, fg=theme["label_tertiary"],
                     font=theme.f_secondary, anchor="w").pack(anchor="w",
                                                              pady=(2, 0))