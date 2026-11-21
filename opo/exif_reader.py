# -*- coding: utf-8 -*-
"""独立 EXIF / GPS / 元数据读取（不依赖第三方库）。

覆盖 JPEG(APP1 Exif)、TIFF、以及 PNG 的 eXIf 块。老照片最关键的三类信息：
拍摄时间、相机型号、GPS 坐标。
"""
from __future__ import annotations

import datetime as _dt
import os
import struct

_TAG_NAMES = {
    0x010F: "Make", 0x0110: "Model", 0x0112: "Orientation",
    0x0132: "DateTime", 0x0131: "Software", 0x010E: "ImageDescription",
    0x9286: "UserComment", 0x9003: "DateTimeOriginal", 0x9004: "DateTimeDigitized",
    0x829A: "ExposureTime", 0x829D: "FNumber", 0x8827: "ISO",
    0x920A: "FocalLength", 0xA002: "PixelXDimension", 0xA003: "PixelYDimension",
    0x011A: "XResolution", 0x011B: "YResolution", 0x0128: "ResolutionUnit",
    0xA430: "CameraOwnerName", 0xA431: "BodySerialNumber",
    0x9C9B: "XPTitle", 0x9C9C: "XPComment", 0x9C9D: "XPAuthor", 0x9C9E: "XPKeywords",
    0x9C9F: "XPSubject",
}

_GPS_TAGS = {
    0: "GPSVersionID", 1: "GPSLatitudeRef", 2: "GPSLatitude", 3: "GPSLongitudeRef",
    4: "GPSLongitude", 5: "GPSAltitudeRef", 6: "GPSAltitude",
    7: "GPSTimeStamp", 8: "GPSSatellites", 29: "GPSDateStamp",
}


def _type_size(t):
    return {1: 1, 2: 1, 3: 2, 4: 4, 5: 8, 6: 1, 7: 1, 8: 2, 9: 4, 10: 8, 11: 4, 12: 8}.get(t, 1)


def _read_ifd(buf, offset, endian, tag_names, out, depth=0):
    if depth > 3 or offset <= 0 or offset + 2 > len(buf):
        return
    try:
        (count,) = struct.unpack_from(endian + "H", buf, offset)
    except Exception:
        return
    base = offset + 2
    for i in range(min(count, 512)):
        entry = base + i * 12
        if entry + 12 > len(buf):
            break
        try:
            tag, typ, cnt = struct.unpack_from(endian + "HHI", buf, entry)
        except Exception:
            break
        size = _type_size(typ) * cnt
        if size <= 0 or cnt > 1_000_000:
            continue
        if size > 4:
            (ptr,) = struct.unpack_from(endian + "I", buf, entry + 8)
            data_at = ptr
        else:
            data_at = entry + 8
        if data_at < 0 or data_at + size > len(buf):
            continue
        blob = buf[data_at:data_at + size]

        if typ in (2,):
            value = blob.split(b"\x00")[0].decode("utf-8", "replace").strip()
        elif typ in (1, 6, 7):
            value = blob
        elif typ == 3:
            vals = struct.unpack(endian + "H" * cnt, blob)
            value = vals[0] if cnt == 1 else vals
        elif typ in (4, 13):
            vals = struct.unpack(endian + "I" * cnt, blob)
            value = vals[0] if cnt == 1 else vals
        elif typ == 9:
            vals = struct.unpack(endian + "i" * cnt, blob)
            value = vals[0] if cnt == 1 else vals
        elif typ == 5:
            vals = struct.unpack(endian + "I" * (cnt * 2), blob)
            value = [(vals[j * 2] / vals[j * 2 + 1]) if vals[j * 2 + 1] else 0.0 for j in range(cnt)]
            value = value[0] if cnt == 1 else value
        elif typ == 10:
            vals = struct.unpack(endian + "i" * (cnt * 2), blob)
            value = [(vals[j * 2] / vals[j * 2 + 1]) if vals[j * 2 + 1] else 0.0 for j in range(cnt)]
            value = value[0] if cnt == 1 else value
        else:
            continue

        name = tag_names.get(tag)
        if name:
            out[name] = value

        if tag == 0x8769 and typ in (4, 13):
            ptr = value if isinstance(value, int) else None
            if ptr:
                out["_exif_ifd"] = ptr
        elif tag == 0x8825 and typ in (4, 13):
            ptr = value if isinstance(value, int) else None
            if ptr:
                _read_ifd(buf, ptr, endian, _GPS_TAGS, out, depth + 1)


