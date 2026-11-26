//! 通用工具：哈希、文件名安全化、日期解析、格式化。
//! 与 Python 版 `opo/util.py` 行为对齐。

use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use chrono::{DateTime, NaiveDate, NaiveDateTime};
use regex::Regex;
use sha1::{Digest, Sha1};

pub const IMG_EXTS: &[&str] = &[
    "jpg", "jpeg", "jpe", "jfif", "png", "bmp", "gif", "tif", "tiff", "webp", "heic", "heif",
    "avif",
];

/// 视频扩展名。这些文件会进图库，但只做抽帧/时长，不做像素质量分析。
pub const VIDEO_EXTS: &[&str] = &[
    "mp4", "mov", "m4v", "avi", "mkv", "wmv", "flv", "webm", "3gp", "mpg", "mpeg", "mts", "m2ts",
];

pub const RAW_EXTS: &[&str] = &["cr2", "cr3", "nef", "arw", "dng", "raf", "orf", "rw2"];

/// 1826 年涅普斯第一张永久照片，作为合理下界
pub const MIN_YEAR: i32 = 1826;

/// 取小写扩展名（不含点）。无扩展名返回空串。
pub fn ext_lower(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default()
}

/// 带点的扩展名，小写，如 `.jpg`
pub fn ext_dot_lower(path: &Path) -> String {
    let e = ext_lower(path);
    if e.is_empty() {
        String::new()
    } else {
        format!(".{e}")
    }
}

pub fn is_image(path: &Path) -> bool {
    let e = ext_lower(path);
    IMG_EXTS.iter().any(|x| *x == e)
}

pub fn is_video(path: &Path) -> bool {
    let e = ext_lower(path);
    VIDEO_EXTS.iter().any(|x| *x == e)
}

/// 该不该进图库：照片与视频都要，RAW 暂不支持解码所以排除。
pub fn is_media(path: &Path) -> bool {
    is_image(path) || is_video(path)
}

/// 流式计算文件 sha1。
pub fn sha1_file(path: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut hasher = Sha1::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

pub fn sha1_text(text: &str) -> String {
    let mut hasher = Sha1::new();
    hasher.update(text.as_bytes());
    hex(&hasher.finalize())
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

static ILLEGAL: OnceLock<Regex> = OnceLock::new();
static WS: OnceLock<Regex> = OnceLock::new();

fn illegal_re() -> &'static Regex {
    ILLEGAL.get_or_init(|| Regex::new(r#"[\\/:*?"<>|\r\n\t]"#).unwrap())
}

fn ws_re() -> &'static Regex {
    WS.get_or_init(|| Regex::new(r"\s+").unwrap())
}

/// 去掉 Windows 非法字符，压缩空白，限制长度（保留扩展名）。
pub fn safe_name(s: &str, maxlen: usize) -> String {
    let replaced = illegal_re().replace_all(s, "_");
    // Python 的 strip(" .") 只去掉两端空格和点
    let trimmed = replaced.trim_matches(|c| c == ' ' || c == '.');
    let collapsed = ws_re().replace_all(trimmed, " ");
    let mut out = collapsed.into_owned();
    if out.is_empty() {
        out = "untitled".into();
    }
    if out.chars().count() > maxlen {
        let p = Path::new(&out);
        let ext = p
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| format!(".{e}"))
            .unwrap_or_default();
        if !ext.is_empty() && ext.chars().count() <= 12 {
            let stem = p
                .file_stem()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_string();
            let keep = maxlen.saturating_sub(ext.chars().count());
            let stem: String = stem.chars().take(keep).collect();
            let stem = stem.trim_end_matches(|c| c == ' ' || c == '.');
            out = format!("{stem}{ext}");
        } else {
            let s: String = out.chars().take(maxlen).collect();
            out = s.trim_end_matches(|c| c == ' ' || c == '.').to_string();
        }
    }
    out
}

/// 人类可读的体积，如 `1.2 MB`。
pub fn human_size(n: f64) -> String {
    if !n.is_finite() {
        return "-".into();
    }
    let mut v = n;
    for unit in ["B", "KB", "MB", "GB", "TB"] {
        if v < 1024.0 || unit == "TB" {
            return if unit == "B" {
                format!("{:.0} {}", v, unit)
            } else {
                format!("{:.1} {}", v, unit)
            };
        }
        v /= 1024.0;
    }
    "-".into()
}

pub fn human_size_i(n: i64) -> String {
    human_size(n as f64)
}

/// 同目录下生成不冲突的路径：`name.jpg` -> `name-1.jpg`。
pub fn unique_path(path: &Path, taken: &mut HashSet<String>) -> PathBuf {
    let mut cand = path.to_path_buf();
    if !cand.exists() && !taken.contains(&cand.to_string_lossy().to_lowercase()) {
        taken.insert(cand.to_string_lossy().to_lowercase());
        return cand;
    }
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| format!(".{s}"))
        .unwrap_or_default();
    let dir = path.parent().unwrap_or(Path::new("."));
    let mut i = 1u32;
    loop {
        cand = dir.join(format!("{stem}-{i}{ext}"));
        let key = cand.to_string_lossy().to_lowercase();
        if !cand.exists() && !taken.contains(&key) {
            taken.insert(key);
            return cand;
        }
        i += 1;
    }
}

