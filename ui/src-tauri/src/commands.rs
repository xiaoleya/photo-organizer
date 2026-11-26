//! Tauri 命令层：把 `po-core` 引擎暴露给前端。
//!
//! 替代原来的 Python HTTP sidecar（`opo/bridge.py`）。
//! 接口语义与 `bridge.py` 的 REST 端点一一对应，返回结构保持不变，
//! 前端视图组件无需改动。
//!
//! 图片不再走 HTTP，改由 `po://` 自定义协议提供（见 main.rs）。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use po_core::classify;
use po_core::config;
use po_core::geo;
use po_core::imaging;
use po_core::library::Library;
use po_core::organizer;
use po_core::pipeline::{self, Progress, RunOptions};
use po_core::restore;
use po_core::util;

use serde::Deserialize;
use serde_json::{json, Value};
use tauri::State;

// ---------------------------------------------------------------- 缩略图缓存

pub struct ThumbCache {
    mem: HashMap<(String, u32), Vec<u8>>,
    order: Vec<(String, u32)>,
}

const THUMB_MEM_MAX: usize = 600;

impl ThumbCache {
    fn new() -> Self {
        Self {
            mem: HashMap::new(),
            order: Vec::new(),
        }
    }

    fn get(&self, key: &(String, u32)) -> Option<Vec<u8>> {
        self.mem.get(key).cloned()
    }

    fn put(&mut self, key: (String, u32), data: Vec<u8>) {
        if self.mem.len() >= THUMB_MEM_MAX {
            if let Some(oldest) = self.order.first().cloned() {
                self.mem.remove(&oldest);
                self.order.remove(0);
            }
        }
        self.order.push(key.clone());
        self.mem.insert(key, data);
    }

    pub fn clear(&mut self) {
        self.mem.clear();
        self.order.clear();
    }
}

// ---------------------------------------------------------------- 全局状态

