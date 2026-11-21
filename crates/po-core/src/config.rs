//! 路径与默认参数。与 Python 版 `opo/config.py` 行为保持一致。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const APP_DIR_NAME: &str = "OldPhotoOrganizer";

/// 应用数据目录。
///
/// 优先级：`OPO_HOME` 环境变量 > Windows 的 `LOCALAPPDATA` > 用户主目录。
pub fn app_dir() -> PathBuf {
    if let Some(base) = std::env::var_os("OPO_HOME") {
        if !base.is_empty() {
            return ensure_dir(PathBuf::from(base));
        }
    }
    if let Some(base) = std::env::var_os("LOCALAPPDATA") {
        if !base.is_empty() {
            return ensure_dir(PathBuf::from(base).join(APP_DIR_NAME));
        }
    }
    if let Some(home) = std::env::var_os("USERPROFILE") {
        if !home.is_empty() {
            return ensure_dir(PathBuf::from(home).join(APP_DIR_NAME));
        }
    }
    ensure_dir(PathBuf::from(APP_DIR_NAME))
}

/// 确保目录存在，且不因并发创建而失败。
pub fn ensure_dir(p: PathBuf) -> PathBuf {
    match std::fs::create_dir_all(&p) {
        Ok(()) => p,
        // 已存在（或另一个线程刚建好）都视为成功
        Err(_) if p.is_dir() => p,
        Err(_) => p,
    }
}

pub fn db_path() -> PathBuf {
    app_dir().join("library.db")
}

pub fn thumb_dir() -> PathBuf {
    ensure_dir(app_dir().join("thumbs"))
}

pub fn log_dir() -> PathBuf {
    ensure_dir(app_dir().join("logs"))
}

pub fn settings_path() -> PathBuf {
    app_dir().join("settings.json")
}

/// 设置项的值。JSON 里既有整数也有浮点，统一用这个枚举承载。
#[derive(Debug, Clone, PartialEq)]
pub enum Setting {
    Float(f64),
    Int(i64),
    /// 布尔在 Python 里是 int 的子类，单独保留以免写回时改变语义
    Bool(bool),
    Str(String),
}

impl Setting {
    pub fn as_f64(&self) -> f64 {
        match self {
            Setting::Float(v) => *v,
            Setting::Int(v) => *v as f64,
            Setting::Bool(v) => {
                if *v {
                    1.0
                } else {
                    0.0
                }
            }
            Setting::Str(s) => s.parse().unwrap_or(0.0),
        }
    }

    pub fn as_i64(&self) -> i64 {
        self.as_f64() as i64
    }

    pub fn as_bool(&self) -> bool {
        match self {
            Setting::Bool(v) => *v,
            Setting::Int(v) => *v != 0,
            Setting::Float(v) => *v != 0.0,
            Setting::Str(s) => !s.is_empty() && s != "0" && !s.eq_ignore_ascii_case("false"),
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Setting::Str(s) => Some(s),
            _ => None,
        }
    }

    /// 保持原设置项的类型，把新值写进去（对应 Python 的 int/float 强转）
    fn coerce_like(&self, incoming: &serde_json::Value) -> Option<Setting> {
        match self {
            Setting::Str(_) => incoming.as_str().map(|s| Setting::Str(s.to_string())),
            Setting::Int(_) => incoming
                .as_i64()
                .or_else(|| incoming.as_f64().map(|f| f as i64))
                .map(Setting::Int),
            Setting::Bool(_) => incoming
                .as_bool()
                .or_else(|| incoming.as_i64().map(|i| i != 0))
                .map(Setting::Bool),
            Setting::Float(_) => incoming.as_f64().map(Setting::Float),
        }
    }

    fn to_json(&self) -> serde_json::Value {
        match self {
            Setting::Float(v) => serde_json::Number::from_f64(*v)
                .map(serde_json::Value::Number)
                .unwrap_or_else(|| serde_json::Value::from(0)),
            Setting::Int(v) => serde_json::Value::from(*v),
            Setting::Bool(v) => serde_json::Value::from(*v),
            Setting::Str(s) => serde_json::Value::from(s.as_str()),
        }
    }
}

