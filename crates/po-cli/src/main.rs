//! 照片整理 CLI。用于批处理与回归测试。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use po_core::util::human_size_i;
use po_core::Library;

fn main() -> ExitCode {
    // 视频封面走 Windows 系统缩略图接口（COM），调用链较重；
    // 主线程默认栈只有 1MB，跑大目录时会溢出。这里给足 16MB。
    let handle = std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            let args: Vec<String> = std::env::args().skip(1).collect();
            match run(&args) {
                Ok(code) => ExitCode::from(code),
                Err(e) => {
                    eprintln!("错误: {e}");
                    ExitCode::FAILURE
                }
            }
        })
        .expect("无法创建工作线程");
    handle.join().unwrap_or(ExitCode::FAILURE)
}

struct Opts {
    db: Option<PathBuf>,
    cmd: String,
    rest: Vec<String>,
}

fn parse_args(args: &[String]) -> Opts {
    let mut db = None;
    let mut cmd = String::new();
    let mut rest = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a == "--db" {
            i += 1;
            if i < args.len() {
                db = Some(PathBuf::from(&args[i]));
            }
        } else if let Some(v) = a.strip_prefix("--db=") {
            db = Some(PathBuf::from(v));
        } else if cmd.is_empty() {
            cmd = a.clone();
        } else {
            rest.push(a.clone());
        }
        i += 1;
    }
    Opts { db, cmd, rest }
}

fn open_lib(db: &Option<PathBuf>) -> anyhow::Result<Library> {
    match db {
        Some(p) => Library::open(p),
        None => Library::open_default(),
    }
}

fn run(args: &[String]) -> anyhow::Result<u8> {
    let o = parse_args(args);
    match o.cmd.as_str() {
        "" | "help" | "--help" | "-h" => {
            print_help();
            Ok(0)
        }
        "scan" => cmd_scan(&o),
        "analyze" => cmd_analyze(&o),
        "run" => cmd_run(&o),
        "dup" => cmd_dup(&o),
        "organize" => cmd_organize(&o),
        "rollback" => cmd_rollback(&o),
        "restore" => cmd_restore(&o),
        "report" => cmd_report(&o),
        "stats" => cmd_stats(&o),
        "list" => cmd_list(&o),
        "tags" => cmd_tags(&o),
        "roots" => cmd_roots(&o),
        "check" => cmd_check(&o),
        other => {
            eprintln!("未知命令: {other}");
            print_help();
            Ok(2)
        }
    }
}

fn print_help() {
    println!(
        r#"照片整理引擎 CLI

用法: po [--db <库文件>] <命令> [参数]

命令:
  scan <目录...>    扫描目录登记照片
      [--force]     忽略 mtime，强制重新登记
  analyze           分析照片（EXIF/质量/哈希/缩略图）
      [--limit N] [--redo] [--workers N]
  run <目录...>     一键全流程（扫描+分析+打标+地点+事件+去重）
      [--redo]
  dup               只重新查找重复照片
  organize          按分类整理归档
      --dest <目录> [--scheme 方案] [--template 模板]
      [--mode copy|move|link] [--on-conflict rename|skip|overwrite]
      [--favorite-dir] [--favorite-only] [--dry-run]
  rollback          撤销整理 [--batch Bxxxx]
  restore           批量修复老照片
      --out <目录> [--preset 名称] [--quality 1-100] [--limit N]
  report            导出报告 --html <文件> [--csv <文件>] [--title 标题]
  stats             显示资源库统计
  list [--limit N]  列出照片
  tags              列出标签及数量
  roots             列出已添加的照片目录
  check             校验资源库完整性（只读）

不加 --db 时使用默认库: %LOCALAPPDATA%\OldPhotoOrganizer\library.db"#
    );
}

fn cmd_scan(o: &Opts) -> anyhow::Result<u8> {
    let mut lib = open_lib(&o.db)?;
    let cfg = po_core::load_settings();
    let force = o.rest.iter().any(|a| a == "--force");
    let dirs: Vec<String> = o
        .rest
        .iter()
        .filter(|a| !a.starts_with("--"))
        .cloned()
        .collect();
    let roots = if dirs.is_empty() { None } else { Some(dirs.as_slice()) };
    let r = po_core::scanner::scan_roots(&mut lib, &cfg, roots, None, None, force);
    println!(
        "扫描完成：新登记 {}，跳过 {}，错误 {}（目录 {} 个，共见 {} 个文件）",
        r.added, r.skipped, r.errors, r.roots, r.seen
    );
    Ok(0)
}

