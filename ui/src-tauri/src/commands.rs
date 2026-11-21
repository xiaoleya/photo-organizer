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
}

impl AppState {
    pub fn new() -> Self {
        Self {
            lib: Arc::new(Mutex::new(None)),
            progress: Progress::new(),
            thumb_cache: Arc::new(Mutex::new(ThumbCache::new())),
            path_cache: Arc::new(Mutex::new(HashMap::new())),
            decode_locks: Arc::new(Mutex::new(HashMap::new())),
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
    pub nodate: Option<String>,
    pub dup_group: Option<i64>,
    pub fav: Option<String>,
    pub year: Option<i64>,
    pub month: Option<i64>,
    pub place: Option<String>,
    pub noplace: Option<String>,
    pub min_quality: Option<f64>,
    pub max_quality: Option<f64>,
    pub q: Option<String>,
    pub sort: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

fn truthy(s: &Option<String>) -> bool {
    matches!(s.as_deref(), Some("1") | Some("true") | Some("yes"))
}

#[tauri::command]
pub fn get_photos(state: State<'_, AppState>, query: PhotoQuery) -> Result<Value, String> {
    state.with_lib(|lib| {
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
        if let Some(m) = query.month {
            where_parts.push("month = ?".into());
            args.push(m.to_string());
        }
        if let Some(p) = query.place.as_deref().filter(|s| !s.is_empty()) {
            where_parts.push("place = ?".into());
            args.push(p.to_string());
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
            where_parts.push("(name LIKE ? OR path LIKE ? OR place LIKE ?)".into());
            let kw = format!("%{kw}%");
            args.extend([kw.clone(), kw.clone(), kw]);
        }

        let order = match query.sort.as_deref() {
            Some("dt DESC") => "dt IS NULL, dt DESC, id DESC",
            Some("quality_score DESC") => "quality_score DESC, id",
            Some("quality_score ASC") => "quality_score ASC, id",
            Some("age_score DESC") => "age_score DESC, id",
            _ => "dt IS NULL, dt ASC, id",
        };

        let limit = query.limit.unwrap_or(200).clamp(1, 2000);
        let offset = query.offset.unwrap_or(0).max(0);
        let w = where_parts.join(" AND ");
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
pub fn get_tags(state: State<'_, AppState>) -> Result<Value, String> {
    state.with_lib(|lib| {
        let items: Vec<Value> = lib
            .all_tags()
            .into_iter()
            .map(|(name, kind, count)| {
                let group = match kind.as_str() {
                    "auto" => {
                        if matches!(
                            name.as_str(),
                            "模糊"
                                | "过暗"
                                | "过亮"
                                | "低分辨率"
                                | "褪色平淡"
                                | "泛黄/棕褐"
                                | "噪点重"
                                | "高光过曝"
                                | "大面积死黑"
                                | "灰白/单色"
                        ) {
                            "质量"
                        } else if name.starts_with("相机:") {
                            "相机"
                        } else {
                            "特征"
                        }
                    }
                    "event" => "事件",
                    "time" => "时间",
                    _ => "自定义",
                };
                json!({ "name": name, "kind": kind, "count": count, "group": group })
            })
            .collect();
        json!({ "items": items })
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
pub fn get_events(state: State<'_, AppState>) -> Result<Value, String> {
    state.with_lib(|lib| {
        let items: Vec<Value> = lib
            .all_events()
            .into_iter()
            .map(|(id, title, start, end, count, cover)| {
                json!({
                    "id": id, "title": title, "start_dt": start,
                    "end_dt": end, "count": count, "cover_id": cover,
                })
            })
            .collect();
        json!({ "items": items })
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
#[tauri::command]
pub fn get_startup_state(state: State<'_, AppState>) -> Result<Value, String> {
    state.with_lib(|lib| {
        let total = lib.query_count("SELECT COUNT(*) c FROM photos", &[]);
        let analyzed = lib.query_count(
            "SELECT COUNT(*) c FROM photos WHERE analyzed_at IS NOT NULL",
            &[],
        );
        let roots = lib.roots().len();
        // 上次扫描时间：优先用 meta 里记录的，其次取库里最大的 analyzed_at
        let last_scan = lib
            .get_meta("last_scan_at")
            .and_then(|s| s.parse::<f64>().ok())
            .or_else(|| {
                let v: f64 = lib
                    .conn()
                    .query_row(
                        "SELECT COALESCE(MAX(analyzed_at),0) FROM photos",
                        [],
                        |r| r.get(0),
                    )
                    .unwrap_or(0.0);
                (v > 0.0).then_some(v)
            })
            .unwrap_or(0.0);
        json!({
            "total": total,
            "analyzed": analyzed,
            "roots": roots,
            "last_scan": last_scan,
            // 有内容才值得问；空库不问
            "has_previous": total > 0,
        })
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
}

#[tauri::command]
pub fn scan_start(state: State<'_, AppState>, req: ScanRequest) -> Result<Value, String> {
    if state.progress.is_running() {
        return Ok(json!({ "ok": false, "error": "已有扫描任务在进行中" }));
    }
    let roots = state.with_lib(|lib| match req.roots.filter(|r| !r.is_empty()) {
        Some(r) => r,
        None => lib.roots(),
    })?;
    if roots.is_empty() {
        return Ok(json!({ "ok": false, "error": "还没有添加照片目录" }));
    }

    state.progress.reset();
    let st = state.inner().clone();
    std::thread::spawn(move || {
        let cfg = config::load_settings();
        let opts = RunOptions {
            // 每次「开始扫描」都按当前实验室开关重算
            reanalyze_all: true,
            ..Default::default()
        };
        let result = st.with_lib(|lib| {
            pipeline::run_full(lib, &cfg, Some(&roots), &st.progress, None, &opts)
        });
        match result {
            Ok(r) if r.error.is_empty() => {}
            Ok(r) => st.progress.finish(&r.error),
            Err(e) => st.progress.finish(&e),
        }
    });
    Ok(json!({ "ok": true }))
}

#[tauri::command]
pub fn scan_sync(state: State<'_, AppState>, req: ScanRequest) -> Result<Value, String> {
    state.progress.reset();
    let prog = state.progress.clone();
    let out = state.with_lib(|lib| {
        let cfg = config::load_settings();
        let r = pipeline::run_full(
            lib,
            &cfg,
            req.roots.as_deref(),
            &prog,
            None,
            &RunOptions::default(),
        );
        serde_json::to_value(&r).unwrap_or(json!({}))
    })?;
    Ok(out)
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct FavoriteRequest {
    pub id: i64,
    pub favorite: Option<bool>,
}

#[tauri::command]
pub fn set_favorite(state: State<'_, AppState>, req: FavoriteRequest) -> Result<Value, String> {
    state.with_lib(|lib| {
        let Some(row) = lib.photo(req.id) else {
            return json!({ "ok": false, "error": "没有这张照片" });
        };
        let want = req.favorite.unwrap_or(!row.is_favorite());
        lib.set_favorite(req.id, want);
        json!({ "ok": true, "id": req.id, "favorite": want })
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

#[tauri::command]
pub fn clean_duplicates(
    state: State<'_, AppState>,
    req: CleanDupsRequest,
) -> Result<Value, String> {
    let keep_all = req.keep_all.unwrap_or(false);
    let group_ids = req.groups.unwrap_or_default();
    if !keep_all && group_ids.is_empty() {
        return Ok(json!({ "ok": false, "error": "未选择要清理的重复组" }));
    }
    let permanent = req.delete.unwrap_or(false);
    let recycle = recycle_dir(req.dest.clone());
    state.with_lib(|lib| {
        let s = lib.clean_duplicate_copies(&group_ids, keep_all, &recycle, permanent);
        json!({
            "ok": true,
            "moved": s.moved, "removed": s.removed, "errors": s.errors,
            "freed": s.freed, "dest": recycle.to_string_lossy(),
            "permanent": permanent,
        })
    })
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct DeletePhotosRequest {
    pub ids: Vec<i64>,
    pub dest: Option<String>,
    pub permanent: Option<bool>,
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
        let s = lib.delete_photos_to(&req.ids, &recycle, permanent);
        json!({
            "ok": true, "moved": s.moved, "removed": s.removed,
            "errors": s.errors, "freed": s.freed,
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
pub fn restore_one(state: State<'_, AppState>, req: RestoreRequest) -> Result<Value, String> {
    let opts = build_restore_options(req.preset.as_deref(), req.options.as_ref());
    state.with_lib(|lib| {
        let Some(row) = lib.photo(req.id) else {
            return json!({ "error": "没有这张照片" });
        };
        let Some(res) = restore::enhance_file(Path::new(&row.path), &opts) else {
            return json!({ "error": "这张图无法解码" });
        };
        let out_path = config::app_dir().join(format!("restored_{}.jpg", req.id));
        let saved = restore::save_jpeg(&res.image, &out_path, 95);
        json!({
            "id": req.id,
            "path": out_path.to_string_lossy(),
            "path_url": format!("/photo?path={}", urlencode(&out_path.to_string_lossy())),
            "before": metrics_json(&res.before),
            "after": metrics_json(&res.after),
            "gain": gain_json(&res.before, &res.after),
            "ok": saved,
        })
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
                .map(|(pid, rel)| json!({ "id": pid, "path": rel }))
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

#[allow(dead_code)]
fn unused(_: &util::Precision) {}

#[cfg(test)]
mod tests {
    use super::*;

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
