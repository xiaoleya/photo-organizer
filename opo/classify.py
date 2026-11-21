# -*- coding: utf-8 -*-
"""自动分类：规则打标 + 事件聚类 + 重复检测。"""
from __future__ import annotations

import datetime as dt
import math
import os
import re
from collections import defaultdict

from . import config
from .util import haversine_km, iso, parse_iso

# --------------------------------------------------------------- 关键词规则
# 面向中文老照片的常见命名习惯；命中即打标签。
KEYWORD_RULES = [
    ("证件照", r"证件|身份证|护照|驾照|一寸|两寸|小一寸|签证照|照片回执"),
    ("毕业", r"毕业|学士服|授位|典礼|聚餐.*毕业"),
    ("婚礼", r"婚礼|结婚|婚纱|喜宴|订婚|伴娘|伴郎|婚庆"),
    ("全家福", r"全家福|全家|合影|家族|亲人|家里人"),
    ("合影", r"合影|合照|集体照|留念|纪念照|排排坐|毕业照"),
    ("婴儿", r"婴儿|满月|百天|周岁|宝宝|新生儿|襁褓"),
    ("儿童", r"儿童|小孩|小朋友|童年|幼儿|孩子"),
    ("旅行", r"旅游|旅行|出游|自驾|春游|秋游|游|风景|景区|爬山|登山|海边|沙滩"),
    ("风景", r"风景|风光|山水|日落|日出|夕阳|云海|雪景|夜景|山|湖|海"),
    ("工作", r"工作|办公|单位|会议|车间|工厂|工地|厂区|公司|同事"),
    ("学校", r"学校|校园|教室|同学|老师|操场|宿舍|校门|班级"),
    ("军营", r"部队|军营|战友|军装|军训|入伍|退伍"),
    ("节日", r"春节|过年|元宵|清明|端午|中秋|国庆|元旦|除夕|圣诞|年三十|拜年"),
    ("生日", r"生日|蛋糕|寿宴|祝寿|寿星"),
    ("聚会", r"聚会|聚餐|饭局|宴|酒席|团年|同学会|喝茶"),
    ("老屋", r"老屋|老家|老宅|院子|祖屋|农村|乡下|村|祠堂"),
    ("城市", r"城市|街景|街道|大楼|广场|车站|火车站|机场|天桥"),
    ("车辆", r"汽车|卡车|摩托|自行车|拖拉机|火车|公交"),
    ("宠物", r"狗|猫|宠物|小狗|猫咪|鸟|鸡|鸭|鹅"),
    ("文档", r"文件|文档|扫描|复印|合同|票据|发票|笔记|手稿|信|证明"),
    ("墓碑/祭祀", r"墓|碑|坟|祭祀|清明|扫墓"),
    ("黑白底片", r"底片|负片|胶片|菲林|黑白片"),
]

# 相机直出/扫描命名，通常没有语义，归到"未命名"
JUNK_NAME = re.compile(r"^(img|dsc|dscn|pxl|p|gopr|_mg|dji|mvimg|wx_camera|mmexport|screenshot|"
                       r"img_\d|scan|photo|pic|dsc_\d|100media|照片|图片|图像)[-_ ]?\d*$", re.I)


def _photo_dt(row):
    return parse_iso(row["dt"])


