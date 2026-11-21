//! 整理归档：按分类规则复制/移动照片，支持命名模板、冲突处理与回滚。
//!
//! 与 Python 版 `opo/organizer.py` 对齐。

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use crate::library::{now_ts, Library, PhotoRow};
use crate::scanner::{CancelFlag, ProgressFn};
use crate::util::{ext_dot_lower, human_size_i, parse_iso, safe_name, sha1_file, unique_path};

/// 预设分类方案：方案名 -> 目录层级模板
pub const SCHEMES: &[(&str, &[&str])] = &[
    ("year", &["{year}"]),
    ("year_month", &["{year}", "{year}-{month}"]),
    (
        "year_month_day",
        &["{year}", "{year}-{month}", "{year}-{month}-{day}"],
    ),
    ("place", &["{place}"]),
    ("place_year", &["{place}", "{year}"]),
    ("place_year_month", &["{place}", "{year}", "{year}-{month}"]),
    ("quality", &["{quality}"]),
    ("dup", &["{dup}"]),
    ("flat", &[]),
];

pub const PLACE_UNKNOWN: &str = "未识别地点";
pub const FAVORITE_DIR: &str = "收藏";

/// 整理计划中的一项：`(photo_id, 相对路径)`
pub type PlanItem = (i64, String);

/// 按模板格式化一段路径。
fn fmt_template(template: &str, row: &PhotoRow, extra: &Extra) -> String {
    let d = row.dt.as_deref().and_then(parse_iso);
    let raw_place = row.place.as_deref().unwrap_or("").trim();
    let place = if raw_place.is_empty() || raw_place == crate::geo::UNKNOWN {
        PLACE_UNKNOWN
    } else {
        raw_place
    };

    let year = match row.year {
        Some(y) if y != 0 => format!("{y:04}"),
        _ => "无年份".to_string(),
    };
    let month = match row.month {
        Some(m) if m != 0 => format!("{m:02}"),
        _ => "00".to_string(),
    };
    let day = match row.day {
        Some(x) if x != 0 => format!("{x:02}"),
        _ => "00".to_string(),
    };
    let hour = format!("{:02}", row.hour.unwrap_or(0));

    let stem = Path::new(row.file_name().as_str())
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("photo")
        .to_string();
    let ext = {
        let e = row
            .ext
            .clone()
            .unwrap_or_else(|| ext_dot_lower(Path::new(&row.path)));
        e.trim_start_matches('.').to_string()
    };

    let mut out = template.to_string();
    let pairs: &[(&str, String)] = &[
        ("{year}", year),
        ("{month}", month),
        ("{day}", day),
        ("{hour}", hour),
        (
            "{minute}",
            d.map(|x| x.format("%M").to_string()).unwrap_or_else(|| "00".into()),
        ),
        (
            "{second}",
            d.map(|x| x.format("%S").to_string()).unwrap_or_else(|| "00".into()),
        ),
        (
            "{date}",
            d.map(|x| x.format("%Y-%m-%d").to_string())
                .unwrap_or_else(|| "0000-00-00".into()),
        ),
        (
            "{time}",
            d.map(|x| x.format("%H-%M-%S").to_string())
                .unwrap_or_else(|| "00-00-00".into()),
        ),
        ("{name}", stem),
        ("{ext}", ext.clone()),
        ("{tag}", extra.tag.clone().unwrap_or_else(|| "未分类".into())),
        (
            "{event}",
            extra.event.clone().unwrap_or_else(|| "未分组".into()),
        ),
        (
            "{quality}",
            extra.quality.clone().unwrap_or_else(|| "未评级".into()),
        ),
        ("{dup}", extra.dup.clone().unwrap_or_else(|| "独立".into())),
        (
            "{camera}",
            safe_name(
                row.camera_model.as_deref().unwrap_or("未知相机"),
                90,
            ),
        ),
        ("{place}", safe_name(place, 60)),
    ];
    for (k, v) in pairs {
        if out.contains(k) {
            out = out.replace(k, v);
        }
    }
    safe_name(&out, 120)
}

#[derive(Debug, Clone, Default)]
struct Extra {
    tag: Option<String>,
    event: Option<String>,
    quality: Option<String>,
    dup: Option<String>,
}