// ---------------------------------------------------------------- 日期解析

/// 解析精度
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Precision {
    Second,
    Day,
    Month,
    Year,
}

impl Precision {
    pub fn as_str(&self) -> &'static str {
        match self {
            Precision::Second => "second",
            Precision::Day => "day",
            Precision::Month => "month",
            Precision::Year => "year",
        }
    }

    pub fn from_str_opt(s: &str) -> Option<Self> {
        match s {
            "second" => Some(Precision::Second),
            "day" => Some(Precision::Day),
            "month" => Some(Precision::Month),
            "year" => Some(Precision::Year),
            _ => None,
        }
    }
}

struct DateRes {
    date_re: Regex,
    ts_re: Regex,
    compact_re: Regex,
    year_only_re: Regex,
}

fn date_res() -> &'static DateRes {
    static RE: OnceLock<DateRes> = OnceLock::new();
    RE.get_or_init(|| DateRes {
        // 原本两端是 (?<!\d) ... (?!\d)，但 regex crate 不支持 look-around，
        // 改成匹配后再用 is_digit_boundary 校验。
        date_re: Regex::new(
            r"(?x)
            (18\d{2}|19\d{2}|20\d{2})
            (?:[-_.年/]?(\d{1,2})[-_.月/]?(\d{1,2})日?)?
            (?:[ _T\-]?(\d{1,2})[时:._\-]?(\d{1,2})[分:._\-]?(\d{1,2})秒?)?
            ",
        )
        .unwrap(),
        ts_re: Regex::new(
            r"(18\d{2}|19\d{2}|20\d{2})(\d{2})(\d{2})[_\-. T]?(\d{2})(\d{2})(\d{2})",
        )
        .unwrap(),
        compact_re: Regex::new(r"(18\d{2}|19\d{2}|20\d{2})(\d{2})(\d{2})").unwrap(),
        year_only_re: Regex::new(r"^(18\d{2}|19\d{2}|20\d{2})$").unwrap(),
    })
}

/// 匹配区间两端是否都不是数字（等价于 Python 的 `(?<!\d)` / `(?!\d)`）。
///
/// 数字全是 ASCII，直接按字节判断即可；前后若落在 UTF-8 多字节字符内部，
/// 那些字节一定 >= 0x80，不会被误判成数字。
fn is_digit_boundary(text: &str, start: usize, end: usize) -> bool {
    let bytes = text.as_bytes();
    let before_ok = start == 0 || !bytes[start - 1].is_ascii_digit();
    let after_ok = end >= bytes.len() || !bytes[end].is_ascii_digit();
    before_ok && after_ok
}