fn cmd_analyze(o: &Opts) -> anyhow::Result<u8> {
    let mut lib = open_lib(&o.db)?;
    let cfg = po_core::load_settings();
    let limit = flag_usize(&o.rest, "--limit").unwrap_or(0);
    let workers = flag_usize(&o.rest, "--workers").unwrap_or(0);
    let redo = o.rest.iter().any(|a| a == "--redo");
    let r = po_core::scanner::analyze_pending(&mut lib, &cfg, limit, None, None, workers, redo);
    println!(
        "分析完成：{}/{}，失败 {}",
        r.done, r.total, r.failed
    );
    Ok(0)
}

fn cmd_run(o: &Opts) -> anyhow::Result<u8> {
    let mut lib = open_lib(&o.db)?;
    let cfg = po_core::load_settings();
    let redo = o.rest.iter().any(|a| a == "--redo");
    let dirs: Vec<String> = o
        .rest
        .iter()
        .filter(|a| !a.starts_with("--"))
        .cloned()
        .collect();
    let roots = if dirs.is_empty() { None } else { Some(dirs.as_slice()) };
    let prog = po_core::pipeline::Progress::new();
    let opts = po_core::pipeline::RunOptions {
        redo,
        ..Default::default()
    };
    println!("开始整理流程…");
    let r = po_core::pipeline::run_full(&mut lib, &cfg, roots, &prog, None, &opts);

    if let Some(s) = &r.scan {
        println!("  扫描    新登记 {}，跳过 {}", s.added, s.skipped);
    }
    if let Some(a) = &r.analyze {
        println!("  分析    {}/{}，失败 {}", a.done, a.total, a.failed);
    }
    if let Some(t) = &r.tag {
        println!("  打标    {} 张，{} 条标签", t.photos, t.tags);
    }
    if let Some(p) = &r.place {
        println!("  地点    更新 {} 张", p.updated);
    }
    if let Some(e) = &r.events {
        println!("  事件    {} 个，覆盖 {} 张", e.events, e.photos);
    }
    if let Some(d) = &r.dup {
        println!(
            "  去重    {} 组，{} 张，可省 {}",
            d.groups,
            d.photos,
            po_core::util::human_size_i(d.saved_bytes)
        );
    }
    match &r.stats {
        Some(s) => println!(
            "完成，用时 {}：共 {} 张，{}",
            po_core::util::fmt_seconds(r.elapsed),
            s.total,
            po_core::util::human_size_i(s.bytes)
        ),
        None => println!("完成，用时 {}", po_core::util::fmt_seconds(r.elapsed)),
    }

    if r.error.is_empty() {
        Ok(0)
    } else {
        eprintln!("流程出错: {}", r.error);
        Ok(1)
    }
}

fn cmd_dup(o: &Opts) -> anyhow::Result<u8> {
    let mut lib = open_lib(&o.db)?;
    let cfg = po_core::load_settings();
    let r = po_core::classify::find_duplicates(&mut lib, &cfg, None, None);
    println!(
        "重复检测：{} 组，{} 张，可省 {}（比较 {} 对）",
        r.groups,
        r.photos,
        po_core::util::human_size_i(r.saved_bytes),
        r.compared
    );
    Ok(0)
}

