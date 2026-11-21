//! 扫描 + 分析流水线。支持增量、多线程、进度回调与取消。
//!
//! 与 Python 版 `opo/scanner.py` 对齐。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use rayon::prelude::*;

use crate::config::{self, Settings};
use crate::imaging;
use crate::library::{now_ts, Library, PhotoUpdate};
use crate::util::{
    ext_dot_lower, from_unix_ts, is_image, iso_dt, parse_datetime, parse_datetime_from_path,
    sha1_file, Precision,
};

/// 遍历时跳过的目录名（小写比较）
pub const SKIP_DIRS: &[&str] = &[
    "$recycle.bin",
    "system volume information",
    ".git",
    "node_modules",
    "__pycache__",
    ".thumbnails",
    "thumbs",
    "@eadir",
];

/// 本工具自己的整理输出目录，扫描时跳过
pub const OWN_OUTPUT_DIRS: &[&str] = &["oldphoto_organized", "opo-organized"];

/// 进度回调：`(阶段, 已完成, 总数, 详情)`
pub type ProgressFn = Arc<dyn Fn(&str, usize, usize, &str) + Send + Sync>;

/// 取消标志
pub type CancelFlag = Arc<AtomicBool>;

/// 遍历目录，产出图片路径。
pub fn walk_images(
    root: &Path,
    recursive: bool,
    exclude: &[PathBuf],
    cancel: Option<&CancelFlag>,
) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if !root.is_dir() {
        return out;
    }
    let is_cancelled = || cancel.map(|c| c.load(Ordering::Relaxed)).unwrap_or(false);

    if !recursive {
        if let Ok(entries) = std::fs::read_dir(root) {
            let mut names: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
            names.sort();
            for p in names {
                if is_cancelled() {
                    return out;
                }
                if p.is_file() && is_image(&p) {
                    out.push(p);
                }
            }
        }
        return out;
    }

    let excluded: Vec<String> = exclude
        .iter()
        .map(|p| p.to_string_lossy().to_lowercase())
        .collect();

    // 显式栈遍历，避免深目录递归爆栈
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if is_cancelled() {
            break;
        }
        let dir_name = dir
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_lowercase();
        if OWN_OUTPUT_DIRS.contains(&dir_name.as_str()) {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut subdirs = Vec::new();
        for e in entries.flatten() {
            let p = e.path();
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                let name = p
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_lowercase();
                if name.starts_with('.')
                    || SKIP_DIRS.contains(&name.as_str())
                    || excluded.contains(&p.to_string_lossy().to_lowercase())
                {
                    continue;
                }
                subdirs.push(p);
            } else if ft.is_file() && is_image(&p) {
                out.push(p);
            }
        }
        // 保证同层顺序稳定，便于复现
        subdirs.sort();
        for d in subdirs.into_iter().rev() {
            stack.push(d);
        }
    }
    out.sort();
    out
}

/// 扫描结果
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ScanResult {
    pub added: usize,
    pub skipped: usize,
    pub errors: usize,
    pub roots: usize,
    pub seen: usize,
}