/// 自动标签里这些属于技术/尺寸类，不适合当语义分类名。
pub const BAD_AUTO_TAGS: &[&str] = &[
    "模糊",
    "过暗",
    "过亮",
    "低分辨率",
    "重复",
    "高光过曝",
    "大面积死黑",
    "横幅",
    "竖幅",
    "噪点重",
    "灰白/单色",
    "褪色平淡",
    "年代感强",
    "棕褐老照片",
    "时间待确认",
    "有人物",
    "高清大图",
    "疑似扫描件",
];

/// 取一个语义标签作为分类名：user 优先于 auto，跳过技术类。
fn tag_of(lib: &Library, pid: i64) -> String {
    let rows = lib.photo_tags_with_kind(pid);
    let mut user: Vec<String> = Vec::new();
    let mut auto: Vec<String> = Vec::new();
    for (name, kind) in rows {
        match kind.as_str() {
            "user" => user.push(name),
            "auto" => {
                if !BAD_AUTO_TAGS.contains(&name.as_str()) {
                    auto.push(name);
                }
            }
            _ => {}
        }
    }
    user.into_iter().chain(auto).next().unwrap_or_else(|| "未分类".into())
}

fn event_of(lib: &Library, pid: i64) -> String {
    lib.photo_tags_with_kind(pid)
        .into_iter()
        .find(|(_, k)| k == "event")
        .map(|(n, _)| n)
        .unwrap_or_else(|| "未分组".into())
}

fn quality_of(row: &PhotoRow) -> String {
    let q = row.quality_score.unwrap_or(0.0);
    if q >= 70.0 {
        "优质".into()
    } else if q >= 40.0 {
        "一般".into()
    } else {
        "较差".into()
    }
}

fn dup_of(row: &PhotoRow) -> String {
    if row.dup_group.is_some() {
        "重复".into()
    } else {
        "独立".into()
    }
}

/// 生成整理计划：`[(photo_id, 相对路径)]`。
pub struct PlanOptions<'a> {
    pub scheme: &'a str,
    pub template: Option<&'a str>,
    pub parts: Option<Vec<String>>,
    pub limit: usize,
    pub only_ids: Option<Vec<i64>>,
    pub favorite_dir: bool,
    pub favorite_in_regular: bool,
}

impl Default for PlanOptions<'_> {
    fn default() -> Self {
        Self {
            scheme: "year_month",
            template: None,
            parts: None,
            limit: 0,
            only_ids: None,
            favorite_dir: false,
            favorite_in_regular: true,
        }
    }
}