fn cmd_organize(o: &Opts) -> anyhow::Result<u8> {
    let mut lib = open_lib(&o.db)?;
    let dest = flag_str(&o.rest, "--dest").unwrap_or_default();
    let scheme = flag_str(&o.rest, "--scheme").unwrap_or_else(|| "year_month".into());
    let template = flag_str(&o.rest, "--template");
    let mode = flag_str(&o.rest, "--mode").unwrap_or_else(|| "copy".into());
    let conflict =
        flag_str(&o.rest, "--on-conflict").unwrap_or_else(|| "rename".into());
    let fav_dir = o.rest.iter().any(|a| a == "--favorite-dir");
    let fav_only = o.rest.iter().any(|a| a == "--favorite-only");
    let dry = o.rest.iter().any(|a| a == "--dry-run");

    let opts = po_core::organizer::PlanOptions {
        scheme: &scheme,
        template: template.as_deref(),
        parts: None,
        limit: 0,
        only_ids: None,
        favorite_dir: fav_dir,
        favorite_in_regular: !fav_only,
    };
    let plan = po_core::organizer::plan_organize(&lib, &opts);
    println!("计划整理 {} 张（方案 {scheme}，模式 {mode}）", plan.len());
    for (_, rel) in plan.iter().take(10) {
        println!("    {rel}");
    }
    if plan.len() > 10 {
        println!("    ... 共 {} 条", plan.len());
    }

    if dry {
        println!("（预览模式，未执行）");
        return Ok(0);
    }
    if dest.is_empty() {
        eprintln!("需要 --dest 目标目录");
        return Ok(2);
    }
    let r = po_core::organizer::execute_organize(
        &mut lib,
        std::path::Path::new(&dest),
        &plan,
        po_core::organizer::Mode::parse(&mode),
        po_core::organizer::Conflict::parse(&conflict),
        true,
        None,
        None,
    );
    println!(
        "完成：{} 张，跳过 {}，错误 {}，批次 {}",
        r.done, r.skipped, r.errors, r.batch
    );
    Ok(0)
}

fn cmd_rollback(o: &Opts) -> anyhow::Result<u8> {
    let mut lib = open_lib(&o.db)?;
    let batch = flag_str(&o.rest, "--batch");
    let r = po_core::organizer::rollback(&mut lib, batch.as_deref());
    println!("回滚：恢复 {}，错误 {}", r.restored, r.errors);
    Ok(0)
}

fn cmd_restore(o: &Opts) -> anyhow::Result<u8> {
    let lib = open_lib(&o.db)?;
    let dest = flag_str(&o.rest, "--out");
    let preset = flag_str(&o.rest, "--preset").unwrap_or_else(|| "标准修复".into());
    let quality: u8 = flag_usize(&o.rest, "--quality").unwrap_or(95) as u8;
    let limit = flag_usize(&o.rest, "--limit").unwrap_or(0);

    let Some(dest) = dest else {
        eprintln!("需要 --out 输出目录");
        return Ok(2);
    };
    let dest = std::path::PathBuf::from(dest);
    std::fs::create_dir_all(&dest)?;

    let opts = po_core::restore::preset_options(&preset);
    let mut rows = lib.all_photos("missing=0", "id");
    if limit > 0 {
        rows.truncate(limit);
    }
    let total = rows.len();
    let (mut done, mut errors) = (0, 0);
    for (i, r) in rows.iter().enumerate() {
        let Some(res) =
            po_core::restore::enhance_file(std::path::Path::new(&r.path), &opts)
        else {
            errors += 1;
            continue;
        };
        let fname = r.file_name();
        let stem = std::path::Path::new(&fname)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("photo");
        let out = dest.join(format!("{stem}_修复.jpg"));
        if po_core::restore::save_jpeg(&res.image, &out, quality) {
            done += 1;
        } else {
            errors += 1;
        }
        print!("\r  修复 {}/{}", i + 1, total);
        use std::io::Write;
        let _ = std::io::stdout().flush();
    }
    println!("\n修复完成 {done} 张（失败 {errors}）-> {}", dest.display());
    Ok(0)
}

fn cmd_report(o: &Opts) -> anyhow::Result<u8> {
    let lib = open_lib(&o.db)?;
    let html = flag_str(&o.rest, "--html");
    let csv = flag_str(&o.rest, "--csv");
    let title = flag_str(&o.rest, "--title").unwrap_or_else(|| "老照片整理报告".into());

    let has_csv = csv.is_some();
    if let Some(c) = csv {
        let n = po_core::report::export_csv(&lib, std::path::Path::new(&c))?;
        println!("CSV: {c}（{n} 行）");
    }
    match html {
        Some(h) => {
            let n = po_core::report::export_html(
                &lib,
                std::path::Path::new(&h),
                &title,
                600,
            )?;
            println!("HTML: {h}（{n} 张）");
        }
        None => {
            if !has_csv {
                eprintln!("需要 --html 或 --csv 至少一个");
                return Ok(2);
            }
        }
    }
    Ok(0)
}

fn flag_str(args: &[String], name: &str) -> Option<String> {
    for (i, a) in args.iter().enumerate() {
        if a == name {
            return args.get(i + 1).cloned();
        }
        if let Some(v) = a.strip_prefix(&format!("{name}=")) {
            return Some(v.to_string());
        }
    }
    None
}

