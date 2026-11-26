//! 一体化流水线：扫描 -> 分析 -> 打标 -> 地点 -> 事件 -> 去重。
//!
//! 与 Python 版 `opo/pipeline.py` 对齐。进度按阶段加权换算成整体百分比。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::config::Settings;
use crate::library::Library;
use crate::scanner::{self, CancelFlag, ProgressFn};
use crate::util::{fmt_seconds, human_size_i};

/// 阶段 -> (起始百分比, 该阶段占的宽度)
///
/// 权重按实际耗时占比给：分析照片最重（截图解码慢），
/// 去重和打标很轻，扫描目录只是遍历文件名。
pub const PHASE_SPAN: &[(&str, f64, f64)] = &[
    ("扫描目录", 0.0, 10.0),
    ("分析照片", 10.0, 60.0),
    ("生成标签", 70.0, 8.0),
    ("识别地点", 78.0, 6.0),
    ("聚类事件", 84.0, 4.0),
    ("查找重复", 88.0, 11.0),
    ("完成", 99.0, 1.0),
];

/// 把各阶段进度换算成整体 0-100，避免每个阶段都从 1% 重新开始。
pub fn overall_percent(phase: &str, done: usize, total: usize) -> f64 {
    if phase.starts_with("完成") {
        return 100.0;
    }
    for (prefix, start, width) in PHASE_SPAN {
        if phase.starts_with(prefix) || phase.contains(prefix) {
            if total == 0 {
                return round1(*start);
            }
            let frac = (done as f64 / total as f64).clamp(0.0, 1.0);
            return round1(start + width * frac);
        }
    }
    0.0
}

fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

/// 线程安全的进度快照，供 UI 轮询。
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ProgressSnapshot {
    pub running: bool,
    pub phase: String,
    pub done: usize,
    pub total: usize,
    pub detail: String,
    pub percent: f64,
    pub elapsed: f64,
    pub elapsed_h: String,
    pub error: String,
    pub cancelled: bool,
}

/// 进度汇聚器：内部状态 + 可选的外部回调。
#[derive(Clone)]
pub struct Progress {
    inner: Arc<Mutex<ProgressInner>>,
    callback: Option<ProgressFn>,
}

struct ProgressInner {
    running: bool,
    phase: String,
    done: usize,
    total: usize,
    detail: String,
    percent: f64,
    error: String,
    cancelled: bool,
    started: f64,
    finished: f64,
}

impl Default for Progress {
    fn default() -> Self {
        Self::new()
    }
}

impl Progress {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(ProgressInner {
                running: false,
                phase: String::new(),
                done: 0,
                total: 0,
                detail: String::new(),
                percent: 0.0,
                error: String::new(),
                cancelled: false,
                started: 0.0,
                finished: 0.0,
            })),
            callback: None,
        }
    }

    /// 带外部回调（HTTP 进度接口 / Tauri 事件）构造。
    pub fn with_callback(cb: ProgressFn) -> Self {
        let p = Self::new();
        Self {
            callback: Some(cb),
            ..p
        }
    }

    pub fn reset(&self) {
        if let Ok(mut g) = self.inner.lock() {
            g.running = true;
            g.phase = "准备中".into();
            g.done = 0;
            g.total = 0;
            g.detail = String::new();
            g.percent = 0.0;
            g.error = String::new();
            g.cancelled = false;
            g.started = crate::library::now_ts();
            g.finished = 0.0;
        }
    }

    pub fn update(&self, phase: &str, done: usize, total: usize, detail: &str) {
        if let Ok(mut g) = self.inner.lock() {
            g.phase = phase.to_string();
            g.done = done;
            if total > 0 {
                g.total = total;
            }
            g.detail = detail.to_string();
            g.percent = overall_percent(phase, done, total);
        }
        if let Some(cb) = &self.callback {
            cb(phase, done, total, detail);
        }
    }

    pub fn finish(&self, error: &str) {
        self.finish_with_status(error, false);
    }

    pub fn cancel(&self) {
        self.finish_with_status("", true);
    }

    fn finish_with_status(&self, error: &str, cancelled: bool) {
        if let Ok(mut g) = self.inner.lock() {
            g.running = false;
            g.error = error.to_string();
            g.cancelled = cancelled;
            g.finished = crate::library::now_ts();
            if error.is_empty() && !cancelled {
                g.percent = 100.0;
            }
        }
    }

    pub fn snapshot(&self) -> ProgressSnapshot {
        let Ok(g) = self.inner.lock() else {
            return ProgressSnapshot::default();
        };
        let now = crate::library::now_ts();
        let started = if g.started > 0.0 { g.started } else { now };
        let end = if g.finished > 0.0 { g.finished } else { now };
        let elapsed = (end - started).max(0.0);
        ProgressSnapshot {
            running: g.running,
            phase: g.phase.clone(),
            done: g.done,
            total: g.total,
            detail: g.detail.clone(),
            percent: g.percent,
            elapsed,
            elapsed_h: fmt_seconds(elapsed),
            error: g.error.clone(),
            cancelled: g.cancelled,
        }
    }

    pub fn is_running(&self) -> bool {
        self.inner.lock().map(|g| g.running).unwrap_or(false)
    }
}