/// 生成整理计划。
pub fn plan_organize(lib: &Library, opts: &PlanOptions<'_>) -> Vec<PlanItem> {
    let parts: Vec<String> = match &opts.parts {
        Some(p) if !p.is_empty() => p.clone(),
        _ => SCHEMES
            .iter()
            .find(|(n, _)| *n == opts.scheme)
            .or_else(|| SCHEMES.iter().find(|(n, _)| *n == "year_month"))
            .map(|(_, p)| p.iter().map(|s| s.to_string()).collect())
            .unwrap_or_default(),
    };

    let need_tag = parts.iter().any(|x| x.contains("{tag}"));
    let need_event = parts.iter().any(|x| x.contains("{event}"));
    let need_quality = parts.iter().any(|x| x.contains("{quality}"));
    let need_dup = parts.iter().any(|x| x.contains("{dup}"));

    let mut where_clause = "missing=0".to_string();
    if let Some(ids) = &opts.only_ids {
        if !ids.is_empty() {
            let list = ids
                .iter()
                .map(|i| i.to_string())
                .collect::<Vec<_>>()
                .join(",");
            where_clause.push_str(&format!(" AND id IN ({list})"));
        }
    }

    let mut rows = lib.all_photos(&where_clause, "dt IS NULL, dt, id");
    if opts.limit > 0 {
        rows.truncate(opts.limit);
    }

    let fname_tpl = opts.template.unwrap_or("{date}_{time}_{name}");
    let mut plan: Vec<PlanItem> = Vec::new();
    let mut taken: HashSet<String> = HashSet::new();

    let add = |pid: i64, rel: String, taken: &mut HashSet<String>, plan: &mut Vec<PlanItem>| {
        let key = rel.to_lowercase();
        let rel = if taken.contains(&key) {
            let p = Path::new(&rel);
            let stem = p
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("file")
                .to_string();
            let ext = p
                .extension()
                .and_then(|s| s.to_str())
                .map(|s| format!(".{s}"))
                .unwrap_or_default();
            let mut k = 2;
            loop {
                let cand = format!("{stem}_{k}{ext}");
                let full = match p.parent() {
                    Some(dir) if !dir.as_os_str().is_empty() => dir.join(&cand),
                    _ => PathBuf::from(&cand),
                };
                let ck = full.to_string_lossy().to_lowercase();
                if !taken.contains(&ck) {
                    taken.insert(ck);
                    break full.to_string_lossy().to_string();
                }
                k += 1;
            }
        } else {
            taken.insert(key);
            rel
        };
        plan.push((pid, rel));
    };

    for r in &rows {
        let mut extra = Extra::default();
        if need_tag {
            extra.tag = Some(tag_of(lib, r.id));
        }
        if need_event {
            extra.event = Some(event_of(lib, r.id));
        }
        if need_quality {
            extra.quality = Some(quality_of(r));
        }
        if need_dup {
            extra.dup = Some(dup_of(r));
        }

        let segs: Vec<String> = parts.iter().map(|p| fmt_template(p, r, &extra)).collect();
        let mut base = fmt_template(fname_tpl, r, &extra);
        let ext = {
            let e = r
                .ext
                .clone()
                .unwrap_or_else(|| ext_dot_lower(Path::new(&r.path)));
            e.trim_start_matches('.').to_lowercase()
        };
        if !ext.is_empty() && !base.to_lowercase().ends_with(&format!(".{ext}")) {
            base = format!("{base}.{ext}");
        }

        let join = |segs: &[String], base: &str| -> String {
            let mut p = PathBuf::new();
            for s in segs {
                if !s.is_empty() {
                    p.push(s);
                }
            }
            p.push(base);
            p.to_string_lossy().to_string()
        };

        if r.is_favorite() && opts.favorite_dir {
            if opts.favorite_in_regular {
                // 常规分类一份 + 收藏夹一份
                add(r.id, join(&segs, &base), &mut taken, &mut plan);
                add(
                    r.id,
                    PathBuf::from(FAVORITE_DIR)
                        .join(&base)
                        .to_string_lossy()
                        .to_string(),
                    &mut taken,
                    &mut plan,
                );
            } else {
                add(
                    r.id,
                    PathBuf::from(FAVORITE_DIR)
                        .join(&base)
                        .to_string_lossy()
                        .to_string(),
                    &mut taken,
                    &mut plan,
                );
            }
            continue;
        }
        add(r.id, join(&segs, &base), &mut taken, &mut plan);
    }
    plan
}

/// 冲突处理策略
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Conflict {
    Rename,
    Skip,
    Overwrite,
}

/// 执行模式
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Copy,
    Move,
    Link,
}

impl Mode {
    pub fn parse(s: &str) -> Self {
        match s {
            "move" => Mode::Move,
            "link" => Mode::Link,
            _ => Mode::Copy,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Mode::Copy => "copy",
            Mode::Move => "move",
            Mode::Link => "link",
        }
    }
}

impl Conflict {
    pub fn parse(s: &str) -> Self {
        match s {
            "skip" => Conflict::Skip,
            "overwrite" => Conflict::Overwrite,
            _ => Conflict::Rename,
        }
    }
}

/// 执行结果
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ExecuteResult {
    pub done: usize,
    pub skipped: usize,
    pub errors: usize,
    pub moved: usize,
    pub total: usize,
    pub batch: String,
    pub dest: String,
    #[serde(skip)]
    pub log_ids: Vec<i64>,
}