/// 校验并构造时间；不合理返回 None。
fn valid(
    y: i32,
    mo: Option<i32>,
    d: Option<i32>,
    h: i32,
    mi: i32,
    s: i32,
) -> Option<NaiveDateTime> {
    let now_year = chrono::Local::now().date_naive().year_ce();
    let max_year = now_year as i32 + 1;
    if y < MIN_YEAR || y > max_year {
        return None;
    }
    let mo = mo.unwrap_or(1);
    let d = d.unwrap_or(1);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) {
        return None;
    }
    if !(0..=23).contains(&h) || !(0..=59).contains(&mi) || !(0..=59).contains(&s) {
        return None;
    }
    NaiveDate::from_ymd_opt(y, mo as u32, d as u32)?.and_hms_opt(h as u32, mi as u32, s as u32)
}

trait YearCe {
    fn year_ce(&self) -> i32;
}

impl YearCe for NaiveDate {
    fn year_ce(&self) -> i32 {
        use chrono::Datelike;
        self.year()
    }
}

/// 从文件名 / 目录名里推断拍摄时间。
///
/// 优先级：紧凑时间戳 > 完整日期时间 > 紧凑日期 > 年月日（含中文） > 年。
pub fn parse_datetime(text: &str) -> (Option<NaiveDateTime>, Option<Precision>) {
    if text.is_empty() {
        return (None, None);
    }
    let name = Path::new(text)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(text);
    let res = date_res();

    if let Some(c) = res.ts_re.captures(name) {
        let y = c[1].parse::<i32>().unwrap_or(0);
        let mo = c[2].parse::<i32>().unwrap_or(0);
        let d = c[3].parse::<i32>().unwrap_or(0);
        let h = c[4].parse::<i32>().unwrap_or(0);
        let mi = c[5].parse::<i32>().unwrap_or(0);
        let s = c[6].parse::<i32>().unwrap_or(0);
        if let Some(dt) = valid(y, Some(mo), Some(d), h, mi, s) {
            return (Some(dt), Some(Precision::Second));
        }
    }

    for c in res.date_re.captures_iter(name) {
        let m0 = c.get(0).unwrap();
        if !is_digit_boundary(name, m0.start(), m0.end()) {
            continue;
        }
        let y = c[1].parse::<i32>().unwrap_or(0);
        let g = |i: usize| c.get(i).map(|m| m.as_str().parse::<i32>().unwrap_or(0));
        let (mo, d, h, mi, s) = (g(2), g(3), g(4), g(5), g(6));

        if mo.is_none() {
            if let Some(dt) = valid(y, None, None, 0, 0, 0) {
                return (Some(dt), Some(Precision::Year));
            }
            continue;
        }
        let Some(dt) = valid(y, mo, d, h.unwrap_or(0), mi.unwrap_or(0), s.unwrap_or(0)) else {
            continue;
        };
        if h.is_some() {
            return (Some(dt), Some(Precision::Second));
        }
        if d.is_some() {
            return (Some(dt), Some(Precision::Day));
        }
        return (Some(dt), Some(Precision::Month));
    }

    if let Some(c) = res.compact_re.captures(name) {
        let m0 = c.get(0).unwrap();
        if !is_digit_boundary(name, m0.start(), m0.end()) {
            return (None, None);
        }
        let y = c[1].parse::<i32>().unwrap_or(0);
        let mo = c[2].parse::<i32>().unwrap_or(0);
        let d = c[3].parse::<i32>().unwrap_or(0);
        if let Some(dt) = valid(y, Some(mo), Some(d), 0, 0, 0) {
            return (Some(dt), Some(Precision::Day));
        }
    }

    let trimmed = name.trim();
    if let Some(c) = res.year_only_re.captures(trimmed) {
        let y = c[1].parse::<i32>().unwrap_or(0);
        if let Some(dt) = valid(y, None, None, 0, 0, 0) {
            return (Some(dt), Some(Precision::Year));
        }
    }
    (None, None)
}