fn cmd_stats(o: &Opts) -> anyhow::Result<u8> {
    let lib = open_lib(&o.db)?;
    let s = lib.stats();
    println!("资源库: {}", lib.path.display());
    println!("  照片总数   {}", s.total);
    println!("  已分析     {}", s.analyzed);
    println!("  有时间     {}", s.with_dt);
    println!("  缺时间     {}", s.no_dt);
    println!("  重复照片   {}", s.dups);
    println!("  已丢失     {}", s.missing);
    println!("  占用空间   {}", human_size_i(s.bytes));
    println!("  标签数     {}", s.tags);
    if !s.years.is_empty() {
        println!("  年份分布:");
        for (y, c) in s.years.iter().take(20) {
            println!("    {y}  {c}");
        }
        if s.years.len() > 20 {
            println!("    ... 共 {} 个年份", s.years.len());
        }
    }
    Ok(0)
}

fn cmd_list(o: &Opts) -> anyhow::Result<u8> {
    let limit = flag_usize(&o.rest, "--limit").unwrap_or(200);
    let lib = open_lib(&o.db)?;
    let rows = lib.all_photos("", "dt IS NULL, dt, id");
    println!(
        "{:<6} {:<19} {:<11} {:>6}  {}",
        "ID", "拍摄时间", "尺寸", "评分", "路径"
    );
    for r in rows.iter().take(limit) {
        let dt = r.dt.clone().unwrap_or_else(|| "-".into());
        let size = format!("{}x{}", r.width.unwrap_or(0), r.height.unwrap_or(0));
        let q = r.quality_score.unwrap_or(0.0);
        println!("{:<6} {:<19} {:<11} {:>6.0}  {}", r.id, dt, size, q, r.path);
    }
    println!("（共 {} 条，显示 {} 条）", rows.len(), rows.len().min(limit));
    Ok(0)
}

fn cmd_tags(o: &Opts) -> anyhow::Result<u8> {
    let lib = open_lib(&o.db)?;
    let tags = lib.all_tags();
    for (name, kind, cnt) in &tags {
        println!("{:<24} {:<8} {:>5}", name, kind, cnt);
    }
    println!("（共 {} 个标签）", tags.len());
    Ok(0)
}

fn cmd_roots(o: &Opts) -> anyhow::Result<u8> {
    let lib = open_lib(&o.db)?;
    let roots = lib.roots();
    if roots.is_empty() {
        println!("还没有添加照片目录");
    }
    for r in &roots {
        let exists = if Path::new(r).is_dir() { "" } else { "  [目录不存在]" };
        println!("{r}{exists}");
    }
    Ok(0)
}