/// 按计划执行。返回统计，可回滚。
pub fn execute_organize(
    lib: &mut Library,
    dest_root: &Path,
    plan: &[PlanItem],
    mode: Mode,
    on_conflict: Conflict,
    verify: bool,
    progress: Option<&ProgressFn>,
    cancel: Option<&CancelFlag>,
) -> ExecuteResult {
    let dest_root = absolutize(dest_root);
    let _ = std::fs::create_dir_all(&dest_root);
    let batch = format!("B{}", now_ts() as i64);
    let total = plan.len();
    let mut out = ExecuteResult {
        total,
        batch: batch.clone(),
        dest: dest_root.to_string_lossy().to_string(),
        ..Default::default()
    };
    let mut taken: HashSet<String> = HashSet::new();

    for (i, (pid, rel)) in plan.iter().enumerate() {
        if cancel.map(|c| c.load(Ordering::Relaxed)).unwrap_or(false) {
            break;
        }
        let Some(row) = lib.photo(*pid) else {
            out.errors += 1;
            continue;
        };
        let src = PathBuf::from(&row.path);
        if !src.exists() {
            out.errors += 1;
            continue;
        }
        let mut target = dest_root.join(rel);
        if let Some(parent) = target.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        if target.exists() {
            match on_conflict {
                Conflict::Skip => {
                    out.skipped += 1;
                    continue;
                }
                Conflict::Overwrite => {
                    let _ = std::fs::remove_file(&target);
                }
                Conflict::Rename => {
                    target = unique_path(&target, &mut taken);
                }
            }
        } else {
            taken.insert(target.to_string_lossy().to_lowercase());
        }

        let ok = match mode {
            Mode::Move => match std::fs::rename(&src, &target) {
                Ok(()) => {
                    out.moved += 1;
                    true
                }
                Err(_) => match copy_file(&src, &target) {
                    Ok(()) => {
                        let _ = std::fs::remove_file(&src);
                        out.moved += 1;
                        true
                    }
                    Err(_) => false,
                },
            },
            Mode::Link => hard_link_or_copy(&src, &target),
            Mode::Copy => copy_file(&src, &target).is_ok(),
        };

        if !ok {
            out.errors += 1;
            continue;
        }

        // 校验：复制后 sha1 应一致
        let mut verified = true;
        if verify && mode != Mode::Link {
            verified = match (&row.sha1, sha1_file(&target).ok()) {
                (Some(expected), Some(actual)) => expected == &actual,
                _ => sha1_file(&target).is_ok(),
            };
        }
        let detail = format!(
            "batch={};mode={};sha1ok={};size={}",
            batch,
            mode.as_str(),
            verified,
            human_size_i(row.size.unwrap_or(0))
        );
        let oid = lib.log_op("organize", &row.path, &target.to_string_lossy(), &detail);
        out.log_ids.push(oid);

        if mode == Mode::Move {
            // 注意：必须用 repoint_photo_path，不能走 upsert_photo。
            // upsert 以 path 为唯一键，传新路径会匹配不到旧行，
            // 结果就是「插入一条新记录 + 旧的还留着」，照片数凭空翻倍。
            lib.repoint_photo_path(&row.path, &target.to_string_lossy());
            lib.mark_missing(*pid, false);
        }

        out.done += 1;
        if let Some(p) = progress {
            if i % 20 == 0 || i == total.saturating_sub(1) {
                p(
                    "organize",
                    i + 1,
                    total,
                    target.file_name().and_then(|s| s.to_str()).unwrap_or(""),
                );
            }
        }
    }
    out
}

fn copy_file(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::copy(src, dst).map(|_| ())
}

/// 硬链接；不支持时退回复制。
fn hard_link_or_copy(src: &Path, dst: &Path) -> bool {
    if std::fs::hard_link(src, dst).is_ok() {
        return true;
    }
    copy_file(src, dst).is_ok()
}

/// 回滚结果
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct RollbackResult {
    pub restored: usize,
    pub errors: usize,
}

/// 撤销某批次（或最近未撤销）的整理操作。
pub fn rollback(lib: &mut Library, batch: Option<&str>) -> RollbackResult {
    let rows: Vec<(i64, String, String)> = match batch {
        Some(b) if !b.is_empty() => lib.organize_ops_of_batch(b),
        _ => {
            // 找最近一批
            match lib.latest_organize_batch() {
                Some(b) => lib.organize_ops_of_batch(&b),
                None => Vec::new(),
            }
        }
    };

    let mut out = RollbackResult::default();
    for (id, src, dst) in rows {
        let (src_p, dst_p) = (PathBuf::from(&src), PathBuf::from(&dst));
        if !dst_p.exists() {
            continue;
        }
        if let Some(parent) = src_p.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        // 源位置还占着就先删目标，否则把目标移回源位置
        let ok = if src_p.exists() {
            std::fs::remove_file(&dst_p).is_ok()
        } else {
            std::fs::rename(&dst_p, &src_p).is_ok()
                || (copy_file(&dst_p, &src_p).is_ok()
                    && std::fs::remove_file(&dst_p).is_ok())
        };
        if ok {
            lib.repoint_photo_path(&dst, &src);
            lib.mark_op_undone(id);
            out.restored += 1;
        } else {
            out.errors += 1;
        }
    }
    out
}

/// 原地重命名结果
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct RenameResult {
    pub renamed: usize,
    pub errors: usize,
    pub batch: String,
    pub dry_run: bool,
}