def auto_tag(lib, cfg: dict, progress=None, cancel=None) -> dict:
    """基于文件名、目录、EXIF、质量与颜色特征批量打标签。"""
    lib.clear_auto_tags()
    rows = lib.all_photos()
    pairs = []
    for i, r in enumerate(rows):
        if cancel is not None and cancel.is_set():
            break
        hay = "%s %s" % (r["name"] or "", r["folder"] or "")
        for tag, pat in KEYWORD_RULES:
            if re.search(pat, hay, re.I):
                pairs.append((r["id"], tag))
        # 质量类标签
        flags = (r["flags"] or "").split("|")
        for f in flags:
            if f:
                pairs.append((r["id"], f))
        if r["is_mono"]:
            pairs.append((r["id"], "黑白"))
        if r["is_sepia"] and not r["is_mono"]:
            pairs.append((r["id"], "棕褐老照片"))
        if r["has_people"]:
            pairs.append((r["id"], "有人物"))
        if (r["age_score"] or 0) >= 55:
            pairs.append((r["id"], "年代感强"))
        if r["dt_source"] == "fs":
            pairs.append((r["id"], "时间待确认"))
        if not r["dt"]:
            pairs.append((r["id"], "缺拍摄时间"))
        # 相机品牌
        mk = (r["camera_make"] or "").strip()
        if mk:
            mk = re.sub(r"\s+", " ", mk)[:24]
            pairs.append((r["id"], "相机:" + mk))
        # 尺寸特征
        w = r["width"] or 0
        h = r["height"] or 0
        if w and h:
            if w < h:
                pairs.append((r["id"], "竖幅"))
            elif w > h:
                pairs.append((r["id"], "横幅"))
            if max(w, h) >= 3000:
                pairs.append((r["id"], "高清大图"))
        # 扫描件判据：超宽或超高，白色背景多
        if w and h and max(w, h) >= 2.2 * max(1, min(w, h)):
            pairs.append((r["id"], "疑似扫描件"))
        if progress and i % 200 == 0:
            progress("tag", i, len(rows), os.path.basename(r["name"] or ""))
    lib.add_tags_bulk(pairs, kind="auto", source="auto")
    return {"photos": len(rows), "tags": len(pairs)}


# --------------------------------------------------------------- 事件聚类

