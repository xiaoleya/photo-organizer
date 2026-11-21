//! 独立 EXIF / GPS / 元数据读取。
//!
//! 覆盖 JPEG(APP1 Exif) 与 TIFF。老照片最关键的三类信息：拍摄时间、相机型号、GPS 坐标。
//! 损坏或不含 EXIF 的文件返回全 `None`，不 panic。

use std::path::Path;

use chrono::NaiveDateTime;

/// EXIF 读取结果。字段与 Python 版 `read_exif()` 对齐。
#[derive(Debug, Clone, Default)]
pub struct ExifInfo {
    pub datetime: Option<NaiveDateTime>,
    pub make: Option<String>,
    pub model: Option<String>,
    pub software: Option<String>,
    pub orientation: u32,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub description: Option<String>,
    pub iso: Option<u32>,
    pub exposure: Option<f64>,
    pub fnumber: Option<f64>,
    pub focal: Option<f64>,
    pub has_exif: bool,
}

impl ExifInfo {
    fn empty() -> Self {
        Self {
            orientation: 1,
            ..Default::default()
        }
    }
}

/// 读取 EXIF。任何失败都返回空结果。
pub fn read_exif(path: &Path) -> ExifInfo {
    let mut out = ExifInfo::empty();
    let Ok(file) = std::fs::File::open(path) else {
        return out;
    };
    let mut bufread = std::io::BufReader::new(file);
    let Ok(exif) = exif::Reader::new().read_from_container(&mut bufread) else {
        return out;
    };
    out.has_exif = true;

    // 拍摄时间：DateTimeOriginal > DateTime > DateTimeDigitized
    let dt_fields = [
        exif::Tag::DateTimeOriginal,
        exif::Tag::DateTime,
        exif::Tag::DateTimeDigitized,
    ];
    for f in dt_fields {
        if out.datetime.is_some() {
            break;
        }
        if let Some(field) = exif.get_field(f, exif::In::PRIMARY) {
            out.datetime = parse_exif_datetime(&field.display_value().to_string());
        }
        if out.datetime.is_none() {
            if let Some(field) = exif.get_field(f, exif::In::PRIMARY) {
                if let exif::Value::Ascii(ref v) = field.value {
                    if let Some(bytes) = v.first() {
                        let s = String::from_utf8_lossy(bytes);
                        out.datetime = parse_exif_datetime(&s);
                    }
                }
            }
        }
    }

    out.make = field_string(&exif, exif::Tag::Make);
    out.model = field_string(&exif, exif::Tag::Model);
    out.software = field_string(&exif, exif::Tag::Software);

    if let Some(field) = exif.get_field(exif::Tag::Orientation, exif::In::PRIMARY) {
        if let Some(v) = field.value.get_uint(0) {
            out.orientation = v;
        }
    }

    out.lat = gps_coord(&exif, exif::Tag::GPSLatitude, exif::Tag::GPSLatitudeRef);
    out.lon = gps_coord(&exif, exif::Tag::GPSLongitude, exif::Tag::GPSLongitudeRef);

    if let Some(field) = exif.get_field(exif::Tag::PixelXDimension, exif::In::PRIMARY) {
        out.width = field.value.get_uint(0);
    }
    if let Some(field) = exif.get_field(exif::Tag::PixelYDimension, exif::In::PRIMARY) {
        out.height = field.value.get_uint(0);
    }

    if let Some(s) = field_string(&exif, exif::Tag::ImageDescription) {
        out.description = Some(s.chars().take(500).collect());
    }

    if let Some(field) = exif.get_field(exif::Tag::PhotographicSensitivity, exif::In::PRIMARY) {
        out.iso = field.value.get_uint(0);
    }
    out.exposure = field_rational(&exif, exif::Tag::ExposureTime);
    out.fnumber = field_rational(&exif, exif::Tag::FNumber);
    out.focal = field_rational(&exif, exif::Tag::FocalLength);

    out
}