/// 原地重命名（同目录内），返回可回滚的日志。
pub fn rename_in_place(
    lib: &mut Library,
    plan: &[(i64, String)],
    dry_run: bool,
) -> RenameResult {
    let batch = format!("R{}", now_ts() as i64);
    let mut out = RenameResult {
        batch: batch.clone(),
        dry_run,
        ..Default::default()
    };
    for (pid, new_name) in plan {
        let Some(row) = lib.photo(*pid) else {
            out.errors += 1;
            continue;
        };
        let src = PathBuf::from(&row.path);
        let Some(dir) = src.parent() else {
            out.errors += 1;
            continue;
        };
        let dst = dir.join(Path::new(new_name).file_name().unwrap_or_default());
        if src == dst || !src.exists() {
            continue;
        }
        if dst.exists() {
            out.errors += 1;
            continue;
        }
        if dry_run {
            out.renamed += 1;
            continue;
        }
        match std::fs::rename(&src, &dst) {
            Ok(()) => {
                // 同样不能走 upsert：path 变了要原地更新，否则会插新行
                lib.repoint_photo_path(&row.path, &dst.to_string_lossy());
                lib.set_photo_name(*pid, dst.file_name().and_then(|s| s.to_str()).unwrap_or(""));
                lib.log_op("rename", &row.path, &dst.to_string_lossy(), &format!("batch={batch}"));
                out.renamed += 1;
            }
            Err(_) => out.errors += 1,
        }
    }
    out
}

