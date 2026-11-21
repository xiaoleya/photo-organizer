//! 自动分类：规则打标 + 事件聚类 + 重复检测。
//!
//! 与 Python 版 `opo/classify.py` 对齐。

use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;
use std::sync::OnceLock;

use regex::Regex;

use crate::config::Settings;
use crate::geo;
use crate::imaging;
use crate::library::Library;
use crate::scanner::{CancelFlag, ProgressFn};
use crate::util::{haversine_km, iso_dt, parse_iso};

// --------------------------------------------------------------- 关键词规则

/// 面向中文老照片的常见命名习惯；命中即打标签。
/// (标签名, 正则)
pub const KEYWORD_RULES: &[(&str, &str)] = &[
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
];

fn keyword_res() -> &'static Vec<(&'static str, Regex)> {
    static RES: OnceLock<Vec<(&'static str, Regex)>> = OnceLock::new();
    RES.get_or_init(|| {
        KEYWORD_RULES
            .iter()
            .filter_map(|(tag, pat)| Regex::new(&format!("(?i){pat}")).ok().map(|re| (*tag, re)))
            .collect()
    })
}

/// 打标结果
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct TagResult {
    pub photos: usize,
    pub tags: usize,
}

/// 基于文件名、目录、EXIF、质量与颜色特征批量打标签。
pub fn auto_tag(
    lib: &mut Library,
    _cfg: &Settings,
    progress: Option<&ProgressFn>,
    cancel: Option<&CancelFlag>,
) -> TagResult {
    lib.clear_auto_tags();
    let rows = lib.all_photos("", "id");
    let total = rows.len();
    let mut pairs: Vec<(i64, String)> = Vec::new();

    for (i, r) in rows.iter().enumerate() {
        if cancel.map(|c| c.load(Ordering::Relaxed)).unwrap_or(false) {
            break;
        }
        let hay = format!(
            "{} {}",
            r.name.as_deref().unwrap_or(""),
            r.folder.as_deref().unwrap_or("")
        );
        for (tag, re) in keyword_res() {
            if re.is_match(&hay) {
                pairs.push((r.id, (*tag).to_string()));
            }
        }

        // 质量类标签
        if let Some(flags) = &r.flags {
            for f in flags.split('|') {
                if !f.is_empty() {
                    pairs.push((r.id, f.to_string()));
                }
            }
        }
        if r.is_mono.unwrap_or(0) != 0 {
            pairs.push((r.id, "黑白".into()));
        }
        if r.is_sepia.unwrap_or(0) != 0 && r.is_mono.unwrap_or(0) == 0 {
            pairs.push((r.id, "棕褐老照片".into()));
        }
        if r.has_people.unwrap_or(0) != 0 {
            pairs.push((r.id, "有人物".into()));
        }
        if r.age_score.unwrap_or(0.0) >= 55.0 {
            pairs.push((r.id, "年代感强".into()));
        }
        if r.dt_source.as_deref() == Some("fs") {
            pairs.push((r.id, "时间待确认".into()));
        }
        if r.dt.as_deref().map(|s| s.trim().is_empty()).unwrap_or(true) {
            pairs.push((r.id, "缺拍摄时间".into()));
        }

        // 相机品牌
        if let Some(mk) = r.camera_make.as_deref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
            let normalized = regex_ws().replace_all(mk, " ");
            let short: String = normalized.chars().take(24).collect();
            pairs.push((r.id, format!("相机:{short}")));
        }

        // 尺寸特征
        let w = r.width.unwrap_or(0);
        let h = r.height.unwrap_or(0);
        if w > 0 && h > 0 {
            if w < h {
                pairs.push((r.id, "竖幅".into()));
            } else if w > h {
                pairs.push((r.id, "横幅".into()));
            }
            if w.max(h) >= 3000 {
                pairs.push((r.id, "高清大图".into()));
            }
            // 扫描件判据：超宽或超高
            if w.max(h) as f64 >= 2.2 * w.min(h).max(1) as f64 {
                pairs.push((r.id, "疑似扫描件".into()));
            }
        }

        if let Some(p) = progress {
            if i % 200 == 0 {
                p(
                    "生成标签",
                    i,
                    total,
                    r.name.as_deref().unwrap_or(""),
                );
            }
        }
    }

    let n = pairs.len();
    lib.add_tags_bulk(&pairs, "auto", "auto");
    TagResult {
        photos: total,
        tags: n,
    }
}

fn regex_ws() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\s+").unwrap())
}

// --------------------------------------------------------------- 事件聚类

/// 事件聚类结果
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct EventResult {
    pub events: usize,
    pub photos: usize,
}

