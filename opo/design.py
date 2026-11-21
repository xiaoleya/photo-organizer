# -*- coding: utf-8 -*-
"""设计令牌：直接移植 MangoDisk 的设计系统，强调色橙→蓝。

数据来源：github.com/harry0703/MangoDisk
    src/assets/themes/base-tokens.css      字体层级 / 圆角基线
    src/assets/themes/mango.css            深浅两套 oklch 调色板
    src/assets/main.css                    布局密度（关键！）
    src/layouts/components/md-sidebar.vue  侧栏条目规格
    src/layouts/components/md-window-titlebar.vue  自绘标题栏

=== 布局密度（main.css 原值，之前一直用错了）===
    --layout-sidebar-collapsed-width: 68px
    --layout-sidebar-expanded-width:  240px
    --layout-sidebar-item-height:     40px      <-- 之前用 34，偏挤
    --layout-page-padding-inline:     20px
    --layout-page-padding-top:        14px
    --layout-page-header-height:      58px
    --layout-result-row-height:       44px
    --layout-action-bar-height:       48px
    --layout-scrollbar-width:         10px

=== 侧栏条目（md-sidebar.vue 原值）===
    height: 40px;  border-radius: 8px;  padding: 0 14px
    font-size: 14px;  icon 24x24
    active: bg-sidebar-accent + font-weight 600
    active::before: left 0, width 3px, height 24px, border-radius 999px, bg primary
    hover: color-mix(sidebar-accent 52%, transparent)

=== 排版（base-tokens.css）===
    empty-title 1.25rem(20px)  section-title .9375rem(15px)
    primary .8125rem(13px)     body .75rem(12px)
    secondary .6875rem(11px)   meta .625rem(10px)
侧栏/正文主字号用 14px 与 13px，比原先 9-10pt 明显更大更清晰。

=== 橙 → 蓝换算（OKLCH 保持 L/C，色相 54.1° → 255°）===
    --brand-mango #FD8204 -> #4EA9FF      --ring #FD8722 -> #4EA9FF
    --accent #513A27 -> #2F4157           --accent-fg #FBCA93 -> #ACD8FF
    --sidebar-accent #52361E -> #293E5A   --sidebar-accent-fg #FECE96 -> #B0DBFF
中性灰原带暖黄（H=70），与蓝冲突，去黄为蓝灰（H=250），明度不变。
"""
from __future__ import annotations

# ---------------------------------------------------------------- 品牌（橙→蓝）
ACCENT = {
    "brand": "#4EA9FF",
    "ring": "#4EA9FF",
    "text": "#89C7FF",
    "hover": "#6CBAFF",
    "bg": "#2F4157",
    "bg_fg": "#ACD8FF",
    "sidebar_bg": "#293E5A",
    "sidebar_fg": "#B0DBFF",
}

# ---------------------------------------------------------------- 深色
DARK = {
    "sidebar": "#161A1E",
    "bg_base": "#1B1E22",
    "bg_workspace": "#212428",
    "bg_layer1": "#292E34",
    "bg_layer2": "#31363B",
    "bg_overlay": "#30363C",
    "bg_secondary": "#353D45",

    "border_l1": "#42484F",
    "border_l2": "#4F565E",
    "border_l3": "#5E666F",
    "input": "#7A848F",

    "label_primary": "#E6ECF2",
    "label_secondary": "#AFB9C3",
    "label_tertiary": "#8A939E",
    "label_dimmed": "#6B747D",
    "label_inverted": "#1B1E22",

    "hover": "#373E45",
    "active": "#3D454D",
    "selected": ACCENT["bg"],

    "error": "#F25C61",
    "success": "#61D591",
    "warn": "#FFBE5C",
    "info": "#75B4FF",

    "accent": ACCENT["brand"],
    "accent_hover": ACCENT["hover"],
    "accent_press": "#3C8FE0",
    "accent_text": "#0E1720",
    "accent_bg": ACCENT["bg"],
    "accent_bg_fg": ACCENT["bg_fg"],
    "sidebar_accent": ACCENT["sidebar_bg"],
    "sidebar_accent_fg": ACCENT["sidebar_fg"],

    "scroll_thumb": "#42484F",
    "scroll_hover": "#5E666F",
}