def cluster_events(lib, cfg: dict, rebuild=True) -> dict:
    """按时间间隔 + 地点跳变切分事件，写 events 表并加 '事件:xxx' 标签。"""
    rows = lib.q("SELECT id,dt,lat,lon,path,quality_score,thumb FROM photos "
                 "WHERE dt IS NOT NULL ORDER BY dt")
    gap = float(cfg.get("event_gap_hours", 6.0)) * 3600.0
    gps_km = float(cfg.get("event_gps_km", 50.0))
    events = []
    cur = []
    last = None
    for r in rows:
        d = _photo_dt(r)
        if d is None:
            continue
        split = False
        if last is not None:
            if (d - last).total_seconds() > gap:
                split = True
            elif (r["lat"] is not None and last_lat is not None
                  and haversine_km(r["lat"], r["lon"], last_lat, last_lon) > gps_km):
                split = True
        if split and cur:
            events.append(cur)
            cur = []
        cur.append(r)
        last = d
        last_lat, last_lon = r["lat"], r["lon"]
    if cur:
        events.append(cur)

    # 合并过碎的事件（少于 3 张且与前后同日）
    merged = []
    for ev in events:
        if merged and len(ev) < 3 and len(merged[-1]) < 3:
            if _same_day(_photo_dt(merged[-1][-1]), _photo_dt(ev[0])):
                merged[-1].extend(ev)
                continue
        merged.append(ev)

    if rebuild:
        lib.ex("DELETE FROM events")
        lib.ex("DELETE FROM photo_tags WHERE source IN ('event','time')")
        lib.ex("DELETE FROM tags WHERE kind IN ('event','time') "
               "AND id NOT IN (SELECT tag_id FROM photo_tags)")
    pairs = []
    n = 0
    for ev in merged:
        if not ev:
            continue
        start = _photo_dt(ev[0])
        end = _photo_dt(ev[-1])
        best = max(ev, key=lambda r: (r["quality_score"] or 0))
        title = _event_title(start, end, ev)
        cur2 = lib.ex("INSERT INTO events(title,start_dt,end_dt,lat,lon,count,cover_id) "
                      "VALUES(?,?,?,?,?,?,?)",
                      (title, iso(start), iso(end), ev[0]["lat"], ev[0]["lon"],
                       len(ev), best["id"]))
        eid = cur2.lastrowid
        for r in ev:
            pairs.append((r["id"], "事件:" + title))
        n += 1
    lib.add_tags_bulk(pairs, kind="event", source="event")

    # 时间维度标签：年 / 年月 / 年代
    tpairs = []
    for r in lib.q("SELECT id,year,month FROM photos WHERE year IS NOT NULL"):
        tpairs.append((r["id"], "%d年" % r["year"]))
        tpairs.append((r["id"], "%d年%02d月" % (r["year"], r["month"] or 1)))
        dec = (r["year"] // 10) * 10
        tpairs.append((r["id"], "%d年代" % dec))
    lib.add_tags_bulk(tpairs, kind="time", source="time")
    return {"events": n, "photos": len(rows)}


def _same_day(a, b):
    return a and b and a.date() == b.date()


def backfill_places(lib, progress=None, redo=False) -> dict:
    """给照片补算地点。

    redo=False：只补空值（快速，日常用）
    redo=True ：全部重算（换了识别算法后用，会把旧的经纬度名改成城市名）
    """
    from . import geo

    if redo:
        rows = lib.q("SELECT id,path,lat,lon FROM photos")
    else:
        rows = lib.q("SELECT id,path,lat,lon FROM photos "
                     "WHERE place IS NULL OR place=''")
    total = len(rows)
    updates = []
    for i, r in enumerate(rows):
        place = geo.resolve_place(r["lat"], r["lon"], r["path"] or "")
        updates.append((place, r["id"]))
        if progress and i % 200 == 0:
            progress("地点识别", i, total, os.path.basename(r["path"] or ""))
    if updates:
        lib.conn.executemany("UPDATE photos SET place=? WHERE id=?", updates)
        lib.conn.commit()
    if progress:
        progress("地点识别", total, max(total, 1), "完成 %d 张" % total)
    return {"total": total, "updated": len(updates), "redo": redo}


def _event_title(start, end, ev):
    if start is None:
        return "未标日期"
    d0, d1 = start.date(), (end or start).date()
    seasons = {1: "冬", 2: "冬", 3: "春", 4: "春", 5: "春", 6: "夏",
               7: "夏", 8: "夏", 9: "秋", 10: "秋", 11: "秋", 12: "冬"}
    if d0 == d1:
        core = "%d-%02d-%02d" % (d0.year, d0.month, d0.day)
    elif d0.year == d1.year and d0.month == d1.month:
        core = "%d-%02d-%02d~%02d" % (d0.year, d0.month, d0.day, d1.day)
    elif d0.year == d1.year:
        core = "%d-%02d-%02d ~ %02d-%02d" % (d0.year, d0.month, d0.day, d1.month, d1.day)
    else:
        core = "%d-%02d-%02d ~ %d-%02d-%02d" % (d0.year, d0.month, d0.day,
                                                d1.year, d1.month, d1.day)
    return "%s %s" % (core, seasons.get(start.month, ""))


# --------------------------------------------------------------- 重复检测

class _UF:
    def __init__(self, n):
        self.p = list(range(n))

    def find(self, a):
        while self.p[a] != a:
            self.p[a] = self.p[self.p[a]]
            a = self.p[a]
        return a

    def union(self, a, b):
        ra, rb = self.find(a), self.find(b)
        if ra != rb:
            self.p[rb] = ra


def find_duplicates(lib, cfg: dict, progress=None, cancel=None) -> dict:
    """两步去重：
    1) 完全相同：sha1 相同
    2) 近似重复：感知哈希汉明距离 + 尺寸比例约束，用并查集合并
    结果写入 dup_groups，并给每张图打 '重复' 标签、组内最优 is_best=1。
    """
    lib.ex("UPDATE photos SET dup_group=NULL, is_best=0")
    lib.ex("DELETE FROM dup_groups")
    lib.ex("DELETE FROM photo_tags WHERE source='dup'")

    rows = lib.q("SELECT id,sha1,p_hash,d_hash,a_hash,width,height,size,megapixels,"
                 "quality_score,blur,path FROM photos WHERE p_hash IS NOT NULL AND missing=0")
    n = len(rows)
    if n == 0:
        return {"groups": 0, "photos": 0}

    # ---- 1. 完全重复
    by_sha = defaultdict(list)
    for r in rows:
        if r["sha1"]:
            by_sha[r["sha1"]].append(r["id"])
    exact = {k: v for k, v in by_sha.items() if len(v) > 1}

    # ---- 2. 近似重复：按 pHash 前 16 位分桶，桶内逐对比较
    maxd = int(cfg.get("phash_distance", 10))
    maxd2 = int(cfg.get("dhash_distance", 8))
    ratio = float(cfg.get("dedup_size_ratio", 3.0))
    uf = _UF(n)
    idx = {r["id"]: i for i, r in enumerate(rows)}
    ph = [int(r["p_hash"], 16) for r in rows]
    dh = [int(r["d_hash"], 16) if r["d_hash"] else None for r in rows]
    ah = [int(r["a_hash"], 16) if r["a_hash"] else None for r in rows]

    # 多段 band 索引（LSH）：64 位哈希切成 4 段 16 位，
    # 汉明距离 <= 10 的一对以极大概率共享至少一段，召回远高于单段前缀。
    BANDS = 4
    band_bits = 64 // BANDS
    band_mask = (1 << band_bits) - 1
    buckets = defaultdict(list)
    for i in range(n):
        v = ph[i]
        for b in range(BANDS):
            buckets[("p", b, (v >> (b * band_bits)) & band_mask)].append(i)
        if ah[i] is not None:
            av = ah[i]
            for b in range(2):
                buckets[("a", b, (av >> (b * 32)) & 0xFFFFFFFF)].append(i)

    seen = set()
    comps = 0
    MAX_BUCKET = 400          # 单桶过大说明图太重复（连拍），限制两两比较量
    cand_pairs = 0
    for key, ids in buckets.items():
        if len(ids) < 2:
            continue
        if len(ids) > MAX_BUCKET:
            ids = ids[:MAX_BUCKET]
        for a in range(len(ids)):
            i = ids[a]
            for b in range(a + 1, len(ids)):
                j = ids[b]
                if i == j:
                    continue
                pair = (i, j) if i < j else (j, i)
                if pair in seen:
                    continue
                seen.add(pair)
                comps += 1
                ra, rb = rows[i], rows[j]
                if not _size_compatible(ra, rb, ratio):
                    continue
                dp = _ham(ph[i], ph[j])
                dd = _ham(dh[i], dh[j]) if (dh[i] is not None and dh[j] is not None) else 999
                da = _ham(ah[i], ah[j]) if (ah[i] is not None and ah[j] is not None) else 999

                # 尺寸比：同一张图的缩放不会差太多
                ra2, rb2 = rows[i], rows[j]
                la = max(ra2["width"] or 1, ra2["height"] or 1)
                lb = max(rb2["width"] or 1, rb2["height"] or 1)
                size_ratio = max(la, lb) / float(max(1, min(la, lb)))

                same = False
                # 三级判据（截图类低对比图需要更严）
                if dp <= 3 and dd <= 10:
                    same = True                                  # 几乎完全一致
                elif dp <= 6 and dd <= 6 and da <= 6:
                    same = True                                  # 三元同时接近
                elif dp <= 8 and dd <= 4 and da <= 3 and size_ratio <= 1.6:
                    same = True                                  # 严格兜底

                if uf.find(i) != uf.find(j):
                    uf.union(i, j)
                    cand_pairs += 1
        if progress and comps and comps % 100000 < 200:
            progress("dedup", comps, 0, "已比较 %d 对" % comps)
        if cancel is not None and cancel.is_set():
            break

    groups = defaultdict(list)
    for i in range(n):
        groups[uf.find(i)].append(rows[i])

    # 把完全相同但哈希没连上的也并进去
    for sha, ids in exact.items():
        roots = {uf.find(idx[i]) for i in ids if i in idx}
        if len(roots) > 1:
            roots = list(roots)
            for r in roots[1:]:
                uf.union(roots[0], r)
    groups = defaultdict(list)
    for i in range(n):
        groups[uf.find(i)].append(rows[i])

    # 并查集有传递性缺陷：A≈B、B≈C 会被连成一组，即使 A 和 C 差很远。
    # 这里对每个组做一致性校验：以组内密度最高的点为锚，
    # 只保留与锚点距离在阈值内的成员，其余拆出去独立成组（或丢弃）。
    def _pixel_mse(path_a, path_b, size=64):
        """缩略图灰度 MSE：哈希可能撞车，但真实像素不会。
        实测参考值：真近似 ~90-200；不同截图 ~1000+。"""
        try:
            from PIL import Image as _Img
            import numpy as _np
            ia = _Img.open(path_a).convert("L")
            ib = _Img.open(path_b).convert("L")
            ia = ia.resize((size, size), _Img.LANCZOS)
            ib = ib.resize((size, size), _Img.LANCZOS)
            a = _np.asarray(ia, dtype=_np.float32)
            b = _np.asarray(ib, dtype=_np.float32)
            return float(_np.mean((a - b) ** 2))
        except Exception:
            return None

    def _split_by_anchor(members, max_dp=6, max_dd=8, max_mse=600.0):
        if len(members) <= 1:
            return [members]
        # 选中心点：与其它成员平均距离最小的那个
        best_i, best_score = 0, 1e9
        for i, a in enumerate(members):
            pa, da_ = int(a["p_hash"], 16), int(a["d_hash"], 16) if a["d_hash"] else None
            tot = 0
            for j, b in enumerate(members):
                if i == j:
                    continue
                pb = int(b["p_hash"], 16)
                tot += _ham(pa, pb)
            avg = tot / max(1, len(members) - 1)
            if avg < best_score:
                best_score, best_i = avg, i
        anchor = members[best_i]
        pa = int(anchor["p_hash"], 16)
        da_ = int(anchor["d_hash"], 16) if anchor["d_hash"] else None
        keep, rest = [], []
        for m in members:
            pb = int(m["p_hash"], 16)
            dp = _ham(pa, pb)
            dd = _ham(da_, int(m["d_hash"], 16)) if (da_ is not None and m["d_hash"]) else 999
            ok = dp <= max_dp and dd <= max_dd
            if ok:
                pa_path = anchor["path"] if "path" in anchor.keys() else None
                pm_path = m["path"] if "path" in m.keys() else None
                if pa_path and pm_path:
                    mse = _pixel_mse(pa_path, pm_path)
                    # 像素差异过大 -> 不是同一张图（哈希撞车）
                    if mse is not None and mse > max_mse:
                        ok = False
            if ok:
                keep.append(m)
            else:
                rest.append(m)
        out = [keep] if len(keep) > 1 else []
        if rest:
            out.extend(_split_by_anchor(rest, max_dp, max_dd))
        return out

    multi = []
    for g in groups.values():
        if len(g) > 1:
            for sub in _split_by_anchor(g):
                if len(sub) > 1:
                    multi.append(sub)
    total = 0
    saved = 0
    pairs = []
    for g in multi:
        kind = "exact" if len({x["sha1"] for x in g if x["sha1"]}) == 1 else "similar"
        best = max(g, key=lambda x: ((x["quality_score"] or 0), (x["megapixels"] or 0),
                                     (x["size"] or 0)))
        keep = best["size"] or 0
        waste = sum((x["size"] or 0) for x in g) - keep
        cur = lib.ex("INSERT INTO dup_groups(kind,count,best_id,keep_bytes,saved_bytes) "
                     "VALUES(?,?,?,?,?)", (kind, len(g), best["id"], keep, max(0, waste)))
        gid = cur.lastrowid
        ids = [x["id"] for x in g]
        lib.conn.executemany("UPDATE photos SET dup_group=?, is_best=? WHERE id=?",
                             [(gid, 1 if x["id"] == best["id"] else 0, x["id"]) for x in g])
        for x in g:
            pairs.append((x["id"], "重复"))
            if x["id"] == best["id"]:
                pairs.append((x["id"], "重复组最佳"))
        total += len(g)
        saved += max(0, waste)
    lib.conn.commit()
    lib.add_tags_bulk(pairs, kind="auto", source="dup")
    return {"groups": len(multi), "photos": total, "saved_bytes": saved,
            "compared": comps}


def _ham(a, b):
    return bin(int(a) ^ int(b)).count("1")
def _size_compatible(a, b, ratio):
    """判断两张图是否可能互为同一照片的不同尺寸版本。

    用三把尺子：
      1) 宽高比差异 <= 18%（容忍轻微裁剪）；
      2) 长边倍率 <= ratio（默认 6.0，容纳 1280 -> 200 这类深度缩放）；
      3) 短边倍率同理，避免只比长边时把细长条误判为同图。
    """
    try:
        aw, ah = a["width"] or 0, a["height"] or 0
        bw, bh = b["width"] or 0, b["height"] or 0
        if not (aw and ah and bw and bh):
            return True
        ra, rb = aw / float(ah), bw / float(bh)
        if max(ra, rb) / max(1e-6, min(ra, rb)) > 1.18:
            return False
        long_a, long_b = max(aw, ah), max(bw, bh)
        short_a, short_b = min(aw, ah), min(bw, bh)
        if long_a / float(max(1, long_b)) > ratio:
            return False
        if long_b / float(max(1, long_a)) > ratio:
            return False
        if short_a / float(max(1, short_b)) > ratio:
            return False
        if short_b / float(max(1, short_a)) > ratio:
            return False
        return True
    except Exception:
        return True