fn absolutize(p: &Path) -> PathBuf {
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|c| c.join(p))
            .unwrap_or_else(|_| p.to_path_buf())
    }
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

    fn make_row(path: &str, year: i64, month: i64, day: i64) -> PhotoRow {
        let mut r = PhotoRow::default();
        r.id = 1;
        r.path = path.into();
        r.name = Path::new(path)
            .file_name()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string());
        r.year = Some(year);
        r.month = Some(month);
        r.day = Some(day);
        r.dt = Some(format!("{year}-{month:02}-{day:02} 10:00:00"));
        r.ext = Some(".jpg".into());
        r
    }

    #[test]
    fn scheme_templates_resolve() {
        let r = make_row(r"D:\p\a.jpg", 1985, 6, 12);
        let extra = Extra::default();
        assert_eq!(fmt_template("{year}", &r, &extra), "1985");
        assert_eq!(fmt_template("{year}-{month}", &r, &extra), "1985-06");
        assert_eq!(
            fmt_template("{year}-{month}-{day}", &r, &extra),
            "1985-06-12"
        );
        assert_eq!(fmt_template("{date}_{name}", &r, &extra), "1985-06-12_a");
    }

    #[test]
    fn template_handles_missing_date() {
        let mut r = make_row("a.jpg", 0, 0, 0);
        r.year = None;
        r.month = None;
        r.day = None;
        r.dt = None;
        let extra = Extra::default();
        assert_eq!(fmt_template("{year}", &r, &extra), "无年份");
        assert_eq!(fmt_template("{date}", &r, &extra), "0000-00-00");
    }

    #[test]
    fn template_unknown_place() {
        let mut r = make_row("a.jpg", 2000, 1, 1);
        r.place = Some(crate::geo::UNKNOWN.into());
        let extra = Extra::default();
        assert_eq!(fmt_template("{place}", &r, &extra), PLACE_UNKNOWN);
    }

    #[test]
    fn template_sanitizes_illegal_chars() {
        let mut r = make_row("a.jpg", 2000, 1, 1);
        r.place = Some("北京/上海".into());
        let extra = Extra::default();
        let s = fmt_template("{place}", &r, &extra);
        assert!(!s.contains('/'), "{s}");
    }

    #[test]
    fn plan_uses_year_month_structure() {
        let mut lib = Library::open_memory().unwrap();
        let mut u = PhotoUpdate::new(r"D:\p\a.jpg");
        u.int("year", Some(1985))
            .int("month", Some(6))
            .int("day", Some(12))
            .text("dt", Some("1985-06-12 10:00:00"))
            .text("ext", Some(".jpg"))
            .text("name", Some("a.jpg"));
        lib.upsert_photo(&u).unwrap();

        let plan = plan_organize(&lib, &PlanOptions::default());
        assert_eq!(plan.len(), 1);
        let rel = &plan[0].1;
        assert!(rel.contains("1985"), "{rel}");
        assert!(rel.contains("1985-06"), "{rel}");
        assert!(rel.ends_with(".jpg"), "{rel}");
    }

    #[test]
    fn plan_flat_has_no_subdirs() {
        let mut lib = Library::open_memory().unwrap();
        let mut u = PhotoUpdate::new("a.jpg");
        u.text("dt", Some("2000-01-01 00:00:00"))
            .int("year", Some(2000))
            .int("month", Some(1))
            .text("ext", Some(".jpg"));
        lib.upsert_photo(&u).unwrap();
        let opts = PlanOptions {
            scheme: "flat",
            ..Default::default()
        };
        let plan = plan_organize(&lib, &opts);
        assert_eq!(plan.len(), 1);
        assert!(!plan[0].1.contains('/'), "{:?}", plan[0].1);
    }

    #[test]
    fn plan_resolves_name_collisions() {
        let mut lib = Library::open_memory().unwrap();
        // 同一年月、同名的两张（不同目录）
        for p in [r"D:\a\x.jpg", r"D:\b\x.jpg"] {
            let mut u = PhotoUpdate::new(p);
            u.text("dt", Some("2000-01-01 00:00:00"))
                .int("year", Some(2000))
                .int("month", Some(1))
                .text("ext", Some(".jpg"))
                .text("name", Some("x.jpg"));
            lib.upsert_photo(&u).unwrap();
        }
        let opts = PlanOptions {
            template: Some("{name}"),
            ..Default::default()
        };
        let plan = plan_organize(&lib, &opts);
        assert_eq!(plan.len(), 2);
        assert_ne!(plan[0].1, plan[1].1, "同名应自动去重");
    }

    #[test]
    fn plan_favorite_dir_duplicates_entry() {
        let mut lib = Library::open_memory().unwrap();
        let mut u = PhotoUpdate::new("a.jpg");
        u.text("dt", Some("2000-01-01 00:00:00"))
            .int("year", Some(2000))
            .int("month", Some(1))
            .text("ext", Some(".jpg"))
            .text("name", Some("a.jpg"))
            .int("favorite", Some(1));
        lib.upsert_photo(&u).unwrap();

        let opts = PlanOptions {
            favorite_dir: true,
            favorite_in_regular: true,
            ..Default::default()
        };
        let plan = plan_organize(&lib, &opts);
        // 常规 + 收藏夹各一份
        assert_eq!(plan.len(), 2, "{plan:?}");
        assert!(plan.iter().any(|(_, r)| r.contains(FAVORITE_DIR)));
        assert!(plan.iter().any(|(_, r)| !r.contains(FAVORITE_DIR)));
    }

    #[test]
    fn plan_favorite_only_when_not_in_regular() {
        let mut lib = Library::open_memory().unwrap();
        let mut u = PhotoUpdate::new("a.jpg");
        u.text("dt", Some("2000-01-01 00:00:00"))
            .text("ext", Some(".jpg"))
            .int("favorite", Some(1));
        lib.upsert_photo(&u).unwrap();
        let opts = PlanOptions {
            favorite_dir: true,
            favorite_in_regular: false,
            ..Default::default()
        };
        let plan = plan_organize(&lib, &opts);
        assert_eq!(plan.len(), 1);
        assert!(plan[0].1.contains(FAVORITE_DIR));
    }

    #[test]
    fn plan_skips_missing_photos() {
        let mut lib = Library::open_memory().unwrap();
        let mut u = PhotoUpdate::new("a.jpg");
        u.int("missing", Some(1));
        lib.upsert_photo(&u).unwrap();
        let plan = plan_organize(&lib, &PlanOptions::default());
        assert!(plan.is_empty());
    }

    #[test]
    fn execute_copy_writes_files() {
        let src = work_dir("po_org_src");
        std::fs::copy(sample_dir().join("DSC_0071.JPG"), src.join("a.jpg")).unwrap();
        let dest = work_dir("po_org_dest");

        let mut lib = Library::open_memory().unwrap();
        let mut u = PhotoUpdate::new(src.join("a.jpg").to_string_lossy().to_string());
        u.text("dt", Some("2000-01-01 00:00:00"))
            .int("year", Some(2000))
            .int("month", Some(1))
            .text("ext", Some(".jpg"));
        lib.upsert_photo(&u).unwrap();

        let plan = plan_organize(&lib, &PlanOptions::default());
        let r = execute_organize(&mut lib, &dest, &plan, Mode::Copy, Conflict::Rename, true, None, None);
        assert_eq!(r.done, 1, "{r:?}");
        assert_eq!(r.errors, 0);
        assert_eq!(r.log_ids.len(), 1);
        assert!(dest.join(&plan[0].1).exists());
        // 源文件仍在
        assert!(src.join("a.jpg").exists());
    }

    #[test]
    fn execute_move_relocates_and_updates_db() {
        let src = work_dir("po_org_move_src");
        std::fs::copy(sample_dir().join("DSC_0071.JPG"), src.join("a.jpg")).unwrap();
        let dest = work_dir("po_org_move_dest");

        let mut lib = Library::open_memory().unwrap();
        let path = src.join("a.jpg").to_string_lossy().to_string();
        let mut u = PhotoUpdate::new(path.clone());
        u.text("dt", Some("2000-01-01 00:00:00"))
            .int("year", Some(2000))
            .int("month", Some(1))
            .text("ext", Some(".jpg"));
        let pid = lib.upsert_photo(&u).unwrap();

        let plan = plan_organize(&lib, &PlanOptions::default());
        let r = execute_organize(&mut lib, &dest, &plan, Mode::Move, Conflict::Rename, true, None, None);
        assert_eq!(r.done, 1);
        assert_eq!(r.moved, 1);
        assert!(!src.join("a.jpg").exists(), "源文件应被移走");
        // 库内路径已更新
        let row = lib.photo(pid).unwrap();
        assert!(row.path.contains(dest.to_string_lossy().as_ref()), "{}", row.path);
    }

    #[test]
    fn execute_conflict_skip() {
        let src = work_dir("po_org_skip_src");
        std::fs::copy(sample_dir().join("DSC_0071.JPG"), src.join("a.jpg")).unwrap();
        let dest = work_dir("po_org_skip_dest");

        let mut lib = Library::open_memory().unwrap();
        let mut u = PhotoUpdate::new(src.join("a.jpg").to_string_lossy().to_string());
        u.text("dt", Some("2000-01-01 00:00:00"))
            .int("year", Some(2000))
            .int("month", Some(1))
            .text("ext", Some(".jpg"));
        lib.upsert_photo(&u).unwrap();

        let plan = plan_organize(&lib, &PlanOptions::default());
        // 先占位
        let target = dest.join(&plan[0].1);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, b"occupied").unwrap();

        let r = execute_organize(&mut lib, &dest, &plan, Mode::Copy, Conflict::Skip, false, None, None);
        assert_eq!(r.skipped, 1);
        assert_eq!(r.done, 0);
        // 占位文件未被覆盖
        assert_eq!(std::fs::read(&target).unwrap(), b"occupied");
    }

    #[test]
    fn execute_handles_missing_source() {
        let dest = work_dir("po_org_missing_dest");
        let mut lib = Library::open_memory().unwrap();
        lib.upsert_photo(&mut PhotoUpdate::new("no_such_file_xyz.jpg"))
            .unwrap();
        let plan = vec![(1i64, "x.jpg".to_string())];
        let r = execute_organize(&mut lib, &dest, &plan, Mode::Copy, Conflict::Rename, false, None, None);
        assert_eq!(r.errors, 1);
        assert_eq!(r.done, 0);
    }

    #[test]
    fn rollback_restores_move() {
        let src = work_dir("po_org_rb_src");
        std::fs::copy(sample_dir().join("DSC_0071.JPG"), src.join("a.jpg")).unwrap();
        let dest = work_dir("po_org_rb_dest");

        let mut lib = Library::open_memory().unwrap();
        let orig = src.join("a.jpg").to_string_lossy().to_string();
        let mut u = PhotoUpdate::new(orig.clone());
        u.text("dt", Some("2000-01-01 00:00:00"))
            .int("year", Some(2000))
            .int("month", Some(1))
            .text("ext", Some(".jpg"));
        lib.upsert_photo(&u).unwrap();

        let plan = plan_organize(&lib, &PlanOptions::default());
        let r = execute_organize(&mut lib, &dest, &plan, Mode::Move, Conflict::Rename, true, None, None);
        assert_eq!(r.moved, 1);
        assert!(!src.join("a.jpg").exists());

        let rb = rollback(&mut lib, Some(&r.batch));
        assert_eq!(rb.restored, 1, "{rb:?}");
        assert_eq!(rb.errors, 0);
        assert!(src.join("a.jpg").exists(), "回滚后源文件应回来");
    }

    #[test]
    fn rollback_of_copy_does_not_destroy_source() {
        let src = work_dir("po_org_rbc_src");
        std::fs::copy(sample_dir().join("DSC_0071.JPG"), src.join("a.jpg")).unwrap();
        let dest = work_dir("po_org_rbc_dest");
        let mut lib = Library::open_memory().unwrap();
        let mut u = PhotoUpdate::new(src.join("a.jpg").to_string_lossy().to_string());
        u.text("dt", Some("2000-01-01 00:00:00"))
            .text("ext", Some(".jpg"))
            .int("year", Some(2000))
            .int("month", Some(1));
        lib.upsert_photo(&u).unwrap();
        let plan = plan_organize(&lib, &PlanOptions::default());
        let r = execute_organize(&mut lib, &dest, &plan, Mode::Copy, Conflict::Rename, true, None, None);
        let _ = rollback(&mut lib, Some(&r.batch));
        // 复制模式下源文件自始至终都在
        assert!(src.join("a.jpg").exists());
        assert_eq!(lib.count(""), 1);
    }

    #[test]
    fn rename_in_place_dry_run_changes_nothing() {
        let src = work_dir("po_org_rename_dry");
        std::fs::copy(sample_dir().join("DSC_0071.JPG"), src.join("a.jpg")).unwrap();
        let mut lib = Library::open_memory().unwrap();
        let pid = lib
            .upsert_photo(&mut PhotoUpdate::new(
                src.join("a.jpg").to_string_lossy().to_string(),
            ))
            .unwrap();
        let r = rename_in_place(&mut lib, &[(pid, "b.jpg".into())], true);
        assert_eq!(r.renamed, 1);
        assert!(src.join("a.jpg").exists(), "预览模式不该真的改名");
        assert!(!src.join("b.jpg").exists());
    }

    #[test]
    fn rename_in_place_actual() {
        let src = work_dir("po_org_rename_real");
        std::fs::copy(sample_dir().join("DSC_0071.JPG"), src.join("a.jpg")).unwrap();
        let mut lib = Library::open_memory().unwrap();
        let pid = lib
            .upsert_photo(&mut PhotoUpdate::new(
                src.join("a.jpg").to_string_lossy().to_string(),
            ))
            .unwrap();
        let r = rename_in_place(&mut lib, &[(pid, "b.jpg".into())], false);
        assert_eq!(r.renamed, 1);
        assert!(!src.join("a.jpg").exists());
        assert!(src.join("b.jpg").exists());
        let row = lib.photo(pid).unwrap();
        assert!(row.path.ends_with("b.jpg"));
        assert_eq!(row.name.as_deref(), Some("b.jpg"));
    }

    #[test]
    fn rename_refuses_to_overwrite_existing() {
        let src = work_dir("po_org_rename_conflict");
        std::fs::copy(sample_dir().join("DSC_0071.JPG"), src.join("a.jpg")).unwrap();
        std::fs::copy(sample_dir().join("DSC_0071.JPG"), src.join("b.jpg")).unwrap();
        let mut lib = Library::open_memory().unwrap();
        let pid = lib
            .upsert_photo(&mut PhotoUpdate::new(
                src.join("a.jpg").to_string_lossy().to_string(),
            ))
            .unwrap();
        let r = rename_in_place(&mut lib, &[(pid, "b.jpg".into())], false);
        assert_eq!(r.errors, 1, "目标已存在应报错而不覆盖");
        assert!(src.join("a.jpg").exists());
        assert!(src.join("b.jpg").exists());
    }

    #[test]
    fn all_schemes_produce_safe_paths() {
        let mut lib = Library::open_memory().unwrap();
        let mut u = PhotoUpdate::new("a.jpg");
        u.text("dt", Some("1985-06-12 10:00:00"))
            .int("year", Some(1985))
            .int("month", Some(6))
            .int("day", Some(12))
            .text("ext", Some(".jpg"))
            .text("name", Some("a.jpg"))
            .text("place", Some("北京"))
            .real("quality_score", Some(80.0));
        lib.upsert_photo(&u).unwrap();

        for (name, _) in SCHEMES {
            let opts = PlanOptions {
                scheme: name,
                ..Default::default()
            };
            let plan = plan_organize(&lib, &opts);
            assert_eq!(plan.len(), 1, "方案 {name} 应生成 1 条计划");
            let rel = &plan[0].1;
            for bad in ['<', '>', ':', '"', '|', '?', '*'] {
                assert!(!rel.contains(bad), "方案 {name} 的路径含非法字符 {bad}: {rel}");
            }
        }
    }
}