def _tiff_parse(buf, out):
    if len(buf) < 8:
        return
    if buf[:2] == b"II":
        endian = "<"
    elif buf[:2] == b"MM":
        endian = ">"
    else:
        return
    (magic,) = struct.unpack_from(endian + "H", buf, 2)
    if magic != 42:
        return
    (first,) = struct.unpack_from(endian + "I", buf, 4)
    _read_ifd(buf, first, endian, _TAG_NAMES, out)
    exif_ptr = out.pop("_exif_ifd", None)
    if exif_ptr:
        _read_ifd(buf, exif_ptr, endian, _TAG_NAMES, out)


def _gps_to_deg(v, ref):
    try:
        if isinstance(v, (list, tuple)) and len(v) >= 3:
            d, m, s = float(v[0]), float(v[1]), float(v[2])
        elif isinstance(v, (int, float)):
            d, m, s = float(v), 0.0, 0.0
        else:
            return None
        deg = d + m / 60.0 + s / 3600.0
        if isinstance(ref, bytes):
            ref = ref.decode("ascii", "replace")
        if str(ref).strip().upper() in ("S", "W"):
            deg = -deg
        return round(deg, 6)
    except Exception:
        return None


def _parse_dt(s):
    if isinstance(s, bytes):
        s = s.decode("ascii", "replace")
    s = str(s or "").strip().replace("\x00", "")
    if not s:
        return None
    s = s.replace("-", ":").replace("/", ":")
    for fmt in ("%Y:%m:%d %H:%M:%S", "%Y:%m:%d %H:%M", "%Y:%m:%d", "%Y:%m:%d %H:%M:%S.%f"):
        try:
            return _dt.datetime.strptime(s[:19], fmt)
        except Exception:
            continue
    return None


def read_exif(path: str) -> dict:
    """返回 {datetime, make, model, software, orientation, lat, lon, width, height, description}"""
    res = {
        "datetime": None, "make": None, "model": None, "software": None,
        "orientation": 1, "lat": None, "lon": None, "width": None, "height": None,
        "description": None, "iso": None, "exposure": None, "fnumber": None,
        "focal": None, "has_exif": False,
    }
    try:
        with open(path, "rb") as f:
            head = f.read(32)
            f.seek(0)
            raw = None
            ext = os.path.splitext(path)[1].lower()
            if head[:2] == b"\xff\xd8":
                # JPEG: 扫描 APP1
                f.seek(2)
                while True:
                    b = f.read(2)
                    if len(b) < 2 or b[0] != 0xFF:
                        break
                    marker = b[1]
                    if marker in (0xD8, 0xD9):
                        continue
                    lb = f.read(2)
                    if len(lb) < 2:
                        break
                    seg_len = struct.unpack(">H", lb)[0] - 2
                    if seg_len < 0:
                        break
                    if marker == 0xE1:
                        seg = f.read(seg_len)
                        if seg[:6] == b"Exif\x00\x00":
                            raw = seg[6:]
                            break
                    else:
                        if marker == 0xDA:
                            break
                        f.seek(seg_len, 1)
            elif head[:4] in (b"II*\x00", b"MM\x00*"):
                raw = f.read(4 * 1024 * 1024)
            elif head[:8] == b"\x89PNG\r\n\x1a\n":
                raw = None  # PNG eXIf 少见，跳过
            if not raw:
                return res
            tags = {}
            _tiff_parse(raw, tags)
            res["has_exif"] = bool(tags)
            dt = _parse_dt(tags.get("DateTimeOriginal")) or _parse_dt(tags.get("DateTime")) \
                 or _parse_dt(tags.get("DateTimeDigitized"))
            res["datetime"] = dt
            for k, src in (("make", "Make"), ("model", "Model"), ("software", "Software")):
                v = tags.get(src)
                if isinstance(v, bytes):
                    v = v.decode("utf-8", "replace")
                if v:
                    res[k] = str(v).strip().strip("\x00") or None
            ori = tags.get("Orientation")
            if isinstance(ori, int):
                res["orientation"] = ori
            lat = _gps_to_deg(tags.get("GPSLatitude"), tags.get("GPSLatitudeRef"))
            lon = _gps_to_deg(tags.get("GPSLongitude"), tags.get("GPSLongitudeRef"))
            res["lat"], res["lon"] = lat, lon
            desc = tags.get("ImageDescription")
            if isinstance(desc, bytes):
                desc = desc.decode("utf-8", "replace")
            res["description"] = str(desc).strip("\x00 ")[:500] if desc else None
            for k, src in (("iso", "ISO"), ("exposure", "ExposureTime"),
                           ("fnumber", "FNumber"), ("focal", "FocalLength")):
                if tags.get(src) is not None:
                    res[k] = tags.get(src)
    except Exception:
        pass
    return res