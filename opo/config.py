# -*- coding: utf-8 -*-
"""路径与默认参数。"""
from __future__ import annotations

import json
import os

APP_DIR_NAME = "OldPhotoOrganizer"


def ensure_dir(d: str) -> str:
    """线程安全地确保目录存在。

    os.makedirs(exist_ok=True) 在多线程同时创建同一目录时仍可能抛
    FileExistsError（检查与创建之间有竞态），这里统一吞掉并复核。
    """
    try:
        os.makedirs(d, exist_ok=True)
    except FileExistsError:
        pass
    except OSError:
        if not os.path.isdir(d):
            raise
    return d


_DIR_CACHE = {}


def _cached_dir(name: str) -> str:
    p = _DIR_CACHE.get(name)
    if p and os.path.isdir(p):
        return p
    p = ensure_dir(os.path.join(app_dir(), name))
    _DIR_CACHE[name] = p
    return p


def app_dir() -> str:
    base = os.environ.get("OPO_HOME")
    if base:
        return ensure_dir(base)
    base = os.environ.get("LOCALAPPDATA") or os.path.expanduser("~")
    return ensure_dir(os.path.join(base, APP_DIR_NAME))


def db_path() -> str:
    return os.path.join(app_dir(), "library.db")


def thumb_dir() -> str:
    return _cached_dir("thumbs")


def log_dir() -> str:
    return _cached_dir("logs")


DEFAULT_SETTINGS = {
    # 质量自适应阈值（可在设置里改）
    "blur_threshold": 90.0,        # 640 宽灰度图上 Laplacian 方差，小于此值算模糊
    "dark_threshold": 0.16,        # 平均亮度（0-1）
    "bright_threshold": 0.86,      # 平均亮度过高
    "white_threshold": 0.30,       # 高光截断比例
    "lowres_mp": 0.70,             # 低于此百万像素算低分辨率
    "faded_contrast": 0.135,       # 对比度低于此值算平淡/褪色
    "faded_saturation": 0.150,     # 饱和度低于此值算褪色
    "sepia_threshold": 0.45,       # 棕褐化程度
    "skin_threshold": 0.055,       # 肤色像素占比，超过算"可能有人物"
    # 事件聚类
    "event_gap_hours": 6.0,        # 相邻照片时间间隔超过此值则分到新事件
    "event_gps_km": 50.0,          # 地点跳变距离阈值
    # 去重
    "phash_distance": 14,          # 感知哈希最大汉明距离（实测：同图<=6，不同图>=26）
    "dhash_distance": 8,
    "dedup_size_ratio": 6.0,       # 长/短边缩放倍率容忍（1280->200 也能配对）
    # 缩略图
    "thumb_size": 320,
    "thumb_quality": 82,
    "analyze_workers": 0,          # 0 = 自动
    "filename_template": "{date}_{time}_{name}",
    # ---- 实验室功能（仅这两项需要开关，默认关闭） ----
    "lab_quality": False,          # 质量待修（模糊/过暗/褪色等判定）
    "lab_restore": False,          # 照片修复
    # 以下功能已转正常驻，保留键位仅用于兼容旧配置
    "lab_duplicates": True,
    "lab_organize": True,
    "lab_favorites": True,
    "lab_places": True,
}


def load_settings(path=None) -> dict:
    s = dict(DEFAULT_SETTINGS)
    p = path or os.path.join(app_dir(), "settings.json")
    try:
        with open(p, "r", encoding="utf-8") as f:
            s.update(json.load(f) or {})
    except Exception:
        pass
    return s


def save_settings(s: dict, path=None) -> None:
    p = path or os.path.join(app_dir(), "settings.json")
    try:
        with open(p, "w", encoding="utf-8") as f:
            json.dump(s, f, ensure_ascii=False, indent=2)
    except Exception:
        pass