/// 设置集合。顺序无关，键名与 Python 版逐字一致。
#[derive(Debug, Clone)]
pub struct Settings {
    values: HashMap<String, Setting>,
}

impl Default for Settings {
    fn default() -> Self {
        Self::new()
    }
}

impl Settings {
    pub fn new() -> Self {
        let mut values = HashMap::new();
        // 质量自适应阈值
        values.insert("blur_threshold".into(), Setting::Float(90.0));
        values.insert("dark_threshold".into(), Setting::Float(0.16));
        values.insert("bright_threshold".into(), Setting::Float(0.86));
        values.insert("white_threshold".into(), Setting::Float(0.30));
        values.insert("lowres_mp".into(), Setting::Float(0.70));
        values.insert("faded_contrast".into(), Setting::Float(0.135));
        values.insert("faded_saturation".into(), Setting::Float(0.150));
        values.insert("sepia_threshold".into(), Setting::Float(0.45));
        values.insert("skin_threshold".into(), Setting::Float(0.055));
        // 事件聚类
        values.insert("event_gap_hours".into(), Setting::Float(6.0));
        values.insert("event_gps_km".into(), Setting::Float(50.0));
        // 去重
        values.insert("phash_distance".into(), Setting::Int(14));
        values.insert("dhash_distance".into(), Setting::Int(8));
        values.insert("dedup_size_ratio".into(), Setting::Float(6.0));
        // 缩略图
        values.insert("thumb_size".into(), Setting::Int(320));
        values.insert("thumb_quality".into(), Setting::Int(82));
        values.insert("analyze_workers".into(), Setting::Int(0));
        values.insert(
            "filename_template".into(),
            Setting::Str("{date}_{time}_{name}".into()),
        );
        // 实验室功能（默认关闭）
        values.insert("lab_quality".into(), Setting::Bool(false));
        values.insert("lab_restore".into(), Setting::Bool(false));
        // 已转正常驻，保留键位仅用于兼容旧配置
        values.insert("lab_duplicates".into(), Setting::Bool(true));
        values.insert("lab_organize".into(), Setting::Bool(true));
        values.insert("lab_favorites".into(), Setting::Bool(true));
        values.insert("lab_places".into(), Setting::Bool(true));
        Self { values }
    }

    pub fn get(&self, key: &str) -> Option<&Setting> {
        self.values.get(key)
    }

    pub fn f64(&self, key: &str) -> f64 {
        self.values.get(key).map(|s| s.as_f64()).unwrap_or(0.0)
    }

    pub fn i64(&self, key: &str) -> i64 {
        self.values.get(key).map(|s| s.as_i64()).unwrap_or(0)
    }

    pub fn usize(&self, key: &str) -> usize {
        let v = self.i64(key);
        if v < 0 {
            0
        } else {
            v as usize
        }
    }

    pub fn bool(&self, key: &str) -> bool {
        self.values.get(key).map(|s| s.as_bool()).unwrap_or(false)
    }

    pub fn string(&self, key: &str) -> Option<&str> {
        self.values.get(key).and_then(|s| s.as_str())
    }

    pub fn contains(&self, key: &str) -> bool {
        self.values.contains_key(key)
    }

    /// 合并外部写入设置；未知键直接忽略。返回是否有改动。
    pub fn merge_json(&mut self, incoming: &serde_json::Map<String, serde_json::Value>) -> bool {
        let mut changed = false;
        for (k, v) in incoming {
            let Some(current) = self.values.get(k) else {
                continue;
            };
            if let Some(coerced) = current.coerce_like(v) {
                self.values.insert(k.clone(), coerced);
                changed = true;
            }
        }
        changed
    }