/// 应用共享状态。内部全部用 `Arc`，因为后台线程也要访问。
#[derive(Clone)]
pub struct AppState {
    pub lib: Arc<Mutex<Option<Library>>>,
    pub progress: Progress,
    pub thumb_cache: Arc<Mutex<ThumbCache>>,
    /// id -> (原图路径, 缩略图路径)。几百个缩略图请求同时进来时，
    /// 靠它避开 `lib` 的 Mutex 争用。
    pub path_cache: Arc<Mutex<HashMap<i64, (String, String)>>>,
    /// (path,size) -> 锁。同一张图只让一个线程解码。
    pub decode_locks: Arc<Mutex<HashMap<(String, u32), Arc<Mutex<()>>>>>,
    /// 只保存取消标志，命令无需等待正在扫描的数据库锁。
    pub scan_cancel: Arc<Mutex<Option<po_core::scanner::CancelFlag>>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            lib: Arc::new(Mutex::new(None)),
            progress: Progress::new(),
            thumb_cache: Arc::new(Mutex::new(ThumbCache::new())),
            path_cache: Arc::new(Mutex::new(HashMap::new())),
            decode_locks: Arc::new(Mutex::new(HashMap::new())),
            scan_cancel: Arc::new(Mutex::new(None)),
        }
    }

    /// 取（必要时打开）资源库。
    pub fn with_lib<T>(&self, f: impl FnOnce(&mut Library) -> T) -> Result<T, String> {
        let mut guard = self.lib.lock().map_err(|e| e.to_string())?;
        if guard.is_none() {
            let lib = Library::open_default().map_err(|e| e.to_string())?;
            *guard = Some(lib);
        }
        Ok(f(guard.as_mut().unwrap()))
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------- JSON 转换

fn opt<T: serde::Serialize>(v: &Option<T>) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

/// 照片行 -> 前端 JSON。字段名与 `bridge.py` 的 `_photo_like` 一致。
///
/// 这里用 `Map` 逐项插入而不是 `json!` 大括号字面量：
/// 50 多个字段会让 `json!` 宏递归超限（recursion_limit）。
fn photo_json(r: &po_core::library::PhotoRow) -> Value {
    let mut m = serde_json::Map::with_capacity(56);
    let mut put = |k: &str, v: Value| {
        m.insert(k.to_string(), v);
    };
    put("id", json!(r.id));
    put("path", json!(r.path));
    put("folder", opt(&r.folder));
    put("name", opt(&r.name));
    put("ext", opt(&r.ext));
    put("size", opt(&r.size));
    put("mtime", opt(&r.mtime));
    put("ctime", opt(&r.ctime));
    put("sha1", opt(&r.sha1));
    put("width", opt(&r.width));
    put("height", opt(&r.height));
    put("megapixels", opt(&r.megapixels));
    put("aspect", opt(&r.aspect));
    put("dt", opt(&r.dt));
    put("dt_source", opt(&r.dt_source));
    put("dt_precision", opt(&r.dt_precision));
    put("year", opt(&r.year));
    put("month", opt(&r.month));
    put("day", opt(&r.day));
    put("hour", opt(&r.hour));
    put("camera_make", opt(&r.camera_make));
    put("camera_model", opt(&r.camera_model));
    put("software", opt(&r.software));
    put("lat", opt(&r.lat));
    put("lon", opt(&r.lon));
    put("place", opt(&r.place));
    put("blur", opt(&r.blur));
    put("brightness", opt(&r.brightness));
    put("contrast", opt(&r.contrast));
    put("saturation", opt(&r.saturation));
    put("hue", opt(&r.hue));
    put("sepia", opt(&r.sepia));
    put("color", opt(&r.color));
    put("skin", opt(&r.skin));
    put("noise", opt(&r.noise));
    put("sharpness", opt(&r.sharpness));
    put("dark_ratio", opt(&r.dark_ratio));
    put("white_ratio", opt(&r.white_ratio));
    put("a_hash", opt(&r.a_hash));
    put("d_hash", opt(&r.d_hash));
    put("p_hash", opt(&r.p_hash));
    put("flags", opt(&r.flags));
    put("has_people", opt(&r.has_people));
    put("is_mono", opt(&r.is_mono));
    put("is_sepia", opt(&r.is_sepia));
    put("age_score", opt(&r.age_score));
    put("quality_score", opt(&r.quality_score));
    put("thumb", opt(&r.thumb));
    put("analyzed_at", opt(&r.analyzed_at));
    put("missing", json!(r.missing.unwrap_or(0)));
    put("dup_group", opt(&r.dup_group));
    put("is_best", json!(r.is_best.unwrap_or(0)));
    put("note", opt(&r.note));
    put("favorite", json!(r.favorite.unwrap_or(0)));
    put(
        "kind",
        json!(r.kind.clone().unwrap_or_else(|| "photo".into())),
    );
    put("duration", opt(&r.duration));
    put("thumb_url", json!(format!("/thumb?id={}", r.id)));
    put("photo_url", json!(format!("/photo?id={}", r.id)));
    Value::Object(m)
}

fn metrics_json(m: &imaging::Metrics) -> Value {
    let mut out = serde_json::Map::with_capacity(17);
    let mut put = |k: &str, v: Value| {
        out.insert(k.to_string(), v);
    };
    put("width", json!(m.width));
    put("height", json!(m.height));
    put("megapixels", json!(m.megapixels));
    put("aspect", json!(m.aspect));
    put("blur", json!(m.blur));
    put("brightness", json!(m.brightness));
    put("contrast", json!(m.contrast));
    put("dark_ratio", json!(m.dark_ratio));
    put("white_ratio", json!(m.white_ratio));
    put("saturation", json!(m.saturation));
    put("hue", json!(m.hue));
    put("sepia", json!(m.sepia));
    put("color", json!(m.color));
    put("skin", json!(m.skin));
    put("noise", json!(m.noise));
    put("sharpness", json!(m.sharpness));
    put("lap_var", json!(m.lap_var));
    Value::Object(out)
}

// ---------------------------------------------------------------- 基础查询

#[tauri::command]
pub fn ping() -> Value {
    json!({ "ok": true, "app": "photo-organizer", "pid": std::process::id() })
}

#[tauri::command]
pub fn get_stats(state: State<'_, AppState>) -> Result<Value, String> {
    state.with_lib(|lib| {
        let st = lib.stats();
        let known_places = lib.query_count(
            "SELECT COUNT(DISTINCT place) c FROM photos WHERE missing=0 \
             AND place IS NOT NULL AND place<>'' AND place<>?",
            &[&geo::UNKNOWN],
        );
        let unknown = lib.query_count(
            "SELECT COUNT(*) c FROM photos WHERE missing=0 AND \
             (place IS NULL OR place='' OR place=?)",
            &[&geo::UNKNOWN],
        );
        json!({
            "total": st.total,
            "analyzed": st.analyzed,
            "with_dt": st.with_dt,
            "no_dt": st.no_dt,
            "dups": st.dups,
            "missing": st.missing,
            "bytes": st.bytes,
            "years": st.years,
            "tags": st.tags,
            "places": known_places,
            "unknown_place": unknown,
        })
    })
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct PhotoQuery {
    pub flag: Option<String>,
    pub dup: Option<String>,
    /// `video` / `photo` / 空（全部）
    pub kind: Option<String>,
    pub nodate: Option<String>,
    pub dup_group: Option<i64>,
    pub fav: Option<String>,
    pub year: Option<i64>,
    pub month: Option<i64>,
    /// 年份区间（含端点）。单独于精确的 `year`，用于「1985-1990」这类查询
    pub year_from: Option<i64>,
    pub year_to: Option<i64>,
    /// 只看判定为「有人物」的照片
    pub has_people: Option<String>,
    pub place: Option<String>,
    pub noplace: Option<String>,
    pub min_quality: Option<f64>,
    pub max_quality: Option<f64>,
    pub q: Option<String>,
    pub sort: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
    /// 标签名，走 tags/photo_tags 关联查询
    pub tag: Option<String>,
}

fn truthy(s: &Option<String>) -> bool {
    matches!(s.as_deref(), Some("1") | Some("true") | Some("yes"))
}

/// 把搜索串拆成关键词。
///
/// * 按空白（空格 / 全角空格 / 制表符）拆分
/// * 去掉空串与重复词（避免 `a a` 生成两条相同条件）
/// * 上限 8 个词：再多的话 SQL 条件会膨胀，而且实际检索意义不大
fn split_keywords(q: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for w in q.split(|c: char| c.is_whitespace() || c == '\u{3000}') {
        let w = w.trim();
        if w.is_empty() {
            continue;
        }
        if out.iter().any(|x| x == w) {
            continue;
        }
        out.push(w.to_string());
        if out.len() >= 8 {
            break;
        }
    }
    out
}

/// 把 PhotoQuery 编译成 (WHERE 子句, 参数)。
///
/// 抽出来是为了让「照片列表」「全选全部筛选结果」「智能相册」共用同一套筛选语义，
/// 避免三处各写一遍导致条件不一致。
fn build_photo_filter(query: &PhotoQuery) -> (String, Vec<String>) {
    let mut where_parts: Vec<String> = vec!["missing=0".into()];
    let mut args: Vec<String> = Vec::new();

    if let Some(f) = query.flag.as_deref().filter(|s| !s.is_empty()) {
        where_parts.push("flags LIKE ?".into());
        args.push(format!("%{f}%"));
    }
    match query.dup.as_deref() {
        Some("only") => where_parts.push("dup_group IS NOT NULL".into()),
        Some("none") => where_parts.push("dup_group IS NULL".into()),
        _ => {}
    }
    match query.kind.as_deref() {
        Some("video") => where_parts.push("kind='video'".into()),
        Some("photo") => where_parts.push("(kind IS NULL OR kind='photo')".into()),
        _ => {}
    }
    if truthy(&query.nodate) {
        where_parts.push("(dt IS NULL OR TRIM(dt)='')".into());
    }
    if let Some(g) = query.dup_group {
        where_parts.push("dup_group = ?".into());
        args.push(g.to_string());
    }
    if truthy(&query.fav) {
        where_parts.push("favorite=1".into());
    }
    if let Some(y) = query.year {
        where_parts.push("year = ?".into());
        args.push(y.to_string());
    }
    if let Some(y) = query.year_from {
        where_parts.push("year >= ?".into());
        args.push(y.to_string());
    }
    if let Some(y) = query.year_to {
        where_parts.push("year <= ?".into());
        args.push(y.to_string());
    }
    if truthy(&query.has_people) {
        where_parts.push("has_people=1".into());
    }
    if let Some(m) = query.month {
        where_parts.push("month = ?".into());
        args.push(m.to_string());
    }
    if let Some(p) = query.place.as_deref().filter(|s| !s.is_empty()) {
        where_parts.push("place = ?".into());
        args.push(p.to_string());
    }
    if let Some(tag) = query.tag.as_deref().filter(|s| !s.is_empty()) {
        where_parts.push(
            "id IN (SELECT photo_id FROM photo_tags \
             JOIN tags ON tags.id=photo_tags.tag_id WHERE tags.name=?)"
                .into(),
        );
        args.push(tag.to_string());
    }
    if truthy(&query.noplace) {
        where_parts.push("(place IS NULL OR place='' OR place=?)".into());
        args.push(geo::UNKNOWN.to_string());
    }
    if let Some(v) = query.min_quality {
        where_parts.push("quality_score >= ?".into());
        args.push(v.to_string());
    }
    if let Some(v) = query.max_quality {
        where_parts.push("quality_score <= ?".into());
        args.push(v.to_string());
    }
    if let Some(kw) = query.q.as_deref().filter(|s| !s.is_empty()) {
        for word in split_keywords(kw) {
            where_parts.push(
                "(name LIKE ? OR path LIKE ? OR place LIKE ? OR note LIKE ?)".into(),
            );
            let pat = format!("%{word}%");
            args.extend([pat.clone(), pat.clone(), pat.clone(), pat]);
        }
    }
    (where_parts.join(" AND "), args)
}

fn photo_order(sort: Option<&str>) -> &'static str {
    match sort {
        Some("dt DESC") => "dt IS NULL, dt DESC, id DESC",
        Some("quality_score DESC") => "quality_score DESC, id",
        Some("quality_score ASC") => "quality_score ASC, id",
        Some("age_score DESC") => "age_score DESC, id",
        _ => "dt IS NULL, dt ASC, id",
    }
}

#[tauri::command]
pub fn get_photos(state: State<'_, AppState>, query: PhotoQuery) -> Result<Value, String> {
    state.with_lib(|lib| {
        let (w, args) = build_photo_filter(&query);
        let order = photo_order(query.sort.as_deref());

        let limit = query.limit.unwrap_or(200).clamp(1, 2000);
        let offset = query.offset.unwrap_or(0).max(0);
        let total = lib.count_params(&w, &args);
        let rows = lib.query_photos_paged(&w, order, &args, limit, offset);
        let items: Vec<Value> = rows.iter().map(photo_json).collect();
        let has_more = (offset + items.len() as i64) < total;
        Ok(json!({
            "items": items,
            "total": total,
            "offset": offset,
            "limit": limit,
            "has_more": has_more,
        }))
    })?
}

#[tauri::command]
pub fn get_photo(state: State<'_, AppState>, id: i64) -> Result<Value, String> {
    state.with_lib(|lib| match lib.photo(id) {
        Some(row) => {
            let mut v = photo_json(&row);
            v["tags"] = json!(lib.tags_of(id));
            v
        }
        None => json!({ "error": "not found" }),
    })
}

#[tauri::command]
pub fn get_timeline(state: State<'_, AppState>) -> Result<Value, String> {
    state.with_lib(|lib| {
        let rows = lib.year_month_counts();
        let mut tree: std::collections::BTreeMap<
            i64,
            (i64, Vec<Value>),
        > = std::collections::BTreeMap::new();
        for (year, month, count) in rows {
            let node = tree.entry(year).or_insert((0, Vec::new()));
            node.0 += count;
            if let Some(m) = month {
                node.1.push(json!({ "month": m, "count": count }));
            }
        }
        let items: Vec<Value> = tree
            .into_iter()
            .rev()
            .map(|(year, (count, months))| {
                json!({ "year": year, "count": count, "months": months })
            })
            .collect();
        json!({ "items": items })
    })
}

#[tauri::command]
pub fn get_places(state: State<'_, AppState>) -> Result<Value, String> {
    state.with_lib(|lib| {
        let rows = lib.place_year_month_counts(geo::UNKNOWN);
        type YearMap = std::collections::BTreeMap<i64, (i64, std::collections::BTreeMap<i64, i64>)>;
        let mut nodes: HashMap<String, (String, String, i64, YearMap)> = HashMap::new();

        for (place, year, month, count) in rows {
            let (parent, leaf) = geo::place_parts(&place);
            let node = nodes
                .entry(place.clone())
                .or_insert((parent, leaf, 0, YearMap::new()));
            node.2 += count;
            if let Some(y) = year {
                let yn = node
                    .3
                    .entry(y)
                    .or_insert((0, std::collections::BTreeMap::new()));
                yn.0 += count;
                if let Some(m) = month {
                    *yn.1.entry(m).or_insert(0) += count;
                }
            }
        }

        let mut items: Vec<Value> = nodes
            .into_iter()
            .map(|(place, (parent, leaf, count, years))| {
                let years_json: Vec<Value> = years
                    .iter()
                    .rev()
                    .map(|(y, (yc, months))| {
                        let ms: Vec<Value> = months
                            .iter()
                            .map(|(m, c)| json!({ "month": m, "count": c }))
                            .collect();
                        json!({ "year": y, "count": yc, "months": ms })
                    })
                    .collect();
                json!({
                    "place": place,
                    "parent": parent,
                    "leaf": leaf,
                    "count": count,
                    "years": years_json,
                })
            })
            .collect();

        // 未知地点排最后，其余按数量降序
        items.sort_by(|a, b| {
            let au = a["place"] == geo::UNKNOWN;
            let bu = b["place"] == geo::UNKNOWN;
            au.cmp(&bu).then_with(|| {
                b["count"]
                    .as_i64()
                    .unwrap_or(0)
                    .cmp(&a["count"].as_i64().unwrap_or(0))
            })
        });
        let known = items.iter().filter(|x| x["place"] != geo::UNKNOWN).count();
        json!({ "items": items, "known_count": known })
    })
}

#[tauri::command]
pub fn get_duplicates(state: State<'_, AppState>) -> Result<Value, String> {
    state.with_lib(|lib| {
        let groups = lib.dup_groups();
        let items: Vec<Value> = groups
            .into_iter()
            .filter_map(|(gid, kind, count, best, saved)| {
                let photos = lib.photos_in_dup_group(gid);
                if photos.is_empty() {
                    return None;
                }
                let photo_items: Vec<Value> = photos.iter().map(photo_json).collect();
                Some(json!({
                    "id": gid, "kind": kind, "count": count,
                    "best_id": best, "saved_bytes": saved,
                    "photos": photo_items,
                }))
            })
            .collect();
        let total = lib.query_count("SELECT COUNT(*) c FROM dup_groups", &[]);
        json!({ "items": items, "total": total })
    })
}

#[tauri::command]
pub fn get_roots(state: State<'_, AppState>) -> Result<Value, String> {
    state.with_lib(|lib| {
        let roots = lib.roots();
        json!({ "items": roots, "total": roots.len() })
    })
}

#[tauri::command]
pub fn get_settings() -> Value {
    let s = config::load_settings();
    json!({ "settings": s.to_json_map() })
}

#[tauri::command]
pub fn get_restore_presets() -> Value {
    json!({
        "presets": restore::preset_names(),
        "options": restore::options_meta(),
        "defaults": restore::preset_options("标准修复").to_json(),
    })
}

#[tauri::command]
pub fn get_scan_progress(state: State<'_, AppState>) -> Value {
    serde_json::to_value(state.progress.snapshot()).unwrap_or(json!({}))
}

/// 启动时的状态快照：前端据此决定要不要弹「清除上次扫描记录」。
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct EditPhotoRequest {
    pub id: i64,
    pub date: Option<String>,
    pub date_precision: Option<String>,
    pub place: Option<String>,
    pub note: Option<String>,
    pub clear_date: Option<bool>,
    pub clear_place: Option<bool>,
    pub clear_note: Option<bool>,
}

#[tauri::command]
pub fn edit_photo(state: State<'_, AppState>, req: EditPhotoRequest) -> Result<Value, String> {
    state.with_lib(|lib| {
        let Some(_) = lib.photo(req.id) else {
            return json!({ "ok": false, "error": "没有这张照片" });
        };
        if req.clear_date.unwrap_or(false) {
            let _ = lib.set_manual_date(req.id, None, None);
        } else if req.date.is_some() {
            let _ = lib.set_manual_date(
                req.id,
                req.date.as_deref(),
                req.date_precision.as_deref(),
            );
        }
        if req.clear_place.unwrap_or(false) {
            let _ = lib.set_manual_place(req.id, None);
        } else if req.place.is_some() {
            let _ = lib.set_manual_place(req.id, req.place.as_deref());
        }
        if req.clear_note.unwrap_or(false) {
            let _ = lib.set_note(req.id, None);
        } else if req.note.is_some() {
            let _ = lib.set_note(req.id, req.note.as_deref());
        }
        json!({ "ok": true })
    })
}

#[tauri::command]
pub fn set_photo_tags(
    state: State<'_, AppState>,
    id: i64,
    tags: Vec<String>,
) -> Result<Value, String> {
    state.with_lib(|lib| {
        let Some(_) = lib.photo(id) else {
            return json!({ "ok": false, "error": "没有这张照片" });
        };
        // 只管理 user 标签：删除这张照片现有的自定义标签，再按顺序写入
        let existing = lib.tags_of(id);
        for name in existing {
            lib.remove_tag(id, &name);
        }
        for name in tags.iter().map(|s| s.trim()).filter(|s| !s.is_empty()) {
            lib.add_tag(id, name, "user", "user");
        }
        json!({ "ok": true, "tags": lib.tags_of(id) })
    })
}

// ---------------------------------------------------------------- 写操作

#[tauri::command]
pub fn save_settings(settings: Value) -> Value {
    let mut s = config::load_settings();
    let changed = settings
        .as_object()
        .map(|m| s.merge_json(m))
        .unwrap_or(false);
    if changed {
        config::save_settings(&s);
    }
    json!({ "ok": changed, "settings": s.to_json_map() })
}

#[tauri::command]
pub fn add_root(state: State<'_, AppState>, path: String) -> Result<Value, String> {
    let clean = path.trim().trim_matches('"').to_string();
    if clean.is_empty() {
        return Ok(json!({ "ok": false, "error": "路径为空" }));
    }
    if !Path::new(&clean).is_dir() {
        return Ok(json!({ "ok": false, "error": format!("目录不存在：{clean}") }));
    }
    state.with_lib(|lib| {
        lib.add_root(Path::new(&clean));
        json!({ "ok": true, "path": clean, "roots": lib.roots() })
    })
}

#[tauri::command]
pub fn remove_root(state: State<'_, AppState>, path: String) -> Result<Value, String> {
    let clean = path.trim().to_string();
    if clean.is_empty() {
        return Ok(json!({ "ok": false, "error": "路径为空" }));
    }
    state.with_lib(|lib| {
        lib.remove_root(Path::new(&clean));
        json!({ "ok": true, "roots": lib.roots() })
    })
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct ScanRequest {
    pub roots: Option<Vec<String>>,
    pub reanalyze_all: Option<bool>,
}

#[tauri::command]
pub fn scan_start(state: State<'_, AppState>, req: ScanRequest) -> Result<Value, String> {
    let roots = state.with_lib(|lib| match req.roots.clone().filter(|r| !r.is_empty()) {
        Some(r) => r,
        None => lib.roots(),
    })?;
    if roots.is_empty() {
        return Ok(json!({ "ok": false, "error": "还没有添加照片目录" }));
    }

    let cancel = pipeline::new_cancel();
    {
        let mut current = state.scan_cancel.lock().map_err(|e| e.to_string())?;
        if state.progress.is_running() {
            return Ok(json!({ "ok": false, "error": "已有扫描任务在进行中" }));
        }
        state.progress.reset();
        *current = Some(cancel.clone());
    }
    let st = state.inner().clone();
    std::thread::spawn(move || {
        let cfg = config::load_settings();
        let opts = RunOptions {
            reanalyze_all: req.reanalyze_all.unwrap_or(false),
            ..Default::default()
        };
        let result = st.with_lib(|lib| {
            pipeline::run_full(
                lib,
                &cfg,
                Some(&roots),
                &st.progress,
                Some(&cancel),
                &opts,
            )
        });
        match result {
            Ok(r) if r.error.is_empty() => {}
            Ok(r) => st.progress.finish(&r.error),
            Err(e) => st.progress.finish(&e),
        }
        if let Ok(mut current) = st.scan_cancel.lock() {
            *current = None;
        }
    });
    Ok(json!({ "ok": true }))
}

#[tauri::command]
pub fn scan_cancel(state: State<'_, AppState>) -> Result<Value, String> {
    let current = state.scan_cancel.lock().map_err(|e| e.to_string())?;
    let Some(cancel) = current.as_ref() else {
        return Ok(json!({ "ok": false, "error": "当前没有正在进行的扫描" }));
    };
    cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    Ok(json!({ "ok": true }))
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct FavoriteRequest {
    /// 单个 id（兼容旧调用）
    pub id: Option<i64>,
    /// 批量 id。给了就忽略 `id`
    pub ids: Option<Vec<i64>>,
    pub favorite: Option<bool>,
}

#[tauri::command]
pub fn set_favorite(state: State<'_, AppState>, req: FavoriteRequest) -> Result<Value, String> {
    state.with_lib(|lib| {
        // 批量优先
        if let Some(ids) = req.ids.as_ref().filter(|v| !v.is_empty()) {
            let mut changed = 0usize;
            let want = req.favorite.unwrap_or(true);
            for id in ids {
                if lib.photo(*id).is_some() {
                    lib.set_favorite(*id, want);
                    changed += 1;
                }
            }
            return json!({ "ok": true, "count": changed, "favorite": want });
        }

        let Some(id) = req.id else {
            return json!({ "ok": false, "error": "没有指定照片" });
        };
        let Some(row) = lib.photo(id) else {
            return json!({ "ok": false, "error": "没有这张照片" });
        };
        let want = req.favorite.unwrap_or(!row.is_favorite());
        lib.set_favorite(id, want);
        json!({ "ok": true, "id": id, "favorite": want, "count": 1 })
    })
}

#[tauri::command]
pub fn reclassify(state: State<'_, AppState>) -> Result<Value, String> {
    state.with_lib(|lib| {
        let cfg = config::load_settings();
        let (total, flagged) = lib.reclassify_flags(&cfg);
        json!({ "ok": true, "total": total, "flagged": flagged, "clean": total - flagged })
    })
}

#[tauri::command]
pub fn backfill_places(
    state: State<'_, AppState>,
    redo: Option<bool>,
) -> Result<Value, String> {
    state.progress.reset();
    let prog = state.progress.clone();
    // 引擎侧的进度回调签名是 Fn(phase, done, total, detail)
    let cb = Arc::new(move |phase: &str, done: usize, total: usize, detail: &str| {
        prog.update(phase, done, total, detail);
    }) as po_core::scanner::ProgressFn;
    let out = state.with_lib(|lib| {
        let r = classify::backfill_places(lib, Some(&cb), redo.unwrap_or(true));
        json!({ "total": r.total, "updated": r.updated })
    })?;
    state.progress.finish("");
    Ok(json!({ "ok": true, "total": out["total"], "updated": out["updated"] }))
}

#[tauri::command]
pub fn reset_index(
    state: State<'_, AppState>,
    keep_roots: Option<bool>,
) -> Result<Value, String> {
    let keep = keep_roots.unwrap_or(true);
    let out = state.with_lib(|lib| {
        let before = lib.query_count("SELECT COUNT(*) c FROM photos", &[]);
        lib.reset_index(keep);
        json!({
            "ok": true,
            "removed": before,
            "roots_kept": if keep { lib.roots() } else { Vec::new() },
        })
    })?;
    if let Ok(mut cache) = state.thumb_cache.lock() {
        cache.clear();
    }
    // 路径缓存也必须清：重置后 id 会指向别的照片
    if let Ok(mut pc) = state.path_cache.lock() {
        pc.clear();
    }
    Ok(out)
}

// ---------------------------------------------------------------- 重复清理

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct CleanDupsRequest {
    pub keep_all: Option<bool>,
    pub groups: Option<Vec<i64>>,
    pub dest: Option<String>,
    #[serde(rename = "delete")]
    pub delete: Option<bool>,
}

fn recycle_dir(dest: Option<String>) -> PathBuf {
    match dest.filter(|s| !s.trim().is_empty()) {
        Some(d) => PathBuf::from(d.trim()),
        None => config::app_dir().join("recycle"),
    }
}

/// 视频封面文件缺失时现场重建，并写回库。
///
/// 缩略图缓存目录属于「可丢弃的缓存」：用户清理磁盘、
/// 或者从旧版本升级时，库里的路径可能已经失效。
/// 这时不能直接返回 404（那样整片视频会一直显示占位图），
/// 应该重新抽一张帧，顺手把路径修正回库。
pub fn regenerate_video_cover(
    state: &AppState,
    id: Option<i64>,
    video: &Path,
) -> Option<String> {
    // 没有 id 说明是按 path 直接请求的（例如刚导出还没入库），
    // 生成到临时文件即可，不写库。
    let out = match id {
        Some(i) => config::thumb_dir().join(format!("video_{i}.jpg")),
        None => config::thumb_dir().join(format!(
            "video_tmp_{}.jpg",
            po_core::library::now_ts() as i64
        )),
    };
    if !po_core::video::extract_frame(video, &out, 480) {
        return None;
    }
    let saved = out.to_string_lossy().to_string();
    if let Some(i) = id {
        let _ = state.with_lib(|lib| lib.set_video_thumb(i, &saved));
    }
    Some(saved)
}

#[tauri::command]
pub fn set_dup_keeper(
    state: State<'_, AppState>,
    group_id: i64,
    photo_id: i64,
) -> Result<Value, String> {
    state.with_lib(|lib| {
        let ok = lib.set_dup_keeper(group_id, photo_id);
        json!({ "ok": ok })
    })
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct DeletePhotosRequest {
    pub ids: Vec<i64>,
    pub dest: Option<String>,
    pub permanent: Option<bool>,
    /// 用户已确认「整组一起删」，不再保底留一张
    pub allow_whole_group: Option<bool>,
}

#[tauri::command]
pub fn delete_photos(
    state: State<'_, AppState>,
    req: DeletePhotosRequest,
) -> Result<Value, String> {
    if req.ids.is_empty() {
        return Ok(json!({ "ok": false, "error": "没有选中照片" }));
    }
    let permanent = req.permanent.unwrap_or(false);
    let recycle = recycle_dir(req.dest.clone());
    state.with_lib(|lib| {
        let s = lib.delete_photos_to(
            &req.ids,
            &recycle,
            permanent,
            req.allow_whole_group.unwrap_or(false),
        );
        json!({
            "ok": true, "moved": s.moved, "removed": s.removed,
            "errors": s.errors, "freed": s.freed, "protected": s.protected,
            "dest": recycle.to_string_lossy(),
        })
    })
}

// ---------------------------------------------------------------- 修复

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct RestoreRequest {
    pub id: i64,
    pub preset: Option<String>,
    pub options: Option<Value>,
}

fn build_restore_options(preset: Option<&str>, custom: Option<&Value>) -> restore::Options {
    let mut opts = restore::preset_options(preset.unwrap_or("标准修复"));
    if let Some(Value::Object(map)) = custom {
        for (k, v) in map {
            opts.set(k, v);
        }
    }
    opts
}

fn gain_json(before: &imaging::Metrics, after: &imaging::Metrics) -> Value {
    let pct = |a: f64, b: f64| -> f64 { ((b / a.max(1e-6) - 1.0) * 1000.0).round() / 10.0 };
    json!({
        "contrast": pct(before.contrast, after.contrast),
        "detail": pct(before.blur, after.blur),
        "sepia": ((1.0 - after.sepia.max(0.0) / before.sepia.max(1e-6)) * 1000.0).round() / 10.0,
    })
}

#[tauri::command]
pub fn restore_apply(state: State<'_, AppState>, req: RestoreRequest) -> Result<Value, String> {
    let opts = build_restore_options(req.preset.as_deref(), req.options.as_ref());
    state.with_lib(|lib| {
        let Some(row) = lib.photo(req.id) else {
            return json!({ "error": "没有这张照片" });
        };
        let Some(res) = restore::enhance_file(Path::new(&row.path), &opts) else {
            return json!({ "error": "这张图无法解码" });
        };
        // 预览限制在 1400px 内，避免 base64 过大
        let preview = imaging::thumbnail(&res.image, 1400);
        let mut buf = Vec::new();
        let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 90);
        if enc.encode_image(&preview).is_err() {
            return json!({ "error": "预览编码失败" });
        }
        json!({
            "id": req.id,
            "image": format!("data:image/jpeg;base64,{}", base64_encode(&buf)),
            "before": metrics_json(&res.before),
            "after": metrics_json(&res.after),
            "gain": gain_json(&res.before, &res.after),
            "screen_like": res.screen_like,
        })
    })
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct RestoreBatchRequest {
    pub ids: Vec<i64>,
    pub dest: Option<String>,
    pub preset: Option<String>,
    pub options: Option<Value>,
}

#[tauri::command]
pub fn restore_batch(
    state: State<'_, AppState>,
    req: RestoreBatchRequest,
) -> Result<Value, String> {
    if req.ids.is_empty() {
        return Ok(json!({ "ok": false, "error": "没有选中照片" }));
    }
    let Some(dest) = req.dest.filter(|s| !s.trim().is_empty()) else {
        return Ok(json!({ "ok": false, "error": "请先选择输出文件夹" }));
    };
    let dest = PathBuf::from(dest.trim());
    let _ = std::fs::create_dir_all(&dest);
    let opts = build_restore_options(req.preset.as_deref(), req.options.as_ref());

    state.with_lib(|lib| {
        let (mut done, mut errors) = (0, 0);
        for id in &req.ids {
            let Some(row) = lib.photo(*id) else {
                errors += 1;
                continue;
            };
            let Some(res) = restore::enhance_file(Path::new(&row.path), &opts) else {
                errors += 1;
                continue;
            };
            let fname = row.file_name();
            let stem = Path::new(&fname)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("photo");
            let out = dest.join(format!("{stem}_修复.jpg"));
            if restore::save_jpeg(&res.image, &out, 95) {
                done += 1;
            } else {
                errors += 1;
            }
        }
        json!({ "ok": true, "done": done, "errors": errors, "dest": dest.to_string_lossy() })
    })
}

// ---------------------------------------------------------------- 整理

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct OrganizeRequest {
    pub scheme: Option<String>,
    pub template: Option<String>,
    pub parts: Option<Vec<String>>,
    pub favorite_dir: Option<bool>,
    pub favorite_in_regular: Option<bool>,
    pub dry_run: Option<bool>,
    pub dest: Option<String>,
    pub mode: Option<String>,
}

#[tauri::command]
pub fn organize(state: State<'_, AppState>, req: OrganizeRequest) -> Result<Value, String> {
    let scheme = req.scheme.clone().unwrap_or_else(|| "year_month".into());
    let template = req
        .template
        .clone()
        .unwrap_or_else(|| "{date}_{name}".into());
    let plan_opts = organizer::PlanOptions {
        scheme: &scheme,
        template: Some(&template),
        parts: req.parts.clone(),
        limit: 0,
        only_ids: None,
        favorite_dir: req.favorite_dir.unwrap_or(false),
        favorite_in_regular: req.favorite_in_regular.unwrap_or(true),
    };
    let dry_run = req.dry_run.unwrap_or(true);
    let dest = req.dest.clone().unwrap_or_default();
    let mode = req.mode.clone().unwrap_or_else(|| "copy".into());
    let path_cache = state.path_cache.clone();

    state.with_lib(|lib| {
        let plan = organizer::plan_organize(lib, &plan_opts);
        if dry_run {
            let preview: Vec<Value> = plan
                .iter()
                .take(500)
                .map(|(pid, rel)| {
                    let from = lib.photo(*pid).map(|r| r.path).unwrap_or_default();
                    json!({ "id": pid, "path": rel, "from": from })
                })
                .collect();
            return json!({ "plan": preview, "total": plan.len() });
        }
        if dest.trim().is_empty() {
            return json!({ "error": "未指定输出目录" });
        }
        let r = organizer::execute_organize(
            lib,
            Path::new(dest.trim()),
            &plan,
            organizer::Mode::parse(&mode),
            organizer::Conflict::Rename,
            true,
            None,
            None,
        );
        let out = json!({
            "done": r.done, "skipped": r.skipped, "errors": r.errors,
            "moved": r.moved, "total": r.total,
            "batch": r.batch, "dest": r.dest,
        });
        // move 模式会改 path，缓存必须失效
        if mode == "move" {
            if let Ok(mut pc) = path_cache.lock() {
                pc.clear();
            }
        }
        out
    })
}

// ---------------------------------------------------------------- 工具

pub fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// 标准 base64 编码
pub fn base64_encode(data: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0];
        let b1 = *chunk.get(1).unwrap_or(&0);
        let b2 = *chunk.get(2).unwrap_or(&0);
        let n = ((b0 as u32) << 16) | ((b1 as u32) << 8) | b2 as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

// ---------------------------------------------------------------- 图片服务

/// 生成缩略图字节（磁盘缓存 + 内存缓存）。
///
/// 同一张图 + 同一尺寸只允许一个线程解码，
/// 否则列表里同一张缩略图被多个 `<img>` 引用（或快速滚动重复请求）时，
/// 会并发解码同一张原图，白白浪费 CPU。
pub fn serve_thumb(state: &AppState, path: &Path, size: u32) -> Option<Vec<u8>> {
    let size = size.clamp(64, 4096);
    let key = (path.to_string_lossy().to_lowercase(), size);
    if let Ok(cache) = state.thumb_cache.lock() {
        if let Some(d) = cache.get(&key) {
            return Some(d);
        }
    }

    // 每 (path,size) 一把锁
    let lock = {
        let mut locks = state.decode_locks.lock().ok()?;
        if locks.len() > 4096 {
            // 只保留正在用的，避免无限增长
            locks.retain(|_, v| Arc::strong_count(v) > 1);
        }
        locks
            .entry(key.clone())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    };
    let _guard = lock.lock().ok()?;

    // 等锁期间可能已被别的线程生成好
    if let Ok(cache) = state.thumb_cache.lock() {
        if let Some(d) = cache.get(&key) {
            return Some(d);
        }
    }

    // 磁盘缓存沿用 Python 版的 v2_ 前缀 + thumb_key
    let mtime = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    let cache_path = config::thumb_dir().join(format!(
        "v2_{}.jpg",
        imaging::thumb_key(&path.to_string_lossy(), size, mtime)
    ));
    if let Ok(data) = std::fs::read(&cache_path) {
        // 完整性检查：必须是完整 JPEG
        if data.len() > 256 && data.starts_with(&[0xFF, 0xD8]) && data.ends_with(&[0xFF, 0xD9]) {
            if let Ok(mut cache) = state.thumb_cache.lock() {
                cache.put(key, data.clone());
            }
            return Some(data);
        }
    }

    let img = imaging::open_image(path)?;
    let thumb = imaging::thumbnail(&img, size);
    let mut buf = Vec::new();
    let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 84);
    if enc.encode_image(&thumb).is_err() {
        return None;
    }
    // 原子写盘，避免并发读到半个文件。
    // 临时文件名必须带线程标识：多线程同时为「不同图片」生成缩略图时，
    // 只用 pid 会互相覆盖临时文件，导致某些图永远写不进去。
    let _ = std::fs::create_dir_all(config::thumb_dir());
    let tid = format!("{:?}", std::thread::current().id());
    let tmp = cache_path.with_extension(format!("jpg.{}.tmp", tid.replace(['(', ')', ' '], "")));
    if std::fs::write(&tmp, &buf).is_ok() {
        // 目标可能已被别的线程写好，rename 覆盖是同花顺
        let _ = std::fs::rename(&tmp, &cache_path);
    }
    if let Ok(mut cache) = state.thumb_cache.lock() {
        cache.put(key, buf.clone());
    }
    Some(buf)
}

/// 按 id 或 path 解析出磁盘上的**源图**路径。
///
/// 优先级有讲究：只要给了 id，就先查库里的 `path`（原图），
/// 因为前端在 `row.thumb` 分支传进来的往往是 320px 的缓存缩略图，
/// 拿它当源图放大到 1200px 会糊成一片、甚至长条变形。
/// 只有原图不在了，才退回 thumb，最后才用显式传入的 path 兜底。
///
/// 性能：一次列表加载会有几百个缩略图请求同时进来，
/// 若每个都去抢 `Library` 的 Mutex 查一次库，就全串行化了。
/// 所以先查内存里的 id->path 缓存，只有 miss 才落库。
pub fn resolve_image_path(state: &AppState, id: Option<i64>, path: Option<&str>) -> Option<PathBuf> {
    if let Some(id) = id {
        // 1) 缓存命中（绝大多数请求走这里，完全不碰数据库锁）
        let cached = state
            .path_cache
            .lock()
            .ok()
            .and_then(|c| c.get(&id).cloned());
        let row_paths = match cached {
            Some((p, t)) => Some((p, t)),
            None => {
                let got = state.with_lib(|lib| lib.photo(id)).ok().flatten();
                got.map(|row| {
                    let entry = (row.path.clone(), row.thumb.clone().unwrap_or_default());
                    if let Ok(mut c) = state.path_cache.lock() {
                        // 简单上限，避免无限增长
                        if c.len() > 20_000 {
                            c.clear();
                        }
                        c.insert(id, entry.clone());
                    }
                    entry
                })
            }
        };

        if let Some((p, t)) = row_paths {
            if !p.is_empty() {
                let pb = PathBuf::from(&p);
                if pb.exists() {
                    return Some(pb);
                }
            }
            if !t.is_empty() {
                let tb = PathBuf::from(&t);
                if tb.exists() {
                    return Some(tb);
                }
            }
        }
    }
    // 2) 没有 id 或库里查不到，用显式传入的路径
    if let Some(p) = path.filter(|s| !s.is_empty()) {
        let pb = PathBuf::from(p);
        if pb.exists() {
            return Some(pb);
        }
    }
    None
}

// ---------------------------------------------------------------- 批量导出

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct ExportRequest {
    pub ids: Vec<i64>,
    pub dest: Option<String>,
    /// `copy`（默认）或 `move`
    pub mode: Option<String>,
}

/// 把选中的照片/视频按原名导出到指定目录（重名自动加序号，不覆盖）。
#[tauri::command]
pub fn export_photos(state: State<'_, AppState>, req: ExportRequest) -> Result<Value, String> {
    if req.ids.is_empty() {
        return Ok(json!({ "ok": false, "error": "没有选中照片" }));
    }
    let Some(dest) = req.dest.filter(|s| !s.trim().is_empty()) else {
        return Ok(json!({ "ok": false, "error": "请先选择输出文件夹" }));
    };
    let dest = PathBuf::from(dest.trim());
    if std::fs::create_dir_all(&dest).is_err() {
        return Ok(json!({ "ok": false, "error": "无法创建输出目录" }));
    }
    let move_mode = req.mode.as_deref() == Some("move");

    state.with_lib(|lib| {
        let (mut done, mut errors) = (0, 0);
        let mut freed = 0i64;
        let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();

        for id in &req.ids {
            let Some(row) = lib.photo(*id) else {
                errors += 1;
                continue;
            };
            let src = PathBuf::from(&row.path);
            if !src.is_file() {
                errors += 1;
                continue;
            }
            // 目标同名时加序号，绝不覆盖已有文件
            let out = util::unique_path(&dest.join(row.file_name()), &mut used);
            let ok = if move_mode {
                std::fs::rename(&src, &out).is_ok()
                    || (std::fs::copy(&src, &out).is_ok() && std::fs::remove_file(&src).is_ok())
            } else {
                std::fs::copy(&src, &out).is_ok()
            };
            if ok {
                done += 1;
                freed += row.size.unwrap_or(0);
                if move_mode {
                    lib.repoint_photo_path(&row.path, &out.to_string_lossy());
                    lib.mark_missing(*id, false);
                }
            } else {
                errors += 1;
            }
        }
        json!({
            "ok": true, "done": done, "errors": errors,
            "freed": freed, "dest": dest.to_string_lossy(),
        })
    })
}

// ---------------------------------------------------------------- 回收站

/// 列出回收站：被移到回收目录、尚未取回的文件。
#[tauri::command]
pub fn get_recycle(state: State<'_, AppState>) -> Result<Value, String> {
    state.with_lib(|lib| {
        // 顺手清掉「文件早就没了」的幽灵记录，
        // 免得它们让「全部取回」失败、或让计数对不上
        let _ = lib.purge_ghost_recycle_entries();
        let items: Vec<Value> = lib
            .recycle_entries()
            .into_iter()
            .map(|(log_id, src, dst, ts)| {
                let name = Path::new(&dst)
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default();
                let size = std::fs::metadata(&dst).map(|m| m.len() as i64).unwrap_or(0);
                let is_vid = util::is_video(Path::new(&dst));
                // 视频没有可实时解码的帧，缩略图走库里预生成的封面
                let thumb = if is_vid {
                    lib.photo_by_path(&dst).and_then(|r| r.thumb)
                } else {
                    None
                };
                json!({
                    "log_id": log_id,
                    "name": name,
                    "src": src,
                    "dst": dst,
                    "size": size,
                    "ts": ts,
                    "kind": if is_vid { "video" } else { "photo" },
                    "thumb": thumb,
                })
            })
            .collect();
        let total = items.len();
        let bytes: i64 = items.iter().map(|v| v["size"].as_i64().unwrap_or(0)).sum();
        json!({ "items": items, "total": total, "bytes": bytes })
    })
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct RecycleAction {
    pub log_ids: Option<Vec<i64>>,
    /// 为 true 时表示全部
    pub all: Option<bool>,
}

fn recycle_ids(lib: &po_core::library::Library, req: &RecycleAction) -> Vec<i64> {
    if req.all == Some(true) {
        return lib.recycle_entries().into_iter().map(|e| e.0).collect();
    }
    req.log_ids.clone().unwrap_or_default()
}

/// 从回收站取回（移回原路径并恢复索引）。
#[tauri::command]
pub fn restore_recycle(
    state: State<'_, AppState>,
    req: RecycleAction,
) -> Result<Value, String> {
    let ids = state.with_lib(|lib| recycle_ids(lib, &req))?;
    if ids.is_empty() {
        return Ok(json!({ "ok": false, "error": "没有可取回的文件" }));
    }
    state.with_lib(|lib| {
        let (mut done, mut errors) = (0, 0);
        let mut last_msg = String::new();
        for id in &ids {
            let (ok, _path, msg) = lib.restore_from_recycle(*id);
            if ok {
                done += 1;
            } else {
                errors += 1;
                if !msg.is_empty() {
                    last_msg = msg;
                }
            }
        }
        json!({ "ok": true, "done": done, "errors": errors, "message": last_msg })
    })
}

/// 永久删除回收站里的文件（不可恢复）。
#[tauri::command]
pub fn purge_recycle(state: State<'_, AppState>, req: RecycleAction) -> Result<Value, String> {
    let ids = state.with_lib(|lib| recycle_ids(lib, &req))?;
    if ids.is_empty() {
        return Ok(json!({ "ok": false, "error": "没有可删除的文件" }));
    }
    state.with_lib(|lib| {
        let (mut done, mut errors) = (0, 0);
        for id in &ids {
            let (ok, _) = lib.purge_recycle(*id);
            if ok {
                done += 1;
            } else {
                errors += 1;
            }
        }
        json!({ "ok": true, "done": done, "errors": errors })
    })
}

#[cfg(test)]
mod tests {    use super::*;

    /// 「全部取回」传的是空 ids + all=true，
    /// 必须能正确反序列化成「取全部」，否则按钮点了等于没反应。
    #[test]
    fn recycle_action_all_flag_parses() {
        let v: Value = serde_json::from_str(r#"{"log_ids":[],"all":true}"#).unwrap();
        let req: RecycleAction = serde_json::from_value(v).unwrap();
        assert_eq!(req.all, Some(true), "all 标志没被解析出来");
        assert_eq!(req.log_ids.as_deref(), Some(&[][..]));
    }

    /// all 缺失（undefined 被序列化成 null）时，不能悄悄退化成「取 0 个」。
    #[test]
    fn recycle_action_all_null_means_empty_selection() {
        let v: Value = serde_json::from_str(r#"{"log_ids":[],"all":null}"#).unwrap();
        let req: RecycleAction = serde_json::from_value(v).unwrap();
        assert_eq!(req.all, None);
        // 这种输入下 recycle_ids 会返回空 —— 所以前端必须保证 all 是明确的布尔值
        assert!(req.log_ids.as_deref().unwrap_or(&[]).is_empty());
    }

    /// 端到端：把文件移进回收站，再用 all=true 全部取回。
    #[test]
    fn restore_all_roundtrip() {
        use po_core::library::{Library, PhotoUpdate};
        let dir = std::env::temp_dir().join("po_cmd_recycle_all");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let recycle = dir.join("recycle");
        std::fs::create_dir_all(&recycle).unwrap();

        let mut lib = Library::open_memory().unwrap();
        let mut paths = Vec::new();
        for i in 0..3 {
            let p = dir.join(format!("p{i}.jpg"));
            std::fs::write(&p, b"x").unwrap();
            let mut u = PhotoUpdate::new(p.to_string_lossy().to_string());
            u.text("name", Some(&format!("p{i}.jpg")));
            lib.upsert_photo(&u).unwrap();
            paths.push(p);
        }
        let ids = lib.all_photos("", "id").iter().map(|r| r.id).collect::<Vec<_>>();
        lib.delete_photos_to(&ids, &recycle, false, false);
        assert_eq!(lib.recycle_entries().len(), 3);
        assert!(paths.iter().all(|p| !p.exists()));

        // 模拟 all=true 的取回
        let req = RecycleAction { log_ids: Some(vec![]), all: Some(true) };
        let to_restore = recycle_ids(&lib, &req);
        assert_eq!(to_restore.len(), 3, "all=true 应选中全部 3 个");
        for id in to_restore {
            let (ok, _, msg) = lib.restore_from_recycle(id);
            assert!(ok, "{msg}");
        }
        assert!(paths.iter().all(|p| p.exists()), "文件都应回到原位");
        assert!(lib.recycle_entries().is_empty());
    }

    #[test]
    fn recycle_action_partial_selection_parses() {
        let v: Value = serde_json::from_str(r#"{"log_ids":[3,7],"all":false}"#).unwrap();
        let req: RecycleAction = serde_json::from_value(v).unwrap();
        assert_eq!(req.all, Some(false));
        assert_eq!(req.log_ids, Some(vec![3, 7]));
    }

    /// 缩略图缓存失效时，视频封面应能现场重建，而不是永远 404。
    #[test]
    fn regenerate_cover_returns_none_for_missing_video() {
        let state = AppState::new();
        let got = regenerate_video_cover(
            &state,
            None,
            std::path::Path::new("no_such_video_xyz.mp4"),
        );
        assert!(got.is_none(), "源文件不存在时不该返回封面路径");
    }

    #[test]
    fn base64_matches_rfc4648() {
        assert_eq!(base64_encode(b"Man"), "TWFu");
        assert_eq!(base64_encode(b"Ma"), "TWE=");
        assert_eq!(base64_encode(b"M"), "TQ==");
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn urlencode_escapes_non_url_chars() {
        assert_eq!(urlencode(r"C:\a b\x.jpg"), "C%3A%5Ca%20b%5Cx.jpg");
        assert_eq!(urlencode("abc-._~"), "abc-._~");
        assert_eq!(urlencode("北京"), "%E5%8C%97%E4%BA%AC");
    }

    fn sample_dir() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../..")
            .join("源码/samples")
    }

    #[test]
    fn serve_thumb_produces_valid_jpeg() {
        let Ok(rd) = std::fs::read_dir(sample_dir()) else {
            return;
        };
        let mut files: Vec<_> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.is_file()
                    && matches!(
                        p.extension().and_then(|s| s.to_str()),
                        Some("jpg") | Some("JPG")
                    )
            })
            .collect();
        files.sort();
        let Some(first) = files.first() else { return };

        let state = AppState::new();
        for size in [160u32, 480, 1200] {
            let data = serve_thumb(&state, first, size).expect("应能生成缩略图");
            assert!(data.len() > 256, "size={size} 数据过小: {}", data.len());
            assert_eq!(&data[..2], &[0xFF, 0xD8], "size={size} 不是 JPEG 头");
            assert_eq!(
                &data[data.len() - 2..],
                &[0xFF, 0xD9],
                "size={size} JPEG 未完整结束"
            );
        }
    }

    #[test]
    fn serve_thumb_hits_memory_cache() {
        let Ok(rd) = std::fs::read_dir(sample_dir()) else {
            return;
        };
        let Some(first) = rd
            .flatten()
            .map(|e| e.path())
            .find(|p| p.is_file() && p.extension().and_then(|s| s.to_str()) == Some("jpg"))
        else {
            return;
        };
        let state = AppState::new();
        let a = serve_thumb(&state, &first, 320).unwrap();
        let b = serve_thumb(&state, &first, 320).unwrap();
        assert_eq!(a, b, "同参数应命中缓存并返回一致字节");
        // 缓存里应确实有这条
        let key = (first.to_string_lossy().to_lowercase(), 320u32);
        let guarded = state.thumb_cache.lock().unwrap();
        assert!(guarded.get(&key).is_some());
    }

    #[test]
    fn serve_thumb_missing_file_returns_none() {
        let state = AppState::new();
        assert!(serve_thumb(&state, std::path::Path::new("no_such_xyz.jpg"), 320).is_none());
    }

    #[test]
    fn resolve_path_by_direct_path() {
        let Ok(rd) = std::fs::read_dir(sample_dir()) else {
            return;
        };
        let Some(first) = rd
            .flatten()
            .map(|e| e.path())
            .find(|p| p.is_file() && p.extension().and_then(|s| s.to_str()) == Some("jpg"))
        else {
            return;
        };
        let state = AppState::new();
        let got = resolve_image_path(&state, None, Some(&first.to_string_lossy()));
        assert_eq!(got, Some(first));
    }

    #[test]
    fn resolve_path_missing_returns_none() {
        let state = AppState::new();
        assert!(resolve_image_path(&state, None, Some("nope_xyz.jpg")).is_none());
        assert!(resolve_image_path(&state, Some(999999), None).is_none());
    }

    #[test]
    fn resolve_prefers_original_over_thumbnail() {
        // 关键行为：给了 id 时，必须返回原图，而不是前端传进来的 thumb 缓存路径。
        // 用 320px 缩略图当源图放大，会糊成一片。
        let Ok(rd) = std::fs::read_dir(sample_dir()) else {
            return;
        };
        let Some(original) = rd
            .flatten()
            .map(|e| e.path())
            .find(|p| p.is_file() && p.extension().and_then(|s| s.to_str()) == Some("jpg"))
        else {
            return;
        };
        let thumb = std::env::temp_dir().join("po_fake_thumb_320.jpg");
        std::fs::write(&thumb, b"pretend thumbnail").unwrap();

        let state = AppState::new();
        let orig_s = original.to_string_lossy().to_string();
        let thumb_s = thumb.to_string_lossy().to_string();
        let pid = state
            .with_lib(|lib| {
                let mut u = po_core::library::PhotoUpdate::new(orig_s.clone());
                u.text("thumb", Some(&thumb_s));
                lib.upsert_photo(&u).ok()
            })
            .unwrap()
            .expect("应能插入照片");

        // 同时给了 id 和 thumb 路径：应优先原图
        let got = resolve_image_path(&state, Some(pid), Some(&thumb_s));
        assert_eq!(got.as_deref(), Some(original.as_path()), "应优先返回原图");

        let _ = std::fs::remove_file(&thumb);
    }

    #[test]
    fn resolve_falls_back_to_thumb_when_original_gone() {
        // 原图不在了，退回 thumb（能显示总比黑着好）
        let thumb = std::env::temp_dir().join("po_fallback_thumb.jpg");
        std::fs::write(&thumb, b"pretend thumbnail").unwrap();
        let thumb_s = thumb.to_string_lossy().to_string();

        let state = AppState::new();
        let pid = state
            .with_lib(|lib| {
                let mut u = po_core::library::PhotoUpdate::new("D:/gone/nowhere.jpg");
                u.text("thumb", Some(&thumb_s));
                lib.upsert_photo(&u).ok()
            })
            .unwrap()
            .expect("应能插入照片");

        let got = resolve_image_path(&state, Some(pid), Some(&thumb_s));
        assert_eq!(got, Some(thumb.clone()), "原图缺失时应退回缩略图");

        let _ = std::fs::remove_file(&thumb);
    }

    #[test]
    fn serve_thumb_from_original_is_not_upscaled_garbage() {
        // 从原图生成的 1200px 缩略图，应该比 320px 版本大（信息量更多）
        let Ok(rd) = std::fs::read_dir(sample_dir()) else {
            return;
        };
        let Some(original) = rd
            .flatten()
            .map(|e| e.path())
            .find(|p| p.is_file() && p.extension().and_then(|s| s.to_str()) == Some("jpg"))
        else {
            return;
        };
        let state = AppState::new();
        let small = serve_thumb(&state, &original, 320).expect("320 应成功");
        let large = serve_thumb(&state, &original, 1200).expect("1200 应成功");
        assert!(
            large.len() > small.len(),
            "1200px 缩略图应比 320px 信息更多: {} vs {}",
            large.len(),
            small.len()
        );
    }

    #[test]
    fn photo_json_has_expected_shape() {
        let row = po_core::library::PhotoRow {
            id: 7,
            path: "a.jpg".into(),
            name: Some("a.jpg".into()),
            width: Some(800),
            height: Some(600),
            quality_score: Some(72.5),
            favorite: Some(1),
            ..Default::default()
        };
        let v = photo_json(&row);
        assert_eq!(v["id"], 7);
        assert_eq!(v["width"], 800);
        assert_eq!(v["favorite"], 1);
        // missing/is_best 为 NULL 时要给 0，前端才好判断
        assert_eq!(v["missing"], 0);
        assert_eq!(v["is_best"], 0);
        assert_eq!(v["thumb_url"], "/thumb?id=7");
        assert_eq!(v["photo_url"], "/photo?id=7");
        // 未设的字段应是 null 而不是丢掉
        assert!(v.get("sha1").is_some());
        assert!(v["sha1"].is_null());
    }

    #[test]
    fn metrics_json_contains_all_keys() {
        let m = imaging::Metrics {
            width: 100,
            height: 50,
            contrast: 0.25,
            noise: 3.5,
            ..Default::default()
        };
        let v = metrics_json(&m);
        for k in [
            "width",
            "height",
            "megapixels",
            "aspect",
            "blur",
            "brightness",
            "contrast",
            "dark_ratio",
            "white_ratio",
            "saturation",
            "hue",
            "sepia",
            "color",
            "skin",
            "noise",
            "sharpness",
            "lap_var",
        ] {
            assert!(v.get(k).is_some(), "缺字段 {k}");
        }
        assert_eq!(v["contrast"], 0.25);
    }

    #[test]
    fn gain_json_handles_zero_baseline() {
        let before = imaging::Metrics {
            contrast: 0.0,
            blur: 0.0,
            sepia: 0.0,
            ..Default::default()
        };
        let after = imaging::Metrics {
            contrast: 0.2,
            blur: 100.0,
            sepia: 0.1,
            ..Default::default()
        };
        // 基线为 0 时不能除零，也不能 panic
        let g = gain_json(&before, &after);
        assert!(g["contrast"].is_number());
        assert!(g["detail"].is_number());
        assert!(g["sepia"].is_number());
    }

    #[test]
    fn truthy_parses_query_flags() {
        assert!(truthy(&Some("1".into())));
        assert!(truthy(&Some("true".into())));
        assert!(truthy(&Some("yes".into())));
        assert!(!truthy(&Some("0".into())));
        assert!(!truthy(&Some("false".into())));
        assert!(!truthy(&None));
    }

    #[test]
    fn split_keywords_basic() {
        assert_eq!(split_keywords("父母 北京"), vec!["父母", "北京"]);
        assert_eq!(split_keywords("  单个  "), vec!["单个"]);
        assert_eq!(split_keywords(""), Vec::<String>::new());
        assert_eq!(split_keywords("   "), Vec::<String>::new());
    }

    #[test]
    fn split_keywords_handles_fullwidth_space() {
        // 中文输入法下常见的全角空格
        assert_eq!(split_keywords("父母\u{3000}北京"), vec!["父母", "北京"]);
        // 制表符与换行也算空白
        assert_eq!(split_keywords("a\tb\nc"), vec!["a", "b", "c"]);
    }

    #[test]
    fn split_keywords_dedupes() {
        // 重复词不该生成两条相同条件
        assert_eq!(split_keywords("北京 北京 上海"), vec!["北京", "上海"]);
    }

    #[test]
    fn split_keywords_caps_at_eight() {
        let long = "1 2 3 4 5 6 7 8 9 10 11 12";
        let got = split_keywords(long);
        assert_eq!(got.len(), 8, "最多 8 个词，实际 {}", got.len());
        assert_eq!(got[0], "1");
        assert_eq!(got[7], "8");
    }

    #[test]
    fn split_keywords_keeps_single_word_intact() {
        // 单个词内部不拆（如「父母合影」是一个词）
        assert_eq!(split_keywords("父母合影"), vec!["父母合影"]);
        // 连字符也不是分隔符
        assert_eq!(split_keywords("1985-06-12"), vec!["1985-06-12"]);
    }

    #[test]
    fn restore_options_merge_preset_and_custom() {
        let custom = json!({"sharpen": false, "upscale": true});
        let o = build_restore_options(Some("仅调色"), Some(&custom));
        // 预设关闭的项保持关闭
        assert!(!o.crop);
        assert!(!o.denoise);
        // 自定义项覆盖预设
        assert!(!o.sharpen);
        assert!(o.upscale);
    }

    #[test]
    fn restore_options_unknown_preset_falls_back() {
        let o = build_restore_options(Some("不存在的预设"), None);
        // 退回默认（标准修复）的开关组合
        assert!(o.crop && o.deskew && o.denoise && o.sharpen);
    }
}