/// 把磁盘上的新文件写进库（只登记，不做重分析）。
pub fn scan_roots(
    lib: &mut Library,
    _cfg: &Settings,
    roots: Option<&[String]>,
    progress: Option<&ProgressFn>,
    cancel: Option<&CancelFlag>,
    force: bool,
) -> ScanResult {
    let roots_owned: Vec<String> = match roots {
        Some(r) => r.to_vec(),
        None => lib.roots(),
    };
    let mut res = ScanResult {
        roots: roots_owned.len(),
        ..Default::default()
    };
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    for root in &roots_owned {
        lib.add_root(Path::new(root));
        let files = walk_images(Path::new(root), true, &[], cancel);
        for path in files {
            if cancel.map(|c| c.load(Ordering::Relaxed)).unwrap_or(false) {
                break;
            }
            let key = path.to_string_lossy().to_string();
            seen.insert(key.clone());

            let Ok(meta) = std::fs::metadata(&path) else {
                res.errors += 1;
                continue;
            };
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs_f64())
                .unwrap_or(0.0);
            let ctime = meta
                .created()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs_f64())
                .unwrap_or(0.0);

            if !force {
                if let Some(row) = lib.photo_by_path(&key) {
                    if (row.mtime.unwrap_or(0.0) - mtime).abs() < 1e-6 {
                        res.skipped += 1;
                        continue;
                    }
                }
            }

            let mut u = PhotoUpdate::new(key.clone());
            u.text("folder", path.parent().and_then(|p| p.to_str()))
                .text("name", path.file_name().and_then(|s| s.to_str()))
                .text(
                    "ext",
                    Some(ext_dot_lower(&path).as_str()).filter(|s| !s.is_empty()),
                )
                .int("size", Some(meta.len() as i64))
                .real("mtime", Some(mtime))
                .real("ctime", Some(ctime));
            if lib.upsert_photo(&u).is_err() {
                res.errors += 1;
                continue;
            }
            res.added += 1;

            if let Some(p) = progress {
                if res.added % 25 == 0 {
                    p(
                        "扫描目录",
                        res.added,
                        0,
                        path.file_name().and_then(|s| s.to_str()).unwrap_or(""),
                    );
                }
            }
        }
    }

    // 标记消失的文件。
    //
    // 注意不要用 `if !seen.is_empty()` 做守卫：扫描一个空目录（或所有文件都已
    // 删除的目录）时 seen 为空，但那恰恰是最需要标记 missing 的情况。
    // 这里的判据是「文件系统上真的不在了」，见下面的 exists() 检查。
    {
        let rows = lib.all_photos("missing=0", "id");
        for r in rows {
            if !seen.contains(&r.path) && !Path::new(&r.path).exists() {
                lib.mark_missing(r.id, true);
            }
        }
    }
    res.seen = seen.len();
    res
}

/// 单张分析结果
struct Analyzed {
    id: i64,
    res: Option<imaging::Analysis>,
    err: Option<String>,
}

/// 分析还没分析过的照片：EXIF、质量、哈希、缩略图。
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct AnalyzeResult {
    pub total: usize,
    pub done: usize,
    pub failed: usize,
}

pub fn analyze_pending(
    lib: &mut Library,
    cfg: &Settings,
    limit: usize,
    progress: Option<&ProgressFn>,
    cancel: Option<&CancelFlag>,
    workers: usize,
    redo: bool,
) -> AnalyzeResult {
    let where_clause = if redo {
        "1=1".to_string()
    } else {
        "analyzed_at IS NULL OR thumb IS NULL".to_string()
    };
    let mut sql = format!("SELECT * FROM photos WHERE {where_clause} ORDER BY id");
    if limit > 0 {
        sql.push_str(&format!(" LIMIT {limit}"));
    }
    let rows = lib.all_photos(&where_clause, "id");
    let rows: Vec<_> = rows.into_iter().take(if limit > 0 { limit } else { usize::MAX }).collect();

    let total = rows.len();
    if total == 0 {
        return AnalyzeResult::default();
    }

    let thumb_dir = config::thumb_dir();
    let thumb_size = cfg.usize("thumb_size") as u32;
    let cfg_owned = cfg.clone();

    let tasks: Vec<(i64, String, f64)> = rows
        .iter()
        .map(|r| (r.id, r.path.clone(), r.mtime.unwrap_or(0.0)))
        .collect();

    let done_ctr = AtomicUsize::new(0);
    let fail_ctr = AtomicUsize::new(0);

    let analyzed: Vec<Analyzed> = {
        let mut pool = rayon::ThreadPoolBuilder::new();
        let n = if workers == 0 {
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4)
                .min(8)
        } else {
            workers
        };
        pool = pool.num_threads(n.max(1));
        let result = std::sync::Mutex::new(Vec::with_capacity(tasks.len()));

        let do_one = |(id, path, mtime): &(i64, String, f64)| -> Analyzed {
            if cancel.map(|c| c.load(Ordering::Relaxed)).unwrap_or(false) {
                return Analyzed {
                    id: *id,
                    res: None,
                    err: Some("cancelled".into()),
                };
            }
            let th = thumb_dir.join(format!(
                "{}.jpg",
                imaging::thumb_key(path, thumb_size, *mtime)
            ));
            let out = imaging::analyze_photo(Path::new(path), &cfg_owned, Some(&th));
            match out {
                Some(mut a) => {
                    // 缩略图路径只在成功时记录
                    if a.thumb_ok {
                        a.thumb_path = Some(th.to_string_lossy().to_string());
                    }
                    Analyzed {
                        id: *id,
                        res: Some(a),
                        err: None,
                    }
                }
                None => Analyzed {
                    id: *id,
                    res: None,
                    err: Some("decode failed".into()),
                },
            }
        };

        match pool.build() {
            Ok(pool) => {
                pool.install(|| {
                    analyzed_parallel(&tasks, &do_one, &done_ctr, &fail_ctr, progress, total, &result);
                });
                result.into_inner().unwrap_or_default()
            }
            Err(_) => {
                // 线程池建不起来就退化成串行
                for t in &tasks {
                    let a = do_one(t);
                    let n = done_ctr.fetch_add(1, Ordering::Relaxed) + 1;
                    if let Some(p) = progress {
                        if n % 20 == 0 || n == total {
                            p("分析照片", n, total, "");
                        }
                    }
                    if a.res.is_none() {
                        fail_ctr.fetch_add(1, Ordering::Relaxed);
                    }
                    if let Ok(mut guard) = result.lock() {
                        guard.push(a);
                    }
                }
                result.into_inner().unwrap_or_default()
            }
        }
    };

    // 写库（单线程，避免 SQLite 竞争）
    let mut out = AnalyzeResult {
        total,
        ..Default::default()
    };
    for a in analyzed {
        match a.res {
            Some(res) => {
                update_from_analysis(lib, a.id, &res, cfg);
                out.done += 1;
            }
            None => {
                let msg = a.err.unwrap_or_default();
                let _ = lib.set_meta("last_error", &format!("{}: {}", a.id, msg));
                out.failed += 1;
            }
        }
    }
    out
}