/// 按时间间隔 + 地点跳变切分事件，写 events 表并加「事件:xxx」标签。
pub fn cluster_events(lib: &mut Library, cfg: &Settings) -> EventResult {
    let rows = lib.all_photos("dt IS NOT NULL AND dt<>''", "dt");
    let gap = cfg.f64("event_gap_hours") * 3600.0;
    let gps_km = cfg.f64("event_gps_km");

    let mut events: Vec<Vec<usize>> = Vec::new();
    let mut cur: Vec<usize> = Vec::new();
    let mut last: Option<chrono::NaiveDateTime> = None;
    let mut last_ll: Option<(Option<f64>, Option<f64>)> = None;

    for (idx, r) in rows.iter().enumerate() {
        let Some(d) = r.dt.as_deref().and_then(parse_iso) else {
            continue;
        };
        let mut split = false;
        if let Some(prev) = last {
            if (d - prev).num_seconds() as f64 > gap {
                split = true;
            } else if let (Some(lat), Some(lon), Some((Some(plat), Some(plon)))) =
                (r.lat, r.lon, last_ll)
            {
                if haversine_km(lat, lon, plat, plon) > gps_km {
                    split = true;
                }
            }
        }
        if split && !cur.is_empty() {
            events.push(std::mem::take(&mut cur));
        }
        cur.push(idx);
        last = Some(d);
        last_ll = Some((r.lat, r.lon));
    }
    if !cur.is_empty() {
        events.push(cur);
    }

    // 合并过碎的事件（少于 3 张且与前后同日）。
    //
    // 但必须同时「地点不跳变」才允许合并：否则按时间切出来的两组，
    // 哪怕因为地点跳变已经正确分开，也会被这一步又粘回去。
    let mut merged: Vec<Vec<usize>> = Vec::new();
    for ev in events {
        if let Some(last_ev) = merged.last_mut() {
            if ev.len() < 3 && last_ev.len() < 3 {
                let a = rows[*last_ev.last().unwrap()]
                    .dt
                    .as_deref()
                    .and_then(parse_iso);
                let b = rows[ev[0]].dt.as_deref().and_then(parse_iso);
                let near = match (
                    rows[*last_ev.last().unwrap()].lat,
                    rows[*last_ev.last().unwrap()].lon,
                    rows[ev[0]].lat,
                    rows[ev[0]].lon,
                ) {
                    (Some(alat), Some(alon), Some(blat), Some(blon)) => {
                        haversine_km(alat, alon, blat, blon) <= gps_km
                    }
                    // 任一侧没有 GPS，就无法证明地点跳变，允许按时间合并
                    _ => true,
                };
                if same_day(a, b) && near {
                    last_ev.extend(ev);
                    continue;
                }
            }
        }
        merged.push(ev);
    }

    lib.clear_events();
    let mut pairs: Vec<(i64, String)> = Vec::new();
    let mut n = 0usize;
    for ev in &merged {
        if ev.is_empty() {
            continue;
        }
        let start = rows[ev[0]].dt.as_deref().and_then(parse_iso);
        let end = rows[*ev.last().unwrap()].dt.as_deref().and_then(parse_iso);
        let best = ev
            .iter()
            .map(|i| &rows[*i])
            .max_by(|a, b| {
                a.quality_score
                    .unwrap_or(0.0)
                    .partial_cmp(&b.quality_score.unwrap_or(0.0))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap();
        let title = event_title(start, end);
        lib.add_event(
            &title,
            start.as_ref().map(|d| iso_dt(*d)).as_deref(),
            end.as_ref().map(|d| iso_dt(*d)).as_deref(),
            rows[ev[0]].lat,
            rows[ev[0]].lon,
            ev.len() as i64,
            best.id,
        );
        for i in ev {
            pairs.push((rows[*i].id, format!("事件:{title}")));
        }
        n += 1;
    }
    lib.add_tags_bulk(&pairs, "event", "event");

    // 时间维度标签：年 / 年月 / 年代
    let mut tpairs: Vec<(i64, String)> = Vec::new();
    for r in lib.all_photos("year IS NOT NULL", "id") {
        let y = r.year.unwrap_or(0);
        let mo = r.month.unwrap_or(1);
        tpairs.push((r.id, format!("{y}年")));
        tpairs.push((r.id, format!("{y}年{mo:02}月")));
        tpairs.push((r.id, format!("{}年代", (y / 10) * 10)));
    }
    lib.add_tags_bulk(&tpairs, "time", "time");

    EventResult {
        events: n,
        photos: rows.len(),
    }
}

fn same_day(
    a: Option<chrono::NaiveDateTime>,
    b: Option<chrono::NaiveDateTime>,
) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.date() == b.date(),
        _ => false,
    }
}