/// 从完整路径逐级向上找日期线索（老照片常按年份分文件夹）。
pub fn parse_datetime_from_path(path: &Path) -> (Option<NaiveDateTime>, Option<Precision>) {
    let mut parts: Vec<String> = Vec::new();
    let mut p: &Path = path;
    for _ in 0..6 {
        let Some(name) = p.file_name().and_then(|s| s.to_str()) else {
            break;
        };
        parts.push(name.to_string());
        match p.parent() {
            Some(parent) if parent != p && !parent.as_os_str().is_empty() => p = parent,
            _ => break,
        }
    }
    for seg in parts {
        let (dt, prec) = parse_datetime(&seg);
        if dt.is_some() {
            return (dt, prec);
        }
    }
    (None, None)
}

/// ISO 字符串（`YYYY-MM-DD HH:MM:SS`）
pub fn iso(dt: Option<NaiveDateTime>) -> Option<String> {
    dt.map(|d| d.format("%Y-%m-%d %H:%M:%S").to_string())
}

pub fn iso_dt(dt: NaiveDateTime) -> String {
    dt.format("%Y-%m-%d %H:%M:%S").to_string()
}

/// 解析 ISO 字符串，容忍 `T` 分隔与纯日期。
pub fn parse_iso(s: &str) -> Option<NaiveDateTime> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let head: String = s.chars().take(19).collect();
    for fmt in ["%Y-%m-%d %H:%M:%S", "%Y-%m-%dT%H:%M:%S", "%Y-%m-%d"] {
        if let Ok(dt) = NaiveDateTime::parse_from_str(&head, fmt) {
            return Some(dt);
        }
        if let Ok(d) = NaiveDate::parse_from_str(&head, fmt) {
            return d.and_hms_opt(0, 0, 0);
        }
    }
    None
}

/// 从 Unix 时间戳（秒）转本地时间。
pub fn from_unix_ts(ts: f64) -> Option<NaiveDateTime> {
    let secs = ts.trunc() as i64;
    let nsecs = ((ts.fract()) * 1_000_000_000.0).round() as u32;
    let nsecs = nsecs.min(999_999_999);
    DateTime::from_timestamp(secs, nsecs).map(|dt| dt.with_timezone(&chrono::Local).naive_local())
}