fn field_string(exif: &exif::Exif, tag: exif::Tag) -> Option<String> {
    let field = exif.get_field(tag, exif::In::PRIMARY)?;
    let s = match &field.value {
        exif::Value::Ascii(v) => {
            let bytes = v.first()?;
            String::from_utf8_lossy(bytes).to_string()
        }
        _ => field.display_value().to_string(),
    };
    let s = s.trim_matches(|c: char| c == '\0' || c.is_whitespace()).to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

fn field_rational(exif: &exif::Exif, tag: exif::Tag) -> Option<f64> {
    let field = exif.get_field(tag, exif::In::PRIMARY)?;
    match &field.value {
        exif::Value::Rational(v) => {
            let r = v.first()?;
            if r.denom == 0 {
                None
            } else {
                Some(r.num as f64 / r.denom as f64)
            }
        }
        _ => None,
    }
}

/// GPS 度分秒 -> 十进制度，并按 N/S/E/W 处理符号。
fn gps_coord(exif: &exif::Exif, coord: exif::Tag, ref_tag: exif::Tag) -> Option<f64> {
    let field = exif.get_field(coord, exif::In::PRIMARY)?;
    let exif::Value::Rational(v) = &field.value else {
        return None;
    };
    if v.is_empty() {
        return None;
    }
    let d = if v.len() >= 1 {
        v[0].to_f64()
    } else {
        0.0
    };
    let m = if v.len() >= 2 { v[1].to_f64() } else { 0.0 };
    let s = if v.len() >= 3 { v[2].to_f64() } else { 0.0 };
    let mut deg = d + m / 60.0 + s / 3600.0;

    if let Some(rf) = exif.get_field(ref_tag, exif::In::PRIMARY) {
        let r = rf.display_value().to_string().to_uppercase();
        if r.starts_with('S') || r.starts_with('W') {
            deg = -deg;
        }
    }
    Some((deg * 1e6).round() / 1e6)
}

/// 解析 EXIF 时间字符串（`2020:01:02 03:04:05`）。
fn parse_exif_datetime(s: &str) -> Option<NaiveDateTime> {
    let s = s.trim().trim_matches('"').replace('\0', "");
    if s.is_empty() {
        return None;
    }
    let normalized = s.replace('-', ":").replace('/', ":");
    let head: String = normalized.chars().take(19).collect();
    for fmt in [
        "%Y:%m:%d %H:%M:%S",
        "%Y:%m:%d %H:%M",
        "%Y:%m:%d",
        "%Y:%m:%d %H:%M:%S%.f",
    ] {
        if let Ok(dt) = NaiveDateTime::parse_from_str(&head, fmt) {
            return Some(dt);
        }
        if let Ok(d) = chrono::NaiveDate::parse_from_str(&head, fmt) {
            return d.and_hms_opt(0, 0, 0);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn sample(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../samples")
            .join(name)
    }

    #[test]
    fn missing_file_returns_empty() {
        let e = read_exif(Path::new("no_such.jpg"));
        assert!(!e.has_exif);
        assert!(e.datetime.is_none());
        assert_eq!(e.orientation, 1);
    }

    #[test]
    fn broken_file_returns_empty() {
        let p = std::env::temp_dir().join("po_exif_broken.jpg");
        std::fs::write(&p, b"\xff\xd8\xff\xe0 nope").unwrap();
        let e = read_exif(&p);
        assert!(!e.has_exif);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn synthetic_sample_has_no_exif() {
        // make_samples.py 生成的是纯 JFIF，没有 EXIF 块
        let e = read_exif(&sample("DSC_0071.JPG"));
        assert!(!e.has_exif, "合成样图不应含 EXIF");
        assert!(e.datetime.is_none());
        assert!(e.make.is_none());
        assert!(e.lat.is_none());
    }

    #[test]
    fn parses_exif_datetime_formats() {
        let d = parse_exif_datetime("2020:01:02 03:04:05").unwrap();
        assert_eq!(d.format("%Y-%m-%d %H:%M:%S").to_string(), "2020-01-02 03:04:05");

        let d = parse_exif_datetime("2020-01-02 03:04:05").unwrap();
        assert_eq!(d.format("%Y").to_string(), "2020");

        let d = parse_exif_datetime("2020:01:02").unwrap();
        assert_eq!(d.format("%H").to_string(), "00");

        assert!(parse_exif_datetime("").is_none());
        assert!(parse_exif_datetime("not a date").is_none());
    }

    #[test]
    fn handles_quoted_and_null_padded() {
        let d = parse_exif_datetime("\"2019:05:06 07:08:09\"").unwrap();
        assert_eq!(d.format("%Y").to_string(), "2019");
        let d = parse_exif_datetime("2019:05:06 07:08:09\0\0").unwrap();
        assert_eq!(d.format("%m").to_string(), "05");
    }
}