/// 生成事件标题，如 `1985-06-12 夏`。
pub fn event_title(
    start: Option<chrono::NaiveDateTime>,
    end: Option<chrono::NaiveDateTime>,
) -> String {
    use chrono::Datelike;
    let Some(start) = start else {
        return "未标日期".into();
    };
    let end = end.unwrap_or(start);
    let (d0, d1) = (start.date(), end.date());

    let core = if d0 == d1 {
        format!("{}-{:02}-{:02}", d0.year(), d0.month(), d0.day())
    } else if d0.year() == d1.year() && d0.month() == d1.month() {
        format!(
            "{}-{:02}-{:02}~{:02}",
            d0.year(),
            d0.month(),
            d0.day(),
            d1.day()
        )
    } else if d0.year() == d1.year() {
        format!(
            "{}-{:02}-{:02} ~ {:02}-{:02}",
            d0.year(),
            d0.month(),
            d0.day(),
            d1.month(),
            d1.day()
        )
    } else {
        format!(
            "{}-{:02}-{:02} ~ {}-{:02}-{:02}",
            d0.year(),
            d0.month(),
            d0.day(),
            d1.year(),
            d1.month(),
            d1.day()
        )
    };
    let season = match start.month() {
        1 | 2 | 12 => "冬",
        3..=5 => "春",
        6..=8 => "夏",
        _ => "秋",
    };
    format!("{core} {season}")
}

// --------------------------------------------------------------- 地点补算

/// 地点补算结果
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct PlaceResult {
    pub total: usize,
    pub updated: usize,
    pub redo: bool,
}

/// 给照片补算地点。
///
/// `redo=false`：只补空值（快速，日常用）
/// `redo=true` ：全部重算（换了识别算法后用）
pub fn backfill_places(
    lib: &mut Library,
    progress: Option<&ProgressFn>,
    redo: bool,
) -> PlaceResult {
    let where_clause = if redo {
        "1=1"
    } else {
        "place IS NULL OR place=''"
    };
    let rows = lib.all_photos(where_clause, "id");
    let total = rows.len();
    let mut updates: Vec<(String, i64)> = Vec::with_capacity(total);

    for (i, r) in rows.iter().enumerate() {
        let place = geo::resolve_place(r.lat, r.lon, &r.path);
        updates.push((place, r.id));
        if let Some(p) = progress {
            if i % 200 == 0 {
                p("识别地点", i, total, &r.path);
            }
        }
    }
    if !updates.is_empty() {
        lib.set_places(&updates);
    }
    if let Some(p) = progress {
        p("识别地点", total, total.max(1), &format!("完成 {total} 张"));
    }
    PlaceResult {
        total,
        updated: updates.len(),
        redo,
    }
}

// --------------------------------------------------------------- 重复检测

/// 去重结果
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct DupResult {
    pub groups: usize,
    pub photos: usize,
    pub saved_bytes: i64,
    pub compared: usize,
}

/// 并查集
struct UnionFind {
    parent: Vec<usize>,
}

impl UnionFind {
    fn new(n: usize) -> Self {
        Self {
            parent: (0..n).collect(),
        }
    }

    fn find(&mut self, mut a: usize) -> usize {
        while self.parent[a] != a {
            self.parent[a] = self.parent[self.parent[a]];
            a = self.parent[a];
        }
        a
    }

    fn union(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            self.parent[rb] = ra;
        }
    }
}