# ---------------------------------------------------------------- 浅色
LIGHT = {
    "sidebar": "#F3F4F6",
    "bg_base": "#FAFBFC",
    "bg_workspace": "#FFFFFF",
    "bg_layer1": "#FFFFFF",
    "bg_layer2": "#F0F2F5",
    "bg_overlay": "#FFFFFF",
    "bg_secondary": "#E8EBEF",

    "border_l1": "#E1E5EA",
    "border_l2": "#CFD5DC",
    "border_l3": "#B9C1CA",
    "input": "#9AA4AF",

    "label_primary": "#1C2126",
    "label_secondary": "#5A6470",
    "label_tertiary": "#7A838E",
    "label_dimmed": "#9AA2AC",
    "label_inverted": "#FFFFFF",

    "hover": "#EDF0F4",
    "active": "#E3E8EE",
    "selected": "#D7E6F8",

    "error": "#C4262E",
    "success": "#1F7A44",
    "warn": "#A64D00",
    "info": "#1B63BE",

    "accent": "#1B63BE",
    "accent_hover": "#1550A0",
    "accent_press": "#124587",
    "accent_text": "#FFFFFF",
    "accent_bg": "#D7E6F8",
    "accent_bg_fg": "#1B4F8A",
    "sidebar_accent": "#DCE6F5",
    "sidebar_accent_fg": "#15458A",

    "scroll_thumb": "#CFD5DC",
    "scroll_hover": "#B9C1CA",
}

# ---------------------------------------------------------------- 字号（px，按 MangoDisk 原值）
FONT_PX = {
    "meta": 10,
    "secondary": 11,
    "body": 12,
    "primary": 13,
    "nav": 14,
    "section": 15,
    "empty_title": 20,
    "page_title": 22,
}

# Tk 在 Windows 上以 pt 计字号；Windows 的 96dpi 下 1pt = 1.333px
# 但 Tk 的字体 size 传正数就是 pt，为了视觉上等于设计稿的 px，
# 这里直接按 px 数值给出 pt 的近似：px * 0.75，再向上取到清晰档位。
# 字号按 MangoDisk 的 px 值 × 1.15 适度放大（用户反馈偏小），
# 在 Tk 里用 pt 表达，配合 DPI 感知后视觉尺寸等于设计稿。
FONT_SIZE = {
    "meta": 8,          # 10px
    "secondary": 8,     # 11px
    "body": 9,          # 12px
    "primary": 9,       # 13px
    "nav": 10,          # 14px
    "section": 10,      # 15px
    "empty_title": 13,  # 20px
    "page_title": 14,   # 22px
}

# 清晰的字体栈：优先 Segoe UI Variable / 雅黑 UI，避免细体发虚
FONT_STACK = ("Segoe UI Variable Text", "Segoe UI", "Microsoft YaHei UI",
              "Microsoft YaHei", "PingFang SC", "Arial")
FONT_DISPLAY = ("Segoe UI Variable Display", "Segoe UI", "Microsoft YaHei UI",
                "Microsoft YaHei", "Arial")
FONT_MONO = ("Cascadia Mono", "Consolas", "Microsoft YaHei UI")

# ---------------------------------------------------------------- 圆角
RADIUS = {"xs": 4, "sm": 6, "md": 8, "lg": 12, "xl": 16, "panel": 20, "pill": 999}

# ---------------------------------------------------------------- 布局密度（main.css 原值）
SIDEBAR_W = 240
SIDEBAR_COLLAPSED = 68
NAV_ITEM_H = 40
NAV_ICON = 22
PAGE_PAD_X = 22
PAGE_PAD_TOP = 16
PAGE_HEADER_H = 62
TITLEBAR_H = 44
ROW_H = 46
BTN_H = 32
SCROLLBAR = 10

SPACE = {"xxs": 2, "xs": 4, "sm": 6, "md": 10, "lg": 14, "xl": 20, "xxl": 28}


def palette(theme="dark"):
    return LIGHT if str(theme).lower() in ("light", "浅色") else DARK


def hex_to_rgb(h):
    h = h.lstrip("#")
    if len(h) == 8:
        h = h[2:]
    return tuple(int(h[i:i + 2], 16) for i in (0, 2, 4))


def blend(fg, bg, alpha):
    fr, fg_, fb = hex_to_rgb(fg)
    br, bg_, bb = hex_to_rgb(bg)
    return "#%02X%02X%02X" % (int(fr * alpha + br * (1 - alpha)),
                              int(fg_ * alpha + bg_ * (1 - alpha)),
                              int(fb * alpha + bb * (1 - alpha)))


def mix(a, b, t):
    """等价于 CSS color-mix(in oklab, a t%, b)。"""
    ra, ga, ba = hex_to_rgb(a)
    rb, gb, bb = hex_to_rgb(b)
    return "#%02X%02X%02X" % (int(ra + (rb - ra) * t),
                              int(ga + (gb - ga) * t),
                              int(ba + (bb - ba) * t))


def surface(c):
    out = {}
    base = c["bg_base"]
    for k, v in c.items():
        if isinstance(v, str) and len(v) == 9 and v.startswith("#"):
            a = int(v[1:3], 16) / 255.0
            out[k] = blend("#" + v[3:], base, a)
        else:
            out[k] = v
    return out