fn analyzed_parallel(
    tasks: &[(i64, String, f64)],
    do_one: &(dyn Fn(&(i64, String, f64)) -> Analyzed + Sync),
    done_ctr: &AtomicUsize,
    fail_ctr: &AtomicUsize,
    progress: Option<&ProgressFn>,
    total: usize,
    result: &std::sync::Mutex<Vec<Analyzed>>,
) {
    tasks.par_iter().for_each(|t| {
        let a = do_one(t);
        let n = done_ctr.fetch_add(1, Ordering::Relaxed) + 1;
        if let Some(p) = progress {
            if n % 20 == 0 || n == total {
                p("分析照片", n, total, "");
            }
        }
        if a.res.is_none() {
            fail_ctr.fetch_add(1, Ordering::Relaxed);
        }
        if let Ok(mut guard) = result.lock() {
            guard.push(a);
        }
    });
}

/// 把分析结果写回库：时间、EXIF、质量指标、哈希、缩略图。
pub fn update_from_analysis(
    lib: &mut Library,
    pid: i64,
    res: &imaging::Analysis,
    cfg: &Settings,
) {
    let Some(row) = lib.photo(pid) else {
        return;
    };
    let path = row.path.clone();
    let path_ref = Path::new(&path);

    // 时间：EXIF > 文件名 > 目录 > 文件修改时间
    let ex = crate::exif::read_exif(path_ref);
    let mut dt = ex.datetime;
    let mut src = "exif";
    let mut prec = Precision::Second;

    if dt.is_none() {
        let name = path_ref.file_name().and_then(|s| s.to_str()).unwrap_or("");
        let (d, p) = parse_datetime(name);
        dt = d;
        src = "filename";
        if let Some(p) = p {
            prec = p;
        }
    }
    if dt.is_none() {
        if let Some(dir) = path_ref.parent() {
            let (d, p) = parse_datetime_from_path(dir);
            dt = d;
            src = "folder";
            if let Some(p) = p {
                prec = p;
            }
        }
    }
    if dt.is_none() {
        if let Some(d) = from_unix_ts(row.mtime.unwrap_or(0.0)) {
            dt = Some(d);
            src = "fs";
            prec = Precision::Day;
        }
    }

    let m = &res.metrics;
    let mut u = PhotoUpdate::new(path.clone());
    u.text("sha1", sha1_file(path_ref).ok().as_deref())
        .int("width", Some(m.width as i64))
        .int("height", Some(m.height as i64))
        .real("megapixels", Some(m.megapixels))
        .real("aspect", Some(m.aspect))
        .real("blur", Some(m.blur))
        .real("brightness", Some(m.brightness))
        .real("contrast", Some(m.contrast))
        .real("saturation", Some(m.saturation))
        .real("hue", Some(m.hue))
        .real("sepia", Some(m.sepia))
        .real("color", Some(m.color))
        .real("skin", Some(m.skin))
        .real("noise", Some(m.noise))
        .real("sharpness", Some(m.sharpness))
        .real("dark_ratio", Some(m.dark_ratio))
        .real("white_ratio", Some(m.white_ratio))
        .text("a_hash", Some(&res.a_hash))
        .text("d_hash", Some(&res.d_hash))
        .text("p_hash", Some(&res.p_hash))
        .text("flags", Some(&res.flags.join("|")))
        .int("has_people", Some(if res.has_people { 1 } else { 0 }))
        .int("is_mono", Some(if res.is_mono { 1 } else { 0 }))
        .int("is_sepia", Some(if res.is_sepia { 1 } else { 0 }))
        .real("age_score", Some(res.age_score))
        .real("quality_score", Some(res.quality_score))
        .text("thumb", res.thumb_path.as_deref())
        .real("analyzed_at", Some(now_ts()))
        .int("missing", Some(0));

    if let Some(d) = dt {
        u.text("dt", Some(&iso_dt(d)))
            .text("dt_source", Some(src))
            .text("dt_precision", Some(prec.as_str()))
            .int("year", Some(d.format("%Y").to_string().parse().unwrap_or(0)))
            .int("month", Some(d.format("%m").to_string().parse().unwrap_or(0)))
            .int("day", Some(d.format("%d").to_string().parse().unwrap_or(0)))
            .int("hour", Some(d.format("%H").to_string().parse().unwrap_or(0)));
    }
    if let Some(v) = ex.make.as_deref().filter(|s| !s.is_empty()) {
        u.text("camera_make", Some(v));
    }
    if let Some(v) = ex.model.as_deref().filter(|s| !s.is_empty()) {
        u.text("camera_model", Some(v));
    }
    if let Some(v) = ex.software.as_deref().filter(|s| !s.is_empty()) {
        u.text("software", Some(v));
    }
    if ex.lat.is_some() {
        u.real("lat", ex.lat).real("lon", ex.lon);
    }

    // 地点识别（常驻功能）
    let hint = format!(
        "{} {}",
        path_ref.parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default(),
        path_ref.file_name().and_then(|s| s.to_str()).unwrap_or("")
    );
    let place = crate::geo::resolve_place(ex.lat, ex.lon, &hint);
    u.text("place", Some(&place));

    let _ = cfg;
    let _ = lib.upsert_photo(&u);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::Library;

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
    fn walk_finds_images_recursively() {
        let files = walk_images(&sample_dir(), true, &[], None);
        assert!(files.len() >= 9, "应找到至少 9 张，实际 {}", files.len());
        // 子目录也要进去
        assert!(files.iter().any(|p| p.ends_with("照片 001.jpg")));
        assert!(files.iter().any(|p| p.to_string_lossy().contains("重复")));
    }

    #[test]
    fn walk_skips_non_images() {
        let dir = work_dir("po_walk_test");
        std::fs::write(dir.join("a.jpg"), b"x").unwrap();
        std::fs::write(dir.join("b.txt"), b"x").unwrap();
        std::fs::write(dir.join("c.png"), b"x").unwrap();
        let files = walk_images(&dir, true, &[], None);
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn walk_skips_hidden_and_skip_dirs() {
        let dir = work_dir("po_walk_skip");
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::create_dir_all(dir.join("node_modules")).unwrap();
        std::fs::create_dir_all(dir.join("thumbs")).unwrap();
        std::fs::create_dir_all(dir.join("normal")).unwrap();
        for d in [".git", "node_modules", "thumbs", "normal"] {
            std::fs::write(dir.join(d).join("x.jpg"), b"x").unwrap();
        }
        std::fs::write(dir.join("top.jpg"), b"x").unwrap();
        let files = walk_images(&dir, true, &[], None);
        // 只应有 top.jpg 和 normal/x.jpg
        assert_eq!(files.len(), 2, "{files:?}");
    }

    #[test]
    fn walk_skips_own_output_dir() {
        let dir = work_dir("po_walk_own");
        std::fs::create_dir_all(dir.join("opo-organized")).unwrap();
        std::fs::write(dir.join("opo-organized").join("x.jpg"), b"x").unwrap();
        std::fs::write(dir.join("y.jpg"), b"x").unwrap();
        let files = walk_images(&dir, true, &[], None);
        assert_eq!(files.len(), 1);
    }

    #[test]
    fn walk_nonexistent_root_is_empty() {
        assert!(walk_images(Path::new("no_such_dir_xyz"), true, &[], None).is_empty());
    }

    #[test]
    fn walk_respects_cancel() {
        let cancel: CancelFlag = Arc::new(AtomicBool::new(true));
        let files = walk_images(&sample_dir(), true, &[], Some(&cancel));
        assert!(files.is_empty(), "取消后不应返回任何文件");
    }

    #[test]
    fn walk_non_recursive_only_top() {
        let dir = work_dir("po_walk_flat");
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("top.jpg"), b"x").unwrap();
        std::fs::write(dir.join("sub").join("deep.jpg"), b"x").unwrap();
        let files = walk_images(&dir, false, &[], None);
        assert_eq!(files.len(), 1);
    }

    #[test]
    fn scan_registers_files() {
        let dir = work_dir("po_scan_test");
        for n in ["1990-05-01 春游.jpg", "无日期照片.jpg"] {
            std::fs::copy(sample_dir().join("DSC_0071.JPG"), dir.join(n)).unwrap();
        }
        let mut lib = Library::open_memory().unwrap();
        let cfg = Settings::new();
        let r = scan_roots(&mut lib, &cfg, Some(&[dir.to_string_lossy().to_string()]), None, None, false);
        assert_eq!(r.added, 2, "{r:?}");
        assert_eq!(lib.count(""), 2);
    }

    #[test]
    fn scan_skips_unchanged_on_rescan() {
        let dir = work_dir("po_scan_rescan");
        std::fs::copy(sample_dir().join("DSC_0071.JPG"), dir.join("a.jpg")).unwrap();
        let mut lib = Library::open_memory().unwrap();
        let cfg = Settings::new();
        let root = dir.to_string_lossy().to_string();
        let r1 = scan_roots(&mut lib, &cfg, Some(&[root.clone()]), None, None, false);
        assert_eq!(r1.added, 1);
        // 第二次没有变化，应全部跳过
        let r2 = scan_roots(&mut lib, &cfg, Some(&[root]), None, None, false);
        assert_eq!(r2.added, 0);
        assert_eq!(r2.skipped, 1);
    }

    #[test]
    fn scan_marks_missing_when_file_gone() {
        let dir = work_dir("po_scan_missing");
        let f = dir.join("a.jpg");
        std::fs::copy(sample_dir().join("DSC_0071.JPG"), &f).unwrap();
        let mut lib = Library::open_memory().unwrap();
        let cfg = Settings::new();
        let root = dir.to_string_lossy().to_string();
        scan_roots(&mut lib, &cfg, Some(&[root.clone()]), None, None, false);
        assert_eq!(lib.count("missing=1"), 0);

        std::fs::remove_file(&f).unwrap();
        // 重新扫描一个现在为空的目录：这恰恰是最需要标记 missing 的场景，
        // 不能因为 seen 为空就跳过整段逻辑。
        scan_roots(&mut lib, &cfg, Some(&[root]), None, None, false);
        assert_eq!(lib.count("missing=1"), 1, "文件已删除，应被标记为丢失");
    }

    #[test]
    fn scan_does_not_mark_missing_if_file_still_exists() {
        // 文件还在但这次没扫到（例如换了 root），不该标 missing
        let dir = work_dir("po_scan_keep");
        let f = dir.join("a.jpg");
        std::fs::copy(sample_dir().join("DSC_0071.JPG"), &f).unwrap();
        let mut lib = Library::open_memory().unwrap();
        let cfg = Settings::new();
        scan_roots(&mut lib, &cfg, Some(&[dir.to_string_lossy().to_string()]), None, None, false);

        // 扫另一个空目录，原文件仍在磁盘上
        let other = work_dir("po_scan_keep_other");
        scan_roots(&mut lib, &cfg, Some(&[other.to_string_lossy().to_string()]), None, None, false);
        assert_eq!(lib.count("missing=1"), 0, "文件仍在磁盘上，不该标 missing");
    }

    #[test]
    fn analyze_produces_metrics_and_thumb() {
        let dir = work_dir("po_analyze_test");
        // 保留原文件名，让日期能从文件名解析出来
        std::fs::copy(
            sample_dir().join("1985-06-12 春节全家福.jpg"),
            dir.join("1985-06-12 春节全家福.jpg"),
        )
        .unwrap();
        let mut lib = Library::open_memory().unwrap();
        let cfg = Settings::new();
        let root = dir.to_string_lossy().to_string();
        scan_roots(&mut lib, &cfg, Some(&[root]), None, None, false);

        let r = analyze_pending(&mut lib, &cfg, 0, None, None, 1, false);
        assert_eq!(r.total, 1);
        assert_eq!(r.done, 1, "{r:?}");
        assert_eq!(r.failed, 0);

        let row = lib.all_photos("", "id").remove(0);
        assert_eq!(row.width, Some(1280));
        assert_eq!(row.height, Some(960));
        assert!(row.p_hash.is_some());
        assert!(row.sha1.is_some());
        assert!(row.analyzed_at.is_some());
        assert!(row.thumb.is_some(), "缩略图路径应写入");
        // 文件名带日期，应被识别为 filename 来源
        assert_eq!(row.dt_source.as_deref(), Some("filename"));
        assert_eq!(row.year, Some(1985));
        assert_eq!(row.month, Some(6));
    }

    #[test]
    fn analyze_thumb_file_actually_created() {
        let dir = work_dir("po_analyze_thumbfile");
        std::fs::copy(sample_dir().join("DSC_0071.JPG"), dir.join("t.jpg")).unwrap();
        let mut lib = Library::open_memory().unwrap();
        let cfg = Settings::new();
        scan_roots(&mut lib, &cfg, Some(&[dir.to_string_lossy().to_string()]), None, None, false);
        analyze_pending(&mut lib, &cfg, 0, None, None, 1, false);
        let row = lib.all_photos("", "id").remove(0);
        let thumb = row.thumb.expect("应有缩略图路径");
        assert!(Path::new(&thumb).exists(), "缩略图文件应真实存在: {thumb}");
        let _ = std::fs::remove_file(&thumb);
    }

    #[test]
    fn analyze_falls_back_to_fs_time() {
        let dir = work_dir("po_analyze_fs");
        std::fs::copy(sample_dir().join("DSC_0071.JPG"), dir.join("no_date_here.jpg")).unwrap();
        let mut lib = Library::open_memory().unwrap();
        let cfg = Settings::new();
        scan_roots(&mut lib, &cfg, Some(&[dir.to_string_lossy().to_string()]), None, None, false);
        analyze_pending(&mut lib, &cfg, 0, None, None, 1, false);
        let row = lib.all_photos("", "id").remove(0);
        assert_eq!(row.dt_source.as_deref(), Some("fs"));
        assert!(row.year.is_some());
    }

    #[test]
    fn analyze_handles_broken_file() {
        let dir = work_dir("po_analyze_broken");
        std::fs::write(dir.join("broken.jpg"), b"\xff\xd8\xff\xe0 not a jpeg").unwrap();
        let mut lib = Library::open_memory().unwrap();
        let cfg = Settings::new();
        scan_roots(&mut lib, &cfg, Some(&[dir.to_string_lossy().to_string()]), None, None, false);
        let r = analyze_pending(&mut lib, &cfg, 0, None, None, 1, false);
        assert_eq!(r.total, 1);
        assert_eq!(r.failed, 1, "损坏文件应计入失败但不崩溃");
        assert_eq!(r.done, 0);
    }

    #[test]
    fn analyze_incremental_skips_done() {
        let dir = work_dir("po_analyze_inc");
        std::fs::copy(sample_dir().join("DSC_0071.JPG"), dir.join("a.jpg")).unwrap();
        let mut lib = Library::open_memory().unwrap();
        let cfg = Settings::new();
        scan_roots(&mut lib, &cfg, Some(&[dir.to_string_lossy().to_string()]), None, None, false);
        let r1 = analyze_pending(&mut lib, &cfg, 0, None, None, 1, false);
        assert_eq!(r1.done, 1);
        // 再跑一次，没有待分析的
        let r2 = analyze_pending(&mut lib, &cfg, 0, None, None, 1, false);
        assert_eq!(r2.total, 0);
    }

    #[test]
    fn analyze_parallel_matches_serial() {
        let dir = work_dir("po_analyze_par");
        for i in 0..4 {
            std::fs::copy(
                sample_dir().join("DSC_0071.JPG"),
                dir.join(format!("p{i}.jpg")),
            )
            .unwrap();
        }
        let cfg = Settings::new();

        let mut lib1 = Library::open_memory().unwrap();
        scan_roots(&mut lib1, &cfg, Some(&[dir.to_string_lossy().to_string()]), None, None, false);
        let r1 = analyze_pending(&mut lib1, &cfg, 0, None, None, 1, false);

        let mut lib2 = Library::open_memory().unwrap();
        scan_roots(&mut lib2, &cfg, Some(&[dir.to_string_lossy().to_string()]), None, None, false);
        let r2 = analyze_pending(&mut lib2, &cfg, 0, None, None, 4, false);

        assert_eq!(r1.done, r2.done);
        assert_eq!(r1.done, 4);

        // 同一张图，并行与串行算出的哈希必须一致
        let a = lib1.all_photos("", "path");
        let b = lib2.all_photos("", "path");
        let ha: Vec<_> = a.iter().filter_map(|r| r.p_hash.clone()).collect();
        let hb: Vec<_> = b.iter().filter_map(|r| r.p_hash.clone()).collect();
        assert_eq!(ha, hb);
    }

    #[test]
    fn analyze_writes_place() {
        let dir = work_dir("po_analyze_place");
        std::fs::create_dir_all(dir.join("北京")).unwrap();
        std::fs::copy(sample_dir().join("DSC_0071.JPG"), dir.join("北京").join("a.jpg")).unwrap();
        let mut lib = Library::open_memory().unwrap();
        let cfg = Settings::new();
        scan_roots(&mut lib, &cfg, Some(&[dir.to_string_lossy().to_string()]), None, None, false);
        analyze_pending(&mut lib, &cfg, 0, None, None, 1, false);
        let row = lib.all_photos("", "id").remove(0);
        assert_eq!(row.place.as_deref(), Some("北京"));
    }

    #[test]
    fn progress_callback_is_called() {
        let dir = work_dir("po_progress");
        for i in 0..3 {
            std::fs::copy(sample_dir().join("DSC_0071.JPG"), dir.join(format!("g{i}.jpg"))).unwrap();
        }
        let mut lib = Library::open_memory().unwrap();
        let cfg = Settings::new();
        let root = dir.to_string_lossy().to_string();
        let hits = Arc::new(AtomicUsize::new(0));
        let hits2 = hits.clone();
        let cb: ProgressFn = Arc::new(move |_phase, _done, _total, _detail| {
            hits2.fetch_add(1, Ordering::Relaxed);
        });
        scan_roots(&mut lib, &cfg, Some(&[root]), Some(&cb), None, false);
        analyze_pending(&mut lib, &cfg, 0, Some(&cb), None, 1, false);
        assert!(hits.load(Ordering::Relaxed) > 0, "进度回调应被调用");
    }
}
