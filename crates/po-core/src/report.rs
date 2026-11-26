//! 报告导出：HTML 相册清单、CSV 元数据表。
//!
//! 与 Python 版 `opo/report.py` 对齐。

use std::fmt::Write as _;
use std::path::Path;

use crate::library::Library;
use crate::util::human_size_i;

/// CSV 导出的列（与 Python 版一致）
pub const CSV_COLS: &[&str] = &[
    "id",
    "path",
    "name",
    "ext",
    "size",
    "dt",
    "dt_source",
    "year",
    "month",
    "width",
    "height",
    "megapixels",
    "quality_score",
    "age_score",
    "blur",
    "brightness",
    "contrast",
    "saturation",
    "sepia",
    "noise",
    "flags",
    "camera_make",
    "camera_model",
    "lat",
    "lon",
    "dup_group",
    "is_best",
    "thumb",
    "kind",
    "duration",
];

/// CSV 字段转义：含逗号、引号或换行时用双引号包裹，内部引号翻倍。
fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') || s.contains('\r') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// 导出 CSV。带 UTF-8 BOM，Excel 打开不乱码。
pub fn export_csv(lib: &Library, out_path: &Path) -> std::io::Result<usize> {
    let rows = lib.all_photos("", "dt IS NULL, dt, id");
    let mut s = String::new();
    s.push('\u{feff}');

    let mut header: Vec<String> = CSV_COLS.iter().map(|c| c.to_string()).collect();
    header.push("tags".into());
    header.push("size_h".into());
    s.push_str(&header.join(","));
    s.push_str("\r\n");

    for r in &rows {
        let mut cells: Vec<String> = Vec::with_capacity(CSV_COLS.len());
        for col in CSV_COLS {
            let v = match *col {
                "id" => r.id.to_string(),
                "path" => r.path.clone(),
                "name" => r.name.clone().unwrap_or_default(),
                "ext" => r.ext.clone().unwrap_or_default(),
                "size" => r.size.map(|v| v.to_string()).unwrap_or_default(),
                "dt" => r.dt.clone().unwrap_or_default(),
                "dt_source" => r.dt_source.clone().unwrap_or_default(),
                "year" => r.year.map(|v| v.to_string()).unwrap_or_default(),
                "month" => r.month.map(|v| v.to_string()).unwrap_or_default(),
                "width" => r.width.map(|v| v.to_string()).unwrap_or_default(),
                "height" => r.height.map(|v| v.to_string()).unwrap_or_default(),
                "megapixels" => r.megapixels.map(|v| format!("{v}")).unwrap_or_default(),
                "quality_score" => r.quality_score.map(|v| format!("{v}")).unwrap_or_default(),
                "age_score" => r.age_score.map(|v| format!("{v}")).unwrap_or_default(),
                "blur" => r.blur.map(|v| format!("{v}")).unwrap_or_default(),
                "brightness" => r.brightness.map(|v| format!("{v}")).unwrap_or_default(),
                "contrast" => r.contrast.map(|v| format!("{v}")).unwrap_or_default(),
                "saturation" => r.saturation.map(|v| format!("{v}")).unwrap_or_default(),
                "sepia" => r.sepia.map(|v| format!("{v}")).unwrap_or_default(),
                "noise" => r.noise.map(|v| format!("{v}")).unwrap_or_default(),
                "flags" => r.flags.clone().unwrap_or_default(),
                "camera_make" => r.camera_make.clone().unwrap_or_default(),
                "camera_model" => r.camera_model.clone().unwrap_or_default(),
                "lat" => r.lat.map(|v| format!("{v}")).unwrap_or_default(),
                "lon" => r.lon.map(|v| format!("{v}")).unwrap_or_default(),
                "dup_group" => r.dup_group.map(|v| v.to_string()).unwrap_or_default(),
                "is_best" => r.is_best.map(|v| v.to_string()).unwrap_or_default(),
                "thumb" => r.thumb.clone().unwrap_or_default(),
                "kind" => r.kind.clone().unwrap_or_else(|| "photo".into()),
                "duration" => r.duration.map(|v| format!("{v:.2}")).unwrap_or_default(),
                _ => String::new(),
            };
            cells.push(csv_escape(&v));
        }
        cells.push(csv_escape(&lib.tags_of(r.id).join("|")));
        cells.push(human_size_i(r.size.unwrap_or(0)));
        s.push_str(&cells.join(","));
        s.push_str("\r\n");
    }

    if let Some(parent) = out_path.parent() {
        if !parent.as_os_str().is_empty() {
            crate::config::ensure_dir(parent.to_path_buf());
        }
    }
    std::fs::write(out_path, s)?;
    Ok(rows.len())
}