/// 全流程开关
#[derive(Debug, Clone)]
pub struct RunOptions {
    pub do_scan: bool,
    pub do_analyze: bool,
    pub do_tag: bool,
    pub do_events: bool,
    pub do_dup: bool,
    pub redo: bool,
    pub reanalyze_all: bool,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            do_scan: true,
            do_analyze: true,
            do_tag: true,
            do_events: true,
            do_dup: true,
            redo: false,
            reanalyze_all: false,
        }
    }
}

/// 各阶段结果汇总
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct RunReport {
    pub scan: Option<scanner::ScanResult>,
    pub analyze: Option<scanner::AnalyzeResult>,
    pub tag: Option<crate::classify::TagResult>,
    pub place: Option<crate::classify::PlaceResult>,
    pub events: Option<crate::classify::EventResult>,
    pub dup: Option<crate::classify::DupResult>,
    pub stats: Option<crate::library::Stats>,
    pub elapsed: f64,
    pub error: String,
    pub cancelled: bool,
}

/// 跑完整流程。
pub fn run_full(
    lib: &mut Library,
    cfg: &Settings,
    roots: Option<&[String]>,
    prog: &Progress,
    cancel: Option<&CancelFlag>,
    opts: &RunOptions,
) -> RunReport {
    prog.reset();
    let t0 = crate::library::now_ts();
    let mut report = RunReport::default();

    let cancelled = |cancel: Option<&CancelFlag>| {
        cancel.map(|c| c.load(Ordering::Relaxed)).unwrap_or(false)
    };
    let result = (|| -> Result<(), String> {
        if cancelled(cancel) {
            report.cancelled = true;
            return Ok(());
        }
        if opts.do_scan {
            let roots_owned: Vec<String> = match roots {
                Some(r) => r.to_vec(),
                None => lib.roots(),
            };
            if roots_owned.is_empty() {
                prog.update("扫描目录", 0, 0, "没有设置任何照片目录");
                return Err("没有设置任何照片目录".into());
            }
            prog.update("扫描目录", 0, 0, "正在枚举文件…");
            let cb = ProgressFn::from(Arc::new({
                let p = prog.clone();
                move |_phase: &str, done: usize, total: usize, detail: &str| {
                    p.update("扫描目录", done, total, detail);
                }
            }) as Arc<dyn Fn(&str, usize, usize, &str) + Send + Sync>);
            let r = scanner::scan_roots(
                lib,
                cfg,
                Some(&roots_owned),
                Some(&cb),
                cancel,
                opts.redo,
            );
            prog.update(
                "扫描目录",
                r.added,
                r.added.max(1),
                &format!("新登记 {} 个文件", r.added),
            );
            report.scan = Some(r);
        }
        if cancelled(cancel) {
            report.cancelled = true;
            return Ok(());
        }

        if opts.do_analyze {
            let cb = ProgressFn::from(Arc::new({
                let p = prog.clone();
                move |_phase: &str, done: usize, total: usize, detail: &str| {
                    p.update("分析照片", done, total, detail);
                }
            }) as Arc<dyn Fn(&str, usize, usize, &str) + Send + Sync>);
            let r = scanner::analyze_pending(
                lib,
                cfg,
                0,
                Some(&cb),
                cancel,
                cfg.usize("analyze_workers"),
                opts.redo || opts.reanalyze_all,
            );
            report.analyze = Some(r);
        }
        if cancelled(cancel) {
            report.cancelled = true;
            return Ok(());
        }

        if opts.do_tag {
            let cb = ProgressFn::from(Arc::new({
                let p = prog.clone();
                move |_phase: &str, done: usize, total: usize, detail: &str| {
                    p.update("生成标签", done, total, detail);
                }
            }) as Arc<dyn Fn(&str, usize, usize, &str) + Send + Sync>);
            let r = crate::classify::auto_tag(lib, cfg, Some(&cb), cancel);
            report.tag = Some(r);

            prog.update("识别地点", 0, 0, "按 GPS 与目录名推断地点…");
            let cb2 = ProgressFn::from(Arc::new({
                let p = prog.clone();
                move |_phase: &str, done: usize, total: usize, detail: &str| {
                    p.update("识别地点", done, total, detail);
                }
            }) as Arc<dyn Fn(&str, usize, usize, &str) + Send + Sync>);
            report.place = Some(crate::classify::backfill_places(lib, Some(&cb2), false));
        }

        if opts.do_events && !cancelled(cancel) {
            prog.update("聚类事件", 0, 1, "按时间与地点分组…");
            report.events = Some(crate::classify::cluster_events(lib, cfg));
        }

        if opts.do_dup && !cancelled(cancel) {
            prog.update("查找重复", 0, 0, "计算感知哈希相似度…");
            let cb = ProgressFn::from(Arc::new({
                let p = prog.clone();
                move |_phase: &str, done: usize, total: usize, detail: &str| {
                    p.update("查找重复", done, total, detail);
                }
            }) as Arc<dyn Fn(&str, usize, usize, &str) + Send + Sync>);
            report.dup = Some(crate::classify::find_duplicates(lib, cfg, Some(&cb), cancel));
        }

        Ok(())
    })();

    report.elapsed = crate::library::now_ts() - t0;
    match result {
        Ok(()) if report.cancelled || cancelled(cancel) => {
            report.cancelled = true;
            report.stats = Some(lib.stats());
            prog.update("已取消", 0, 0, "已完成的分析已保留，下次可继续扫描");
            prog.cancel();
        }
        Ok(()) => {
            // 记下这次扫描完成的时间，供下次启动时判断"有没有上次的记录"
            let _ = lib.set_meta("last_scan_at", &crate::library::now_ts().to_string());
            report.stats = Some(lib.stats());
            let st = report.stats.as_ref().unwrap();
            let total = (st.total.max(1)) as usize;
            prog.update(
                "完成",
                total,
                total,
                &format!(
                    "共 {} 张，{}，用时 {}",
                    st.total,
                    human_size_i(st.bytes),
                    fmt_seconds(report.elapsed)
                ),
            );
            prog.finish("");
        }
        Err(e) => {
            report.error = e.clone();
            prog.update("出错", 0, 0, &e);
            prog.finish(&e);
        }
    }
    report
}