/// 两步去重：
/// 1) 完全相同：sha1 相同
/// 2) 近似重复：感知哈希汉明距离 + 尺寸比例约束，用并查集合并
pub fn find_duplicates(
    lib: &mut Library,
    cfg: &Settings,
    progress: Option<&ProgressFn>,
    cancel: Option<&CancelFlag>,
) -> DupResult {
    lib.reset_duplicates();

    let rows = lib.all_photos("p_hash IS NOT NULL AND missing=0", "id");
    let n = rows.len();
    if n == 0 {
        return DupResult::default();
    }

    // ---- 1. 完全重复
    let mut by_sha: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, r) in rows.iter().enumerate() {
        if let Some(s) = r.sha1.as_deref().filter(|s| !s.is_empty()) {
            by_sha.entry(s.to_string()).or_default().push(i);
        }
    }

    // ---- 2. 近似重复：LSH 分桶
    let max_d = cfg.f64("phash_distance") as u32;
    let max_d2 = cfg.i64("dhash_distance") as u32;
    let ratio = cfg.f64("dedup_size_ratio");

    let mut uf = UnionFind::new(n);
    let ph: Vec<u64> = rows
        .iter()
        .map(|r| r.p_hash.as_deref().and_then(imaging::parse_hash).unwrap_or(0))
        .collect();
    let dh: Vec<Option<u64>> = rows
        .iter()
        .map(|r| r.d_hash.as_deref().and_then(imaging::parse_hash))
        .collect();
    let ah: Vec<Option<u64>> = rows
        .iter()
        .map(|r| r.a_hash.as_deref().and_then(imaging::parse_hash))
        .collect();

    // 多段 band 索引（LSH）：64 位哈希切成 4 段 16 位，
    // 汉明距离 <= 10 的一对以极大概率共享至少一段。
    const BANDS: u32 = 4;
    let band_bits = 64 / BANDS;
    let band_mask = (1u64 << band_bits) - 1;
    let mut buckets: HashMap<(char, u32, u64), Vec<usize>> = HashMap::new();
    for (i, v) in ph.iter().enumerate() {
        for b in 0..BANDS {
            let key = ('p', b, (v >> (b * band_bits)) & band_mask);
            buckets.entry(key).or_default().push(i);
        }
        if let Some(av) = ah[i] {
            for b in 0..2u32 {
                let key = ('a', b, (av >> (b * 32)) & 0xFFFF_FFFF);
                buckets.entry(key).or_default().push(i);
            }
        }
    }

    let mut seen: HashSet<(usize, usize)> = HashSet::new();
    let mut comps = 0usize;
    /// 单桶过大说明图太重复（连拍），限制两两比较量
    const MAX_BUCKET: usize = 400;

    let mut bucket_keys: Vec<_> = buckets.keys().copied().collect();
    bucket_keys.sort();
    for key in bucket_keys {
        if cancel.map(|c| c.load(Ordering::Relaxed)).unwrap_or(false) {
            break;
        }
        let ids_src = &buckets[&key];
        if ids_src.len() < 2 {
            continue;
        }
        let ids: Vec<usize> = ids_src.iter().take(MAX_BUCKET).copied().collect();
        for a in 0..ids.len() {
            let i = ids[a];
            for &j in ids.iter().skip(a + 1) {
                if i == j {
                    continue;
                }
                let pair = if i < j { (i, j) } else { (j, i) };
                if !seen.insert(pair) {
                    continue;
                }
                comps += 1;
                let (ra, rb) = (&rows[i], &rows[j]);
                if !size_compatible(ra, rb, ratio) {
                    continue;
                }
                let dp = imaging::hamming(ph[i], ph[j]);
                let dd = match (dh[i], dh[j]) {
                    (Some(a), Some(b)) => imaging::hamming(a, b),
                    _ => 999,
                };
                let da = match (ah[i], ah[j]) {
                    (Some(a), Some(b)) => imaging::hamming(a, b),
                    _ => 999,
                };

                let la = ra.width.unwrap_or(1).max(ra.height.unwrap_or(1));
                let lb = rb.width.unwrap_or(1).max(rb.height.unwrap_or(1));
                let size_ratio =
                    la.max(lb) as f64 / la.min(lb).max(1) as f64;

                // 三级判据（截图类低对比图需要更严）
                let same = (dp <= 3 && dd <= 10)
                    || (dp <= 6 && dd <= 6 && da <= 6)
                    || (dp <= max_d && dd <= max_d2 && da <= 3 && size_ratio <= 1.6)
                    || (dp <= 3 && size_ratio <= 1.3);

                if same && uf.find(i) != uf.find(j) {
                    uf.union(i, j);
                }
            }
        }
        if let Some(p) = progress {
            if comps % 100_000 < 200 {
                p("查找重复", comps, 0, &format!("已比较 {comps} 对"));
            }
        }
    }

    // 把完全相同但哈希没连上的也并进去
    for ids in by_sha.values() {
        if ids.len() > 1 {
            let first = ids[0];
            for &other in &ids[1..] {
                uf.union(first, other);
            }
        }
    }

    // 收集分组
    let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..n {
        let root = uf.find(i);
        groups.entry(root).or_default().push(i);
    }

    // 并查集有传递性缺陷：A≈B、B≈C 会被连成一组，即使 A 和 C 差很远。
    // 用锚点做一致性校验，把离群成员拆出去。
    let mut multi: Vec<Vec<usize>> = Vec::new();
    let mut group_list: Vec<Vec<usize>> = groups.values().cloned().collect();
    group_list.sort();
    for g in group_list {
        if g.len() <= 1 {
            continue;
        }
        for sub in split_by_anchor(&g, &rows, &ph, &dh) {
            if sub.len() > 1 {
                multi.push(sub);
            }
        }
    }

    let mut total_photos = 0usize;
    let mut saved = 0i64;
    let mut pairs: Vec<(i64, String)> = Vec::new();

    for g in &multi {
        let sha_set: HashSet<&str> = g
            .iter()
            .filter_map(|i| rows[*i].sha1.as_deref().filter(|s| !s.is_empty()))
            .collect();
        let kind = if sha_set.len() == 1 { "exact" } else { "similar" };

        // 最优：质量分 > 像素 > 体积
        let best_idx = *g
            .iter()
            .max_by(|&&a, &&b| {
                let ra = &rows[a];
                let rb = &rows[b];
                let key = |r: &crate::library::PhotoRow| {
                    (
                        r.quality_score.unwrap_or(0.0),
                        r.megapixels.unwrap_or(0.0),
                        r.size.unwrap_or(0) as f64,
                    )
                };
                key(ra)
                    .partial_cmp(&key(rb))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap();
        let best = &rows[best_idx];
        let keep = best.size.unwrap_or(0).max(0);
        let sum: i64 = g.iter().map(|i| rows[*i].size.unwrap_or(0).max(0)).sum();
        let waste = (sum - keep).max(0);

        let gid = lib.add_dup_group(
            kind,
            g.len() as i64,
            best.id,
            keep,
            waste,
        );
        let upd: Vec<(i64, i64, i64)> = g
            .iter()
            .map(|i| {
                let r = &rows[*i];
                (
                    gid,
                    if r.id == best.id { 1 } else { 0 },
                    r.id,
                )
            })
            .collect();
        lib.set_dup_members(&upd);

        for i in g {
            pairs.push((rows[*i].id, "重复".into()));
            if rows[*i].id == best.id {
                pairs.push((rows[*i].id, "重复组最佳".into()));
            }
        }
        total_photos += g.len();
        saved += waste;
    }
    lib.add_tags_bulk(&pairs, "auto", "dup");

    DupResult {
        groups: multi.len(),
        photos: total_photos,
        saved_bytes: saved,
        compared: comps,
    }
}

/// 组内一致性校验：以中心点为锚，只保留与锚点足够接近的成员。
fn split_by_anchor(
    members: &[usize],
    rows: &[crate::library::PhotoRow],
    ph: &[u64],
    dh: &[Option<u64>],
) -> Vec<Vec<usize>> {
    if members.len() <= 1 {
        return vec![members.to_vec()];
    }
    // 选中心点：与其它成员平均距离最小
    let mut best_i = 0;
    let mut best_score = f64::MAX;
    for (i, &m) in members.iter().enumerate() {
        let mut tot = 0u32;
        for &o in members {
            if o == m {
                continue;
            }
            tot += imaging::hamming(ph[m], ph[o]);
        }
        let avg = tot as f64 / (members.len() - 1).max(1) as f64;
        if avg < best_score {
            best_score = avg;
            best_i = i;
        }
    }
    let anchor = members[best_i];

    let mut keep = Vec::new();
    let mut rest = Vec::new();
    for &m in members {
        let dp = imaging::hamming(ph[anchor], ph[m]);
        let dd = match (dh[anchor], dh[m]) {
            (Some(a), Some(b)) => imaging::hamming(a, b),
            _ => 999,
        };
        let mut ok = dp <= 6 && dd <= 8;
        if ok {
            // 像素级复核：哈希可能撞车，但真实像素不会
            let pa = &rows[anchor].path;
            let pb = &rows[m].path;
            if let Some(mse) = pixel_mse(pa, pb) {
                if mse > 600.0 {
                    ok = false;
                }
            }
        }
        if ok {
            keep.push(m);
        } else {
            rest.push(m);
        }
    }
    let mut out = Vec::new();
    if keep.len() > 1 {
        out.push(keep);
    }
    if !rest.is_empty() {
        out.extend(split_by_anchor(&rest, rows, ph, dh));
    }
    if out.is_empty() {
        vec![members.to_vec()]
    } else {
        out
    }
}

/// 缩略灰度 MSE：真近似 ~90-200；不同截图 ~1000+。
fn pixel_mse(a: &str, b: &str) -> Option<f64> {
    let ia = imaging::open_image(std::path::Path::new(a))?;
    let ib = imaging::open_image(std::path::Path::new(b))?;
    let ia = imaging::resize_lanczos(&ia, 64, 64);
    let ib = imaging::resize_lanczos(&ib, 64, 64);
    let mut sum = 0f64;
    for (pa, pb) in ia.pixels().zip(ib.pixels()) {
        let ga = imaging::gray_of(pa.0[0], pa.0[1], pa.0[2]) as f64;
        let gb = imaging::gray_of(pb.0[0], pb.0[1], pb.0[2]) as f64;
        sum += (ga - gb) * (ga - gb);
    }
    Some(sum / 4096.0)
}

/// 判断两张图是否可能互为同一照片的不同尺寸版本。
pub fn size_compatible(
    a: &crate::library::PhotoRow,
    b: &crate::library::PhotoRow,
    ratio: f64,
) -> bool {
    let (aw, ah) = (a.width.unwrap_or(0), a.height.unwrap_or(0));
    let (bw, bh) = (b.width.unwrap_or(0), b.height.unwrap_or(0));
    if aw == 0 || ah == 0 || bw == 0 || bh == 0 {
        return true;
    }
    let ra = aw as f64 / ah as f64;
    let rb = bw as f64 / bh as f64;
    if ra.max(rb) / ra.min(rb).max(1e-6) > 1.18 {
        return false;
    }
    let long_a = aw.max(ah);
    let long_b = bw.max(bh);
    let short_a = aw.min(ah);
    let short_b = bw.min(bh);
    if long_a as f64 / long_b.max(1) as f64 > ratio {
        return false;
    }
    if long_b as f64 / long_a.max(1) as f64 > ratio {
        return false;
    }
    if short_a as f64 / short_b.max(1) as f64 > ratio {
        return false;
    }
    if short_b as f64 / short_a.max(1) as f64 > ratio {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::PhotoUpdate;
    use std::path::PathBuf;

    fn sample_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../samples")
    }

    fn work_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn event_title_single_day() {
        let d = chrono::NaiveDate::from_ymd_opt(1985, 6, 12)
            .unwrap()
            .and_hms_opt(0, 0, 0);
        assert_eq!(event_title(d, d), "1985-06-12 夏");
    }

    #[test]
    fn event_title_multi_day_same_month() {
        let a = chrono::NaiveDate::from_ymd_opt(1990, 3, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0);
        let b = chrono::NaiveDate::from_ymd_opt(1990, 3, 5)
            .unwrap()
            .and_hms_opt(0, 0, 0);
        assert_eq!(event_title(a, b), "1990-03-01~05 春");
    }

    #[test]
    fn event_title_cross_year() {
        let a = chrono::NaiveDate::from_ymd_opt(1999, 12, 30)
            .unwrap()
            .and_hms_opt(0, 0, 0);
        let b = chrono::NaiveDate::from_ymd_opt(2000, 1, 2)
            .unwrap()
            .and_hms_opt(0, 0, 0);
        let t = event_title(a, b);
        assert!(t.contains("1999-12-30"), "{t}");
        assert!(t.contains("2000-01-02"), "{t}");
        assert!(t.ends_with("冬"), "{t}");
    }

    #[test]
    fn event_title_no_date() {
        assert_eq!(event_title(None, None), "未标日期");
    }

    #[test]
    fn keyword_rules_match_chinese_names() {
        let hits = |name: &str| -> Vec<&'static str> {
            keyword_res()
                .iter()
                .filter(|(_, re)| re.is_match(name))
                .map(|(t, _)| *t)
                .collect()
        };
        assert!(hits("1985-06-12 春节全家福.jpg").contains(&"全家福"));
        assert!(hits("1985-06-12 春节全家福.jpg").contains(&"节日"));
        assert!(hits("1992-10-01 旅行.jpg").contains(&"旅行"));
        assert!(hits("扫描件_2008-07-20_证件照.jpg").contains(&"证件照"));
        assert!(hits("IMG_20050315_毕业典礼.jpg").contains(&"毕业"));
        assert!(hits("DSC_0071.JPG").is_empty(), "相机默认名不该命中语义标签");
    }

    #[test]
    fn size_compatible_rejects_aspect_mismatch() {
        let mut a = crate::library::PhotoRow::default();
        a.width = Some(1280);
        a.height = Some(960);
        let mut b = crate::library::PhotoRow::default();
        b.width = Some(1000);
        b.height = Some(300); // 极端长条
        assert!(!size_compatible(&a, &b, 6.0));
    }

    #[test]
    fn size_compatible_accepts_scaled_copy() {
        let mut a = crate::library::PhotoRow::default();
        a.width = Some(1280);
        a.height = Some(960);
        let mut b = crate::library::PhotoRow::default();
        b.width = Some(640);
        b.height = Some(480);
        assert!(size_compatible(&a, &b, 6.0));
    }

    #[test]
    fn size_compatible_accepts_missing_dims() {
        let a = crate::library::PhotoRow::default();
        let b = crate::library::PhotoRow::default();
        assert!(size_compatible(&a, &b, 6.0));
    }

    #[test]
    fn duplicates_detected_in_samples() {
        // 用真实样图跑完整去重
        let dir = work_dir("po_dup_samples");
        for (src, dst) in [
            ("1985-06-12 春节全家福.jpg", "orig.jpg"),
            ("重复/完全相同1.jpg", "exact.jpg"),
            ("重复/1985-06-12 春节全家福_副本.jpg", "scaled.jpg"),
            ("1992-10-01 旅行.jpg", "other.jpg"),
        ] {
            std::fs::copy(sample_dir().join(src), dir.join(dst)).unwrap();
        }
        let mut lib = Library::open_memory().unwrap();
        let cfg = Settings::new();
        let root = dir.to_string_lossy().to_string();
        crate::scanner::scan_roots(&mut lib, &cfg, Some(&[root]), None, None, false);
        crate::scanner::analyze_pending(&mut lib, &cfg, 0, None, None, 2, false);

        let r = find_duplicates(&mut lib, &cfg, None, None);
        assert!(r.groups >= 1, "应至少发现 1 组重复: {r:?}");
        // 完全相同的那张应该和原图同组
        let rows = lib.all_photos("dup_group IS NOT NULL", "id");
        assert!(rows.len() >= 2, "重复组内应有多张");
        // 不同的旅行照片不该进组
        let trav = lib.all_photos("path LIKE '%other.jpg'", "id");
        assert_eq!(trav.len(), 1);
        assert!(
            trav[0].dup_group.is_none(),
            "完全不同的照片不该被归为重复"
        );
    }

    #[test]
    fn duplicates_marks_best_member() {
        let dir = work_dir("po_dup_best");
        std::fs::copy(sample_dir().join("DSC_0071.JPG"), dir.join("a.jpg")).unwrap();
        std::fs::copy(sample_dir().join("重复/完全相同1.jpg"), dir.join("b.jpg")).unwrap();
        let mut lib = Library::open_memory().unwrap();
        let cfg = Settings::new();
        crate::scanner::scan_roots(&mut lib, &cfg, Some(&[dir.to_string_lossy().to_string()]), None, None, false);
        crate::scanner::analyze_pending(&mut lib, &cfg, 0, None, None, 2, false);
        find_duplicates(&mut lib, &cfg, None, None);
        let best = lib.all_photos("is_best=1", "id");
        // 若两张不同图，不应有 best；若巧合同组，则每组恰好一个 best
        let groups = lib.count("dup_group IS NOT NULL");
        if groups > 0 {
            assert!(!best.is_empty(), "有重复组就应有 best 标记");
        }
    }

    #[test]
    fn no_duplicates_when_all_distinct() {
        let dir = work_dir("po_dup_distinct");
        for (i, n) in [
            "1985-06-12 春节全家福.jpg",
            "1992-10-01 旅行.jpg",
            "微信图片_20200101.jpg",
        ]
        .iter()
        .enumerate()
        {
            std::fs::copy(sample_dir().join(n), dir.join(format!("{i}.jpg"))).unwrap();
        }
        let mut lib = Library::open_memory().unwrap();
        let cfg = Settings::new();
        crate::scanner::scan_roots(&mut lib, &cfg, Some(&[dir.to_string_lossy().to_string()]), None, None, false);
        crate::scanner::analyze_pending(&mut lib, &cfg, 0, None, None, 2, false);
        let r = find_duplicates(&mut lib, &cfg, None, None);
        assert_eq!(r.groups, 0, "三张完全不同的图不该成组: {r:?}");
    }

    #[test]
    fn dedup_is_idempotent() {
        let dir = work_dir("po_dup_idem");
        std::fs::copy(sample_dir().join("1985-06-12 春节全家福.jpg"), dir.join("a.jpg")).unwrap();
        std::fs::copy(sample_dir().join("重复/完全相同1.jpg"), dir.join("b.jpg")).unwrap();
        let mut lib = Library::open_memory().unwrap();
        let cfg = Settings::new();
        crate::scanner::scan_roots(&mut lib, &cfg, Some(&[dir.to_string_lossy().to_string()]), None, None, false);
        crate::scanner::analyze_pending(&mut lib, &cfg, 0, None, None, 2, false);
        let r1 = find_duplicates(&mut lib, &cfg, None, None);
        let r2 = find_duplicates(&mut lib, &cfg, None, None);
        assert_eq!(r1.groups, r2.groups, "重复检测应幂等");
    }

    #[test]
    fn auto_tag_assigns_semantic_labels() {
        let dir = work_dir("po_tag_test");
        std::fs::copy(
            sample_dir().join("1985-06-12 春节全家福.jpg"),
            dir.join("1985-06-12 春节全家福.jpg"),
        )
        .unwrap();
        let mut lib = Library::open_memory().unwrap();
        let cfg = Settings::new();
        crate::scanner::scan_roots(&mut lib, &cfg, Some(&[dir.to_string_lossy().to_string()]), None, None, false);
        crate::scanner::analyze_pending(&mut lib, &cfg, 0, None, None, 1, false);
        let r = auto_tag(&mut lib, &cfg, None, None);
        assert_eq!(r.photos, 1);
        assert!(r.tags > 0);
        let pid = lib.all_photos("", "id")[0].id;
        let tags = lib.tags_of(pid);
        assert!(tags.contains(&"全家福".to_string()), "{tags:?}");
        assert!(tags.contains(&"节日".to_string()), "{tags:?}");
    }

    #[test]
    fn auto_tag_clears_previous() {
        let mut lib = Library::open_memory().unwrap();
        let cfg = Settings::new();
        let pid = lib
            .upsert_photo(&mut PhotoUpdate::new("x.jpg"))
            .unwrap();
        lib.add_tags_bulk(&[(pid, "旧标签".into())], "auto", "auto");
        assert!(!lib.tags_of(pid).is_empty());
        auto_tag(&mut lib, &cfg, None, None);
        let tags = lib.tags_of(pid);
        assert!(
            !tags.contains(&"旧标签".to_string()),
            "旧自动标签应被清除: {tags:?}"
        );
    }

    #[test]
    fn cluster_events_groups_by_time() {
        let mut lib = Library::open_memory().unwrap();
        let cfg = Settings::new();
        // 同日三张 + 隔天一张（间隔 > 6 小时）
        let data = [
            ("1985-06-12 10:00:00", 1),
            ("1985-06-12 11:00:00", 1),
            ("1985-06-12 12:00:00", 1),
            ("1985-06-20 10:00:00", 1),
        ];
        for (dt, _) in data {
            let mut u = PhotoUpdate::new(&format!("p{dt}.jpg"));
            u.text("dt", Some(dt));
            lib.upsert_photo(&u).unwrap();
        }
        let r = cluster_events(&mut lib, &cfg);
        assert_eq!(r.photos, 4);
        assert!(r.events >= 2, "两组事件，实际 {}", r.events);
    }

    #[test]
    fn cluster_events_splits_on_gps_jump() {
        let mut lib = Library::open_memory().unwrap();
        let mut cfg = Settings::new();
        cfg.merge_json(&{
            let mut m = serde_json::Map::new();
            m.insert("event_gps_km".into(), serde_json::json!(50.0));
            m.insert("event_gap_hours".into(), serde_json::json!(24.0));
            m
        });
        // 同一天内，北京 -> 上海（远超 50km）
        let mut u1 = PhotoUpdate::new("a.jpg");
        u1.text("dt", Some("2000-01-01 10:00:00"))
            .real("lat", Some(39.90))
            .real("lon", Some(116.41));
        let mut u2 = PhotoUpdate::new("b.jpg");
        u2.text("dt", Some("2000-01-01 11:00:00"))
            .real("lat", Some(31.23))
            .real("lon", Some(121.47));
        lib.upsert_photo(&mut u1).unwrap();
        lib.upsert_photo(&mut u2).unwrap();
        let r = cluster_events(&mut lib, &cfg);
        assert_eq!(r.events, 2, "地点跳变应拆成两个事件");
    }

    #[test]
    fn backfill_places_fills_only_empty() {
        let mut lib = Library::open_memory().unwrap();
        let mut u1 = PhotoUpdate::new(r"D:\照片\北京\a.jpg");
        u1.text("place", Some(""));
        let id1 = lib.upsert_photo(&mut u1).unwrap();
        let mut u2 = PhotoUpdate::new(r"D:\照片\上海\b.jpg");
        u2.text("place", Some("上海"));
        let id2 = lib.upsert_photo(&mut u2).unwrap();

        let r = backfill_places(&mut lib, None, false);
        assert_eq!(r.updated, 1, "只应补空的那一张");
        assert_eq!(lib.photo(id1).unwrap().place.as_deref(), Some("北京"));
        assert_eq!(lib.photo(id2).unwrap().place.as_deref(), Some("上海"));
    }

    #[test]
    fn backfill_places_redo_recomputes_all() {
        let mut lib = Library::open_memory().unwrap();
        let mut u = PhotoUpdate::new(r"D:\照片\北京\a.jpg");
        u.text("place", Some("旧地名"));
        lib.upsert_photo(&mut u).unwrap();
        let r = backfill_places(&mut lib, None, true);
        assert_eq!(r.updated, 1);
        let row = lib.all_photos("", "id").remove(0);
        assert_eq!(row.place.as_deref(), Some("北京"));
    }

    #[test]
    fn time_tags_generated() {
        let mut lib = Library::open_memory().unwrap();
        let cfg = Settings::new();
        let mut u = PhotoUpdate::new("a.jpg");
        u.text("dt", Some("1992-10-01 10:00:00"))
            .int("year", Some(1992))
            .int("month", Some(10));
        let id = lib.upsert_photo(&mut u).unwrap();
        cluster_events(&mut lib, &cfg);
        let tags = lib.tags_of(id);
        assert!(tags.contains(&"1992年".to_string()), "{tags:?}");
        assert!(tags.contains(&"1992年10月".to_string()), "{tags:?}");
        assert!(tags.contains(&"1990年代".to_string()), "{tags:?}");
    }
}