const CSS: &str = r#"
body{font-family:"Microsoft YaHei","PingFang SC",system-ui,sans-serif;background:#14161a;color:#e8e8ea;margin:0;padding:24px}
h1{font-size:22px;margin:0 0 6px}
.sub{color:#8b93a1;font-size:13px;margin-bottom:18px}
.stats{display:flex;flex-wrap:wrap;gap:10px;margin-bottom:22px}
.card{background:#1d2026;border:1px solid #2a2f38;border-radius:10px;padding:10px 14px;min-width:110px}
.card b{display:block;font-size:19px;color:#7fd1c1}
.card span{font-size:12px;color:#8b93a1}
h2{font-size:16px;margin:26px 0 10px;border-left:3px solid #7fd1c1;padding-left:8px}
.grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(168px,1fr));gap:12px}
.item{background:#1d2026;border:1px solid #2a2f38;border-radius:8px;overflow:hidden}
.item img{width:100%;height:128px;object-fit:cover;display:block;background:#0e1013}
.meta{padding:6px 8px;font-size:11px;color:#98a0ac;line-height:1.5}
.meta b{color:#d6dae2;font-weight:500;display:block;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.tag{display:inline-block;background:#26303a;color:#8fd4c6;border-radius:4px;padding:0 4px;margin:1px 2px 0 0;font-size:10px}
.tag.vid{background:#3a2a26;color:#ffb088}
table{border-collapse:collapse;width:100%;font-size:12px}
td,th{border-bottom:1px solid #262b33;padding:6px 8px;text-align:left}
th{color:#8b93a1;font-weight:500}
"#;

/// HTML 转义，避免路径/标签里的尖括号破坏结构。
fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// 导出 HTML 报告。
pub fn export_html(
    lib: &Library,
    out_path: &Path,
    title: &str,
    max_items: usize,
) -> std::io::Result<usize> {
    let rows = lib.all_photos("", "dt IS NULL, dt, id");
    let st = lib.stats();
    let out_dir = out_path.parent().unwrap_or(Path::new("."));

    let mut h = String::new();
    h.push_str("<!doctype html><html lang='zh-CN'><head><meta charset='utf-8'>");
    let _ = write!(h, "<title>{}</title><style>{}</style></head><body>", esc(title), CSS);
    let _ = write!(h, "<h1>{}</h1>", esc(title));
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    let db_name = lib
        .path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let _ = write!(
        h,
        "<div class='sub'>生成时间 {} · 资源库 {}</div>",
        now,
        esc(&db_name)
    );

    // 统计卡片
    h.push_str("<div class='stats'>");
    let cards: [(&str, String); 7] = [
        ("照片总数", st.total.to_string()),
        ("已分析", st.analyzed.to_string()),
        ("有时间", st.with_dt.to_string()),
        ("缺时间", st.no_dt.to_string()),
        ("重复照片", st.dups.to_string()),
        ("占用空间", human_size_i(st.bytes)),
        ("标签数", st.tags.to_string()),
    ];
    for (label, val) in cards {
        let _ = write!(
            h,
            "<div class='card'><b>{}</b><span>{}</span></div>",
            esc(&val),
            esc(label)
        );
    }
    h.push_str("</div>");

    // 事件
    let events = lib.recent_events(30);
    if !events.is_empty() {
        h.push_str(
            "<h2>事件分组（最近 30 个）</h2><table><tr><th>事件</th><th>照片数</th><th>开始时间</th></tr>",
        );
        for (title, count, start) in events {
            let _ = write!(
                h,
                "<tr><td>{}</td><td>{}</td><td>{}</td></tr>",
                esc(&title),
                count,
                esc(&start.chars().take(10).collect::<String>())
            );
        }
        h.push_str("</table>");
    }

    // 重复组
    let dups = lib.top_dup_groups(20);
    if !dups.is_empty() {
        h.push_str(
            "<h2>重复照片组</h2><table><tr><th>组</th><th>类型</th><th>张数</th><th>可节省</th></tr>",
        );
        for (id, kind, count, saved) in dups {
            let k = if kind == "exact" { "完全相同" } else { "近似" };
            let _ = write!(
                h,
                "<tr><td>#{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                id,
                k,
                count,
                human_size_i(saved)
            );
        }
        h.push_str("</table>");
    }

    // 年份分布
    if !st.years.is_empty() {
        h.push_str("<h2>年份分布</h2><table><tr><th>年份</th><th>数量</th></tr>");
        for (y, c) in &st.years {
            let _ = write!(h, "<tr><td>{}</td><td>{}</td></tr>", y, c);
        }
        h.push_str("</table>");
    }

    // 照片墙
    let _ = write!(
        h,
        "<h2>照片清单（前 {} 张）</h2><div class='grid'>",
        max_items
    );
    for r in rows.iter().take(max_items) {
        // 缩略图优先用相对路径，换机器也能打开
        let src = match &r.thumb {
            Some(t) if Path::new(t).exists() => relativize(Path::new(t), out_dir),
            _ => String::new(),
        };
        let tags = lib.tags_of(r.id);
        let tag_html: String = tags
            .iter()
            .take(6)
            .map(|t| format!("<span class='tag'>{}</span>", esc(t)))
            .collect();
        let name = Path::new(&r.path)
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        // 视频额外标出时长
        let dur_badge = match (r.is_video(), r.duration_text()) {
            (true, Some(d)) => format!("<span class='tag vid'>▶ {}</span>", esc(&d)),
            (true, None) => "<span class='tag vid'>▶ 视频</span>".to_string(),
            _ => String::new(),
        };
        let _ = write!(
            h,
            "<div class='item'><img loading='lazy' src='{}' alt=''>\
             <div class='meta'><b>{}</b>{}<br>{}x{} · {} {}<br>{}</div></div>",
            esc(&src),
            esc(&name),
            esc(r.dt.as_deref().unwrap_or("无日期")),
            r.width.unwrap_or(0),
            r.height.unwrap_or(0),
            human_size_i(r.size.unwrap_or(0)),
            dur_badge,
            tag_html
        );
    }
    h.push_str("</div></body></html>");

    if let Some(parent) = out_path.parent() {
        if !parent.as_os_str().is_empty() {
            crate::config::ensure_dir(parent.to_path_buf());
        }
    }
    std::fs::write(out_path, h)?;
    Ok(rows.len().min(max_items))
}

/// 尽量算相对路径，失败就退回 `file:///` 绝对路径。
fn relativize(target: &Path, base: &Path) -> String {
    let (Ok(t), Ok(b)) = (
        std::fs::canonicalize(target),
        std::fs::canonicalize(base),
    ) else {
        return format!("file:///{}", target.to_string_lossy().replace('\\', "/"));
    };
    match pathdiff(&t, &b) {
        Some(p) => p.to_string_lossy().replace('\\', "/"),
        None => format!("file:///{}", t.to_string_lossy().replace('\\', "/")),
    }
}

/// 计算 `target` 相对 `base` 的路径。
fn pathdiff(target: &Path, base: &Path) -> Option<std::path::PathBuf> {
    let t: Vec<_> = target.components().collect();
    let b: Vec<_> = base.components().collect();
    let common = t
        .iter()
        .zip(b.iter())
        .take_while(|(a, c)| a == c)
        .count();
    if common == 0 {
        return None;
    }
    let mut out = std::path::PathBuf::new();
    for _ in common..b.len() {
        out.push("..");
    }
    for c in &t[common..] {
        out.push(c.as_os_str());
    }
    if out.as_os_str().is_empty() {
        out.push(".");
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::PhotoUpdate;
    use std::path::PathBuf;

    fn work_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn seed(lib: &mut Library) {
        let mut u = PhotoUpdate::new(r"D:\photos\a.jpg");
        u.text("name", Some("a.jpg"))
            .text("dt", Some("1985-06-12 10:00:00"))
            .int("year", Some(1985))
            .int("month", Some(6))
            .int("width", Some(1280))
            .int("height", Some(960))
            .int("size", Some(500_000))
            .real("quality_score", Some(75.0))
            .text("flags", Some("模糊|过暗"));
        let pid = lib.upsert_photo(&u).unwrap();
        lib.add_tags_bulk(&[(pid, "全家福".into()), (pid, "节日".into())], "auto", "auto");
    }

    #[test]
    fn csv_has_bom_and_header() {
        let dir = work_dir("po_csv");
        let mut lib = Library::open_memory().unwrap();
        seed(&mut lib);
        let p = dir.join("photos.csv");
        let n = export_csv(&lib, &p).unwrap();
        assert_eq!(n, 1);

        let raw = std::fs::read(&p).unwrap();
        assert_eq!(&raw[..3], b"\xef\xbb\xbf", "应带 UTF-8 BOM");

        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.contains("id,path,name"));
        assert!(text.contains("全家福|节日"), "标签应合并导出");
    }

    #[test]
    fn csv_escapes_special_chars() {
        let dir = work_dir("po_csv_esc");
        let mut lib = Library::open_memory().unwrap();
        let mut u = PhotoUpdate::new(r#"D:\a,b\say "hi".jpg"#);
        u.text("name", Some(r#"say "hi".jpg"#));
        lib.upsert_photo(&u).unwrap();
        let p = dir.join("x.csv");
        export_csv(&lib, &p).unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        // 含逗号与引号的字段必须被引号包裹，内部引号翻倍
        assert!(text.contains("\"D:\\a,b"), "{text}");
        assert!(text.contains("\"\"hi\"\""), "{text}");
    }

    #[test]
    fn html_contains_stats_and_items() {
        let dir = work_dir("po_html");
        let mut lib = Library::open_memory().unwrap();
        seed(&mut lib);
        let p = dir.join("report.html");
        export_html(&lib, &p, "老照片整理报告", 600).unwrap();

        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.contains("照片总数"));
        assert!(text.contains("老照片整理报告"));
        assert!(text.contains("<img"));
        assert!(text.contains("全家福"));
        assert!(text.contains("1985-06-12"));
    }

    #[test]
    fn html_escapes_dangerous_content() {
        let dir = work_dir("po_html_esc");
        let mut lib = Library::open_memory().unwrap();
        let mut u = PhotoUpdate::new(r"D:\<script>alert(1)</script>.jpg");
        u.text("name", Some("<script>alert(1)</script>.jpg"));
        let pid = lib.upsert_photo(&u).unwrap();
        lib.add_tags_bulk(&[(pid, "<b>xss</b>".into())], "auto", "auto");

        let p = dir.join("r.html");
        // 标题里的恶意内容也必须转义
        export_html(&lib, &p, "<evil>", 10).unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(!text.contains("<script>alert(1)</script>"), "不得注入脚本");
        assert!(!text.contains("<b>xss</b>"), "标签也应转义");
        assert!(text.contains("&lt;b&gt;xss&lt;/b&gt;"), "标签应转义成实体");
        assert!(text.contains("&lt;evil&gt;"), "标题应转义");
    }

    #[test]
    fn html_escapes_raw_path_with_angle_brackets() {
        // path 列不经过 safe_name，尖括号会原样进 HTML，必须靠 esc 兜住
        let dir = work_dir("po_html_esc2");
        let mut lib = Library::open_memory().unwrap();
        lib.upsert_photo(&mut PhotoUpdate::new("D:\\a<b>c.jpg"))
            .unwrap();
        let p = dir.join("r.html");
        export_html(&lib, &p, "T", 10).unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(!text.contains("<b>c.jpg"), "路径里的尖括号不能原样输出");
    }

    #[test]
    fn empty_library_exports_fine() {
        let dir = work_dir("po_report_empty");
        let lib = Library::open_memory().unwrap();
        let csv = dir.join("e.csv");
        assert_eq!(export_csv(&lib, &csv).unwrap(), 0);
        let html = dir.join("e.html");
        assert_eq!(export_html(&lib, &html, "空库", 100).unwrap(), 0);
        assert!(html.exists());
    }

    #[test]
    fn html_respects_max_items() {
        let dir = work_dir("po_html_max");
        let mut lib = Library::open_memory().unwrap();
        for i in 0..5 {
            lib.upsert_photo(&mut PhotoUpdate::new(format!("p{i}.jpg")))
                .unwrap();
        }
        let p = dir.join("m.html");
        let n = export_html(&lib, &p, "T", 2).unwrap();
        assert_eq!(n, 2);
        let text = std::fs::read_to_string(&p).unwrap();
        assert_eq!(text.matches("<img").count(), 2);
    }

    #[test]
    fn pathdiff_computes_relative() {
        let t = Path::new(r"C:\a\b\c.txt");
        let b = Path::new(r"C:\a\d");
        let d = pathdiff(t, b).unwrap();
        assert_eq!(d.to_string_lossy().replace('\\', "/"), "../b/c.txt");
    }

    #[test]
    fn pathdiff_same_dir() {
        let d = pathdiff(Path::new(r"C:\a\x.txt"), Path::new(r"C:\a")).unwrap();
        assert_eq!(d.to_string_lossy(), "x.txt");
    }

    #[test]
    fn csv_columns_match_expected_order() {
        // 列顺序不能改，否则下游脚本会错位
        assert_eq!(CSV_COLS[0], "id");
        assert_eq!(CSV_COLS[1], "path");
        assert_eq!(CSV_COLS.len(), 30);
        assert!(CSV_COLS.contains(&"p_hash") == false, "Python 版没导出 p_hash");
        // 视频支持后新增的两列
        assert!(CSV_COLS.contains(&"kind"));
        assert!(CSV_COLS.contains(&"duration"));
    }
}