pub fn haversine_km(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    if !(lat1.is_finite() && lon1.is_finite() && lat2.is_finite() && lon2.is_finite()) {
        return 0.0;
    }
    const R: f64 = 6371.0;
    let p1 = lat1.to_radians();
    let p2 = lat2.to_radians();
    let dp = p2 - p1;
    let dl = (lon2 - lon1).to_radians();
    let a = (dp / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
    2.0 * R * (a.sqrt().min(1.0)).asin()
}

/// 秒数格式化成「1 小时 2 分」这类中文表述。
pub fn fmt_seconds(sec: f64) -> String {
    if !sec.is_finite() {
        return "-".into();
    }
    let sec = sec as i64;
    let (h, rem) = (sec / 3600, sec % 3600);
    let (m, s) = (rem / 60, rem % 60);
    if h != 0 {
        return format!("{h} 小时 {m} 分");
    }
    if m != 0 {
        return format!("{m} 分 {s} 秒");
    }
    format!("{s} 秒")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ymd(y: i32, m: u32, d: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
    }

    #[test]
    fn parse_dash_date() {
        let (dt, p) = parse_datetime("1985-06-12 春节全家福.jpg");
        assert_eq!(dt, Some(ymd(1985, 6, 12)));
        assert_eq!(p, Some(Precision::Day));
    }

    #[test]
    fn parse_chinese_date() {
        let (dt, p) = parse_datetime("1985年06月12日 老家院子合影.jpg");
        assert_eq!(dt, Some(ymd(1985, 6, 12)));
        assert_eq!(p, Some(Precision::Day));
    }

    #[test]
    fn parse_compact_timestamp() {
        let (dt, p) = parse_datetime("20050315_141530.jpg");
        assert_eq!(p, Some(Precision::Second));
        let d = dt.unwrap();
        assert_eq!(d.date(), ymd(2005, 3, 15).date());
        assert_eq!(d.time().to_string(), "14:15:30");
    }

    #[test]
    fn parse_year_folder() {
        let (dt, p) = parse_datetime("2004年");
        assert_eq!(dt, Some(ymd(2004, 1, 1)));
        assert_eq!(p, Some(Precision::Year));
    }

    #[test]
    fn parse_no_date() {
        let (dt, p) = parse_datetime("DSC_0071.JPG");
        assert_eq!(dt, None);
        assert_eq!(p, None);
    }

    #[test]
    fn parse_rejects_out_of_range() {
        // 13 月、32 日都不合法；也不该被当成 2004 年整年
        let (dt, _) = parse_datetime("2004-13-45.jpg");
        assert_eq!(dt, None);
    }

    #[test]
    fn path_walks_up() {
        let (dt, p) = parse_datetime_from_path(Path::new("D:/照片/2004年/照片 001.jpg"));
        // 文件名自身没有日期，应从上级目录 2004年 得到
        assert_eq!(dt, Some(ymd(2004, 1, 1)));
        assert_eq!(p, Some(Precision::Year));
    }

    #[test]
    fn path_prefers_filename() {
        let (dt, _) = parse_datetime_from_path(Path::new("D:/2004年/1999-03-04 生日.jpg"));
        assert_eq!(dt, Some(ymd(1999, 3, 4)));
    }

    #[test]
    fn safe_name_strips_illegal() {
        assert_eq!(safe_name(r#"a/b\c:d*e?f"g<h>i|j"#, 90), "a_b_c_d_e_f_g_h_i_j");
        assert_eq!(safe_name("   ", 90), "untitled");
        assert_eq!(safe_name("..hidden..", 90), "hidden");
        // 制表符属于非法字符集，先变下划线，空格再压缩
        assert_eq!(safe_name("a   b\t c", 90), "a b_ c");
        assert_eq!(safe_name("a   b  c", 90), "a b c");
    }

    #[test]
    fn safe_name_keeps_extension_when_truncating() {
        let s = safe_name(&format!("{}.jpg", "长".repeat(200)), 20);
        assert!(s.ends_with(".jpg"));
        assert!(s.chars().count() <= 20);
    }

    #[test]
    fn human_size_units() {
        assert_eq!(human_size(512.0), "512 B");
        assert_eq!(human_size(1536.0), "1.5 KB");
        assert_eq!(human_size(1024.0 * 1024.0 * 3.5), "3.5 MB");
    }

    #[test]
    fn fmt_seconds_readable() {
        assert_eq!(fmt_seconds(45.0), "45 秒");
        assert_eq!(fmt_seconds(125.0), "2 分 5 秒");
        assert_eq!(fmt_seconds(3725.0), "1 小时 2 分");
    }

    #[test]
    fn iso_roundtrip() {
        let dt = ymd(1985, 6, 12);
        let s = iso(Some(dt)).unwrap();
        assert_eq!(s, "1985-06-12 00:00:00");
        assert_eq!(parse_iso(&s), Some(dt));
        assert_eq!(parse_iso("1985-06-12"), Some(dt));
        assert_eq!(parse_iso("1985-06-12T00:00:00"), Some(dt));
    }

    #[test]
    fn haversine_known_distance() {
        // 北京 -> 上海 约 1067 km
        let d = haversine_km(39.90, 116.41, 31.23, 121.47);
        assert!((d - 1067.0).abs() < 30.0, "got {d}");
    }

    #[test]
    fn is_image_by_extension() {
        assert!(is_image(Path::new("a.JPG")));
        assert!(is_image(Path::new("a.webp")));
        assert!(!is_image(Path::new("a.txt")));
        assert!(!is_image(Path::new("noext")));
    }

    #[test]
    fn is_video_by_extension() {
        assert!(is_video(Path::new("a.mp4")));
        assert!(is_video(Path::new("a.MOV")));
        assert!(is_video(Path::new("a.mkv")));
        assert!(!is_video(Path::new("a.jpg")));
        assert!(!is_video(Path::new("noext")));
    }

    #[test]
    fn is_media_covers_both_but_not_raw() {
        assert!(is_media(Path::new("a.jpg")));
        assert!(is_media(Path::new("a.mp4")));
        assert!(!is_media(Path::new("a.cr2")), "RAW 不进图库");
        assert!(!is_media(Path::new("a.txt")));
    }
}