/// 取消标志辅助
pub fn new_cancel() -> CancelFlag {
    Arc::new(AtomicBool::new(false))
}

pub fn is_cancelled(c: &CancelFlag) -> bool {
    c.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn percent_weights_cover_full_range() {
        assert_eq!(overall_percent("完成", 1, 1), 100.0);
        assert_eq!(overall_percent("扫描目录", 0, 0), 0.0);
        // 分析照片占 10~70
        let p50 = overall_percent("分析照片", 50, 100);
        assert!((p50 - 40.0).abs() < 0.1, "{p50}");
        let p100 = overall_percent("分析照片", 100, 100);
        assert!((p100 - 70.0).abs() < 0.1, "{p100}");
        // 去重 88~99
        let d = overall_percent("查找重复", 100, 100);
        assert!((d - 99.0).abs() < 0.1, "{d}");
    }

    #[test]
    fn percent_monotonic_across_phases() {
        let phases = [
            ("扫描目录", 10usize),
            ("分析照片", 100),
            ("生成标签", 50),
            ("识别地点", 20),
            ("聚类事件", 5),
            ("查找重复", 40),
        ];
        let mut last = -1.0;
        for (ph, total) in phases {
            let p = overall_percent(ph, total, total);
            assert!(p >= last, "{ph} 的 {p} 应不小于前一阶段 {last}");
            last = p;
        }
    }

    #[test]
    fn unknown_phase_returns_zero() {
        assert_eq!(overall_percent("不存在的阶段", 5, 10), 0.0);
    }

    #[test]
    fn progress_snapshot_tracks_state() {
        let p = Progress::new();
        assert!(!p.snapshot().running);
        p.reset();
        let s = p.snapshot();
        assert!(s.running);
        assert_eq!(s.phase, "准备中");

        p.update("分析照片", 5, 10, "a.jpg");
        let s = p.snapshot();
        assert_eq!(s.done, 5);
        assert_eq!(s.total, 10);
        assert_eq!(s.detail, "a.jpg");
        assert!(s.percent > 10.0 && s.percent < 70.0);

        p.finish("");
        let s = p.snapshot();
        assert!(!s.running);
        assert_eq!(s.percent, 100.0);
        assert!(s.error.is_empty());
    }

    #[test]
    fn progress_records_error() {
        let p = Progress::new();
        p.reset();
        p.finish("磁盘满了");
        let s = p.snapshot();
        assert!(!s.running);
        assert_eq!(s.error, "磁盘满了");
        assert_ne!(s.percent, 100.0);
    }

    #[test]
    fn progress_callback_fires() {
        let hits = Arc::new(Mutex::new(Vec::<(String, usize, usize)>::new()));
        let h2 = hits.clone();
        let cb: ProgressFn = Arc::new(move |phase, done, total, _detail| {
            if let Ok(mut v) = h2.lock() {
                v.push((phase.to_string(), done, total));
            }
        });
        let p = Progress::with_callback(cb);
        p.reset();
        p.update("分析照片", 1, 2, "");
        p.update("分析照片", 2, 2, "");
        let v = hits.lock().unwrap();
        assert_eq!(v.len(), 2);
        assert_eq!(v[1].0, "分析照片");
        assert_eq!(v[1].2, 2);
    }

    #[test]
    fn empty_roots_reports_error() {
        let mut lib = Library::open_memory().unwrap();
        let cfg = Settings::new();
        let prog = Progress::new();
        let opts = RunOptions::default();
        let r = run_full(&mut lib, &cfg, Some(&[]), &prog, None, &opts);
        assert!(!r.error.is_empty());
        assert!(r.error.contains("没有设置任何照片目录"));
        assert!(!prog.snapshot().running);
    }

    #[test]
    fn full_pipeline_on_samples() {
        let dir = work_dir("po_pipeline");
        for n in [
            "1985-06-12 春节全家福.jpg",
            "1992-10-01 旅行.jpg",
            "DSC_0071.JPG",
            "2004年/照片 001.jpg",
        ] {
            let src = sample_dir().join(n);
            let dst = dir.join(src.file_name().unwrap());
            std::fs::copy(&src, &dst).unwrap();
        }
        // 重复样本
        std::fs::create_dir_all(dir.join("dup")).unwrap();
        std::fs::copy(
            sample_dir().join("重复/完全相同1.jpg"),
            dir.join("dup").join("完全相同1.jpg"),
        )
        .unwrap();

        let mut lib = Library::open_memory().unwrap();
        let cfg = Settings::new();
        let prog = Progress::new();
        let root = dir.to_string_lossy().to_string();
        let r = run_full(
            &mut lib,
            &cfg,
            Some(&[root]),
            &prog,
            None,
            &RunOptions::default(),
        );
        assert!(r.error.is_empty(), "流程出错: {}", r.error);

        let scan = r.scan.unwrap();
        assert_eq!(scan.added, 5);

        let analyze = r.analyze.unwrap();
        assert_eq!(analyze.done, 5, "{analyze:?}");

        let st = r.stats.unwrap();
        assert_eq!(st.total, 5);
        assert_eq!(st.analyzed, 5);

        // 打标应该有产出
        let tag = r.tag.unwrap();
        assert!(tag.photos > 0);

        // 完全相同的两张应被识别为重复
        let dup = r.dup.unwrap();
        assert!(dup.groups >= 1, "应至少发现 1 组重复: {dup:?}");

        let snap = prog.snapshot();
        assert!(!snap.running);
        assert_eq!(snap.percent, 100.0);
    }

    #[test]
    fn pipeline_respects_cancel() {
        let dir = work_dir("po_pipeline_cancel");
        std::fs::copy(sample_dir().join("DSC_0071.JPG"), dir.join("a.jpg")).unwrap();
        let mut lib = Library::open_memory().unwrap();
        let cfg = Settings::new();
        let prog = Progress::new();
        let cancel = new_cancel();
        cancel.store(true, Ordering::Relaxed);
        let r = run_full(
            &mut lib,
            &cfg,
            Some(&[dir.to_string_lossy().to_string()]),
            &prog,
            Some(&cancel),
            &RunOptions::default(),
        );
        // 取消后不应崩溃，也不把取消当成一次完整扫描
        assert!(r.cancelled);
        assert_eq!(lib.count(""), 0);
    }

    #[test]
    fn pipeline_skips_phases_that_are_off() {
        let dir = work_dir("po_pipeline_partial");
        std::fs::copy(sample_dir().join("DSC_0071.JPG"), dir.join("a.jpg")).unwrap();
        let mut lib = Library::open_memory().unwrap();
        let cfg = Settings::new();
        let prog = Progress::new();
        let opts = RunOptions {
            do_tag: false,
            do_events: false,
            do_dup: false,
            ..Default::default()
        };
        let r = run_full(
            &mut lib,
            &cfg,
            Some(&[dir.to_string_lossy().to_string()]),
            &prog,
            None,
            &opts,
        );
        assert!(r.error.is_empty());
        assert!(r.scan.is_some());
        assert!(r.analyze.is_some());
        assert!(r.tag.is_none(), "打标应被跳过");
        assert!(r.events.is_none(), "事件应被跳过");
        assert!(r.dup.is_none(), "去重应被跳过");
    }

    #[test]
    fn second_run_is_incremental() {
        let dir = work_dir("po_pipeline_inc");
        std::fs::copy(sample_dir().join("DSC_0071.JPG"), dir.join("a.jpg")).unwrap();
        let mut lib = Library::open_memory().unwrap();
        let cfg = Settings::new();
        let prog = Progress::new();
        let root = dir.to_string_lossy().to_string();
        let r1 = run_full(&mut lib, &cfg, Some(&[root.clone()]), &prog, None, &RunOptions::default());
        assert!(r1.error.is_empty());
        // 再跑：没有新增、没有待分析
        let r2 = run_full(&mut lib, &cfg, Some(&[root]), &prog, None, &RunOptions::default());
        assert!(r2.error.is_empty());
        assert_eq!(r2.scan.unwrap().added, 0);
        assert_eq!(r2.analyze.unwrap().total, 0);
    }
}