/// 只读校验：检查 schema 完整性、孤儿记录、缺失文件等。
fn cmd_check(o: &Opts) -> anyhow::Result<u8> {
    let lib = open_lib(&o.db)?;
    let mut problems = 0usize;
    println!("校验: {}", lib.path.display());

    // 1. schema 列齐全
    let required = ["path", "dt", "year", "p_hash", "flags", "favorite", "place"];
    let cols = lib.photo_columns();
    for c in required {
        if !cols.contains(&c.to_string()) {
            println!("  [缺列] photos.{c}");
            problems += 1;
        }
    }
    if problems == 0 {
        println!("  [OK] photos 表列完整（{} 列）", cols.len());
    }

    // 2. 孤儿 photo_tags
    let orphan_tags = lib.query_count(
        "SELECT COUNT(*) FROM photo_tags WHERE photo_id NOT IN (SELECT id FROM photos)",
        &[],
    );
    if orphan_tags > 0 {
        println!("  [孤儿] photo_tags 有 {orphan_tags} 条指向不存在的照片");
        problems += 1;
    } else {
        println!("  [OK] photo_tags 无孤儿记录");
    }

    // 3. 同一 path 不应重复（UNIQUE 约束应已保证）
    let dup_paths = lib.query_count(
        "SELECT COUNT(*) FROM (SELECT path FROM photos GROUP BY path HAVING COUNT(*) > 1)",
        &[],
    );
    if dup_paths > 0 {
        println!("  [重复] 有 {dup_paths} 个重复 path");
        problems += 1;
    } else {
        println!("  [OK] path 无重复");
    }

    // 4. 已标记 missing 但文件其实还在
    let mut resurrected = 0;
    let missing_rows = lib.all_photos("missing=1", "id");
    for r in &missing_rows {
        if Path::new(&r.path).exists() {
            resurrected += 1;
        }
    }
    if resurrected > 0 {
        println!("  [提示] {resurrected} 张标记为丢失的照片文件其实还在（重新扫描可恢复）");
    }

    // 5. 已分析照片缺缩略图
    let no_thumb = lib.query_count(
        "SELECT COUNT(*) FROM photos WHERE analyzed_at IS NOT NULL AND (thumb IS NULL OR thumb='')",
        &[],
    );
    let analyzed = lib.query_count("SELECT COUNT(*) FROM photos WHERE analyzed_at IS NOT NULL", &[]);
    if no_thumb > 0 {
        println!("  [提示] {no_thumb}/{analyzed} 张已分析照片没有缩略图");
    } else if analyzed > 0 {
        println!("  [OK] {analyzed} 张已分析照片都有缩略图");
    }

    // 6. 缩略图文件是否真的存在（抽查前 200 张）
    let rows = lib.all_photos("thumb IS NOT NULL AND thumb<>''", "id");
    let sample: Vec<_> = rows.iter().take(200).collect();
    let missing_files = sample
        .iter()
        .filter(|r| {
            r.thumb
                .as_ref()
                .map(|t| !Path::new(t).exists())
                .unwrap_or(false)
        })
        .count();
    if missing_files > 0 {
        println!(
            "  [提示] 抽查 {} 张缩略图中 {missing_files} 个文件已不存在",
            sample.len()
        );
    } else if !sample.is_empty() {
        println!("  [OK] 抽查 {} 张缩略图文件均存在", sample.len());
    }

    // 7. 日期字段一致性：dt 有值但 year 为空
    let dt_no_year = lib.query_count(
        "SELECT COUNT(*) FROM photos WHERE dt IS NOT NULL AND dt<>'' AND year IS NULL",
        &[],
    );
    if dt_no_year > 0 {
        println!("  [不一致] {dt_no_year} 张有 dt 但缺 year");
        problems += 1;
    } else {
        println!("  [OK] dt 与 year 一致");
    }

    // 8. 重复组引用完整性
    let bad_dups = lib.query_count(
        "SELECT COUNT(*) FROM photos WHERE dup_group IS NOT NULL \
         AND dup_group NOT IN (SELECT id FROM dup_groups)",
        &[],
    );
    if bad_dups > 0 {
        println!("  [不一致] {bad_dups} 张照片引用了不存在的重复组");
        problems += 1;
    } else {
        println!("  [OK] 重复组引用完整");
    }

    println!(
        "\n结论: {}",
        if problems == 0 {
            "无结构性问题".to_string()
        } else {
            format!("发现 {problems} 个结构性问题")
        }
    );

    // 顺带打印年份/地点概览，方便人工核对
    print_summary(&lib);
    Ok(if problems == 0 { 0 } else { 1 })
}

fn print_summary(lib: &Library) {
    let mut stmt = match lib
        .conn()
        .prepare("SELECT name, kind, COUNT(*) c FROM tags GROUP BY kind ORDER BY c DESC")
    {
        Ok(s) => s,
        Err(_) => return,
    };
    let kinds: Vec<(String, String, i64)> = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                r.get::<_, i64>(2)?,
            ))
        })
        .map(|it| it.flatten().collect())
        .unwrap_or_default();
    if kinds.is_empty() {
        return;
    }
    let mut by_kind: HashMap<String, i64> = HashMap::new();
    for (_, kind, _) in &kinds {
        *by_kind.entry(kind.clone()).or_insert(0) += 1;
    }
    let mut v: Vec<_> = by_kind.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1));
    println!("\n标签类型分布:");
    for (k, c) in v {
        println!("  {:<10} {} 个", if k.is_empty() { "未分类" } else { &k }, c);
    }
}

fn flag_usize(args: &[String], name: &str) -> Option<usize> {
    for (i, a) in args.iter().enumerate() {
        if a == name {
            return args.get(i + 1).and_then(|v| v.parse().ok());
        }
        if let Some(v) = a.strip_prefix(&format!("{name}=")) {
            return v.parse().ok();
        }
    }
    None
}