    pub fn to_json_map(&self) -> serde_json::Map<String, serde_json::Value> {
        self.values
            .iter()
            .map(|(k, v)| (k.clone(), v.to_json()))
            .collect()
    }

    pub fn to_json_string(&self) -> String {
        serde_json::to_string_pretty(&serde_json::Value::Object(self.to_json_map()))
            .unwrap_or_else(|_| "{}".into())
    }

    pub fn keys(&self) -> impl Iterator<Item = &String> {
        self.values.keys()
    }
}

/// 读取设置。文件缺失或损坏时退回默认值，与 Python 版一致。
pub fn load_settings() -> Settings {
    load_settings_from(&settings_path())
}

pub fn load_settings_from(path: &Path) -> Settings {
    let mut s = Settings::new();
    let Ok(text) = std::fs::read_to_string(path) else {
        return s;
    };
    let Ok(serde_json::Value::Object(map)) = serde_json::from_str(&text) else {
        return s;
    };
    // 用「覆盖」语义：只接受已知键，未知键忽略
    let mut merged = serde_json::Map::new();
    for (k, v) in map {
        if s.contains(&k) {
            merged.insert(k, v);
        }
    }
    s.merge_json(&merged);
    s
}

pub fn save_settings(s: &Settings) -> bool {
    save_settings_to(s, &settings_path())
}

pub fn save_settings_to(s: &Settings, path: &Path) -> bool {
    if let Some(parent) = path.parent() {
        ensure_dir(parent.to_path_buf());
    }
    // 先写临时文件再原子替换，避免写入中断留下半个 JSON
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, s.to_json_string()).is_err() {
        return false;
    }
    match std::fs::rename(&tmp, path) {
        Ok(()) => true,
        Err(_) => {
            let _ = std::fs::remove_file(&tmp);
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_python() {
        let s = Settings::new();
        assert!((s.f64("blur_threshold") - 90.0).abs() < 1e-9);
        assert!((s.f64("faded_saturation") - 0.150).abs() < 1e-9);
        assert_eq!(s.i64("phash_distance"), 14);
        assert_eq!(s.i64("thumb_size"), 320);
        assert!(!s.bool("lab_quality"));
        assert!(s.bool("lab_places"));
        assert_eq!(s.string("filename_template"), Some("{date}_{time}_{name}"));
    }

    #[test]
    fn int_and_float_coercion() {
        let mut s = Settings::new();
        let mut m = serde_json::Map::new();
        // 浮点键收到整数，应存成浮点
        m.insert("blur_threshold".into(), serde_json::json!(80));
        // 整数键收到浮点，应截断成整数
        m.insert("phash_distance".into(), serde_json::json!(10.7));
        assert!(s.merge_json(&m));
        assert!((s.f64("blur_threshold") - 80.0).abs() < 1e-9);
        assert_eq!(s.i64("phash_distance"), 10);
    }

    #[test]
    fn unknown_keys_ignored() {
        let mut s = Settings::new();
        let mut m = serde_json::Map::new();
        m.insert("no_such_setting".into(), serde_json::json!(1));
        assert!(!s.merge_json(&m));
        assert!(!s.contains("no_such_setting"));
    }

    #[test]
    fn roundtrip_through_json() {
        let mut s = Settings::new();
        let mut m = serde_json::Map::new();
        m.insert("blur_threshold".into(), serde_json::json!(120.5));
        m.insert("lab_quality".into(), serde_json::json!(true));
        s.merge_json(&m);

        let text = s.to_json_string();
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert!((parsed["blur_threshold"].as_f64().unwrap() - 120.5).abs() < 1e-9);
        assert_eq!(parsed["lab_quality"].as_bool(), Some(true));
    }

    #[test]
    fn bool_from_legacy_int() {
        // 旧配置里实验室开关存的是 0/1
        let mut s = Settings::new();
        let mut m = serde_json::Map::new();
        m.insert("lab_quality".into(), serde_json::json!(1));
        s.merge_json(&m);
        assert!(s.bool("lab_quality"));
    }
}
