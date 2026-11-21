# -*- coding: utf-8 -*-
"""生成用于自测的合成老照片样本（无需外部素材）。"""
import os, math, random, sys
import numpy as np
from PIL import Image, ImageDraw, ImageFilter

OUT = sys.argv[1] if len(sys.argv) > 1 else "samples"
random.seed(7); np.random.seed(7)


def base(seed, w=1200, h=900, kind="portrait"):
    r = random.Random(seed)
    sky = np.zeros((h, w, 3), np.float32)
    top = np.array([r.randint(90,140), r.randint(120,170), r.randint(160,220)], np.float32)
    bot = np.array([r.randint(120,180), r.randint(130,190), r.randint(120,170)], np.float32)
    for y in range(h):
        t = y / max(1, h-1)
        sky[y, :, :] = top*(1-t) + bot*t
    img = Image.fromarray(np.clip(sky,0,255).astype(np.uint8))
    d = ImageDraw.Draw(img)
    # 地面
    gy = int(h*0.62)
    d.rectangle([0, gy, w, h], fill=tuple(int(x) for x in (110+r.randint(-20,20), 95+r.randint(-20,20), 70+r.randint(-20,20))))
    if kind == "portrait":
        for i in range(r.randint(1,3)):
            cx = int(w*(0.3+0.2*i)); cy = int(h*0.42)
            hh = int(h*0.32); ww = int(hh*0.72)
            sk = (200+r.randint(-25,25), 165+r.randint(-25,25), 140+r.randint(-25,25))
            d.ellipse([cx-ww//2, cy-hh//2, cx+ww//2, cy+hh//2], fill=sk)
            d.ellipse([cx-ww//3, cy-hh//2+ww//4, cx-ww//3+ww//6, cy-hh//2+ww//4+ww//6], fill=(40,35,30))
            d.ellipse([cx+ww//3-ww//6, cy-hh//2+ww//4, cx+ww//3, cy-hh//2+ww//4+ww//6], fill=(40,35,30))
            d.rectangle([cx-ww//2, cy+hh//2-8, cx+ww//2, cy+hh], fill=(60,60,70))
    elif kind == "landscape":
        for i in range(r.randint(2,4)):
            bx = int(w*(0.15+0.22*i))
            d.polygon([(bx, gy), (bx+int(w*0.09), gy-int(h*0.30)), (bx+int(w*0.18), gy)],
                      fill=(70+r.randint(0,40), 90+r.randint(0,40), 70+r.randint(0,30)))
    elif kind == "document":
        d.rectangle([int(w*0.1), int(h*0.08), int(w*0.9), int(h*0.92)], fill=(245,240,228))
        for i in range(18):
            y = int(h*0.14) + i*int(h*0.042)
            d.line([(int(w*0.15), y), (int(w*(0.15+random.random()*0.6)), y)], fill=(60,55,50), width=3)
    elif kind == "group":
        for i in range(6):
            cx = int(w*(0.14+0.145*i)); cy = int(h*0.50)
            hh = int(h*0.26); ww = int(hh*0.66)
            sk = (195+r.randint(-20,20), 160+r.randint(-20,20), 135+r.randint(-20,20))
            d.ellipse([cx-ww//2, cy-hh//2, cx+ww//2, cy+hh//2], fill=sk)
            d.rectangle([cx-ww//2, cy+hh//2-6, cx+ww//2, cy+hh], fill=(70,70,80))
    return img


def age(img, grain=14.0, sepia=1.0, fade=0.55, blur=0.0, scratches=0, vignette=True):
    a = np.asarray(img, np.float32).copy()
    h, w = a.shape[:2]
    # 褪色：向灰靠近
    g = a.mean(axis=2, keepdims=True)
    a = a*(1-fade*0.45) + g*fade*0.45
    # 泛黄
    a[:,:,0] *= (1.0 + 0.22*sepia)
    a[:,:,1] *= (1.0 + 0.06*sepia)
    a[:,:,2] *= (1.0 - 0.28*sepia)
    # 渐晕
    if vignette:
        yy, xx = np.mgrid[0:h, 0:w]
        cx, cy = w/2, h/2
        r = np.sqrt(((xx-cx)/(w*0.62))**2 + ((yy-cy)/(h*0.62))**2)
        v = np.clip(1.0 - 0.42*np.clip(r-0.55, 0, 3)**1.5, 0.35, 1.0)
        a *= v[:,:,None]
    # 颗粒
    a += np.random.normal(0, grain, a.shape).astype(np.float32)
    a = np.clip(a, 0, 255).astype(np.uint8)
    img = Image.fromarray(a)
    if blur > 0:
        img = img.filter(ImageFilter.GaussianBlur(blur))
    if scratches:
        d = ImageDraw.Draw(img)
        for _ in range(scratches):
            x = random.randint(0, w-1); y0 = random.randint(0, int(h*0.3))
            y1 = y0 + random.randint(int(h*0.2), int(h*0.7))
            c = 235 if random.random() > 0.5 else 35
            d.line([(x, y0), (x+random.randint(-6,6), y1)], fill=(c,c,c), width=random.randint(1,2))
        img = Image.fromarray(np.clip(np.asarray(img, np.int16), 0, 255).astype(np.uint8))
    return img


CFG = [
    ("1985-06-12 春节全家福.jpg", "group",    dict(grain=16, sepia=1.0, fade=0.7, scratches=14)),
    ("1985年06月12日 老家院子合影.jpg", "portrait", dict(grain=15, sepia=0.9, fade=0.65, scratches=10)),
    ("1992-10-01 旅行.jpg",   "landscape", dict(grain=12, sepia=0.7, fade=0.5, scratches=8)),
    ("IMG_20050315_毕业典礼.jpg", "group", dict(grain=8, sepia=0.35, fade=0.25, scratches=3)),
    ("20050315_141530.jpg",   "portrait", dict(grain=9, sepia=0.3, fade=0.2, scratches=2)),
    ("扫描件_2008-07-20_证件照.jpg", "document", dict(grain=6, sepia=0.2, fade=0.15, scratches=2)),
    ("微信图片_20200101.jpg",   "landscape", dict(grain=5, sepia=0.05, fade=0.05, scratches=0)),
    ("DSC_0071.JPG",          "portrait", dict(grain=7, sepia=0.1, fade=0.1, scratches=1)),
    ("模糊的老照片 1978.jpg", "portrait", dict(grain=20, sepia=1.0, fade=0.8, blur=2.2, scratches=20)),
]

os.makedirs(OUT, exist_ok=True)
made = []
for i, (name, kind, kw) in enumerate(CFG):
    img = base(i+1, 1280, 960, kind)
    img = age(img, **kw)
    ext = os.path.splitext(name)[1].lower()
    if ext == ".jpg":
        p = os.path.join(OUT, name)
        img.save(p, "JPEG", quality=88)
        made.append(p)
    else:
        p = os.path.join(OUT, name + ".jpg")
        img.save(p, "JPEG", quality=88)
        made.append(p)

# 重复样本：同一张的缩放 + 重压缩 + 轻微调亮
src = Image.open(made[0])
dupdir = os.path.join(OUT, "重复")
os.makedirs(dupdir, exist_ok=True)
src.resize((640, 480), Image.LANCZOS).save(os.path.join(dupdir, "1985-06-12 春节全家福_副本.jpg"), "JPEG", quality=72)
a = np.asarray(src, np.int16); a = np.clip(a+8, 0, 255).astype(np.uint8)
Image.fromarray(a).save(os.path.join(dupdir, "copy of 1985-06-12 春节全家福.jpg"), "JPEG", quality=80)
src.save(os.path.join(dupdir, "完全相同1.jpg"), "JPEG", quality=88)

# 子目录：按年份归档的原始目录结构
sub = os.path.join(OUT, "2004年")
os.makedirs(sub, exist_ok=True)
base(99, 900, 1200, "portrait")
age(base(99, 900, 1200, "portrait"), grain=10, sepia=0.5, fade=0.35, scratches=4).save(
    os.path.join(sub, "照片 001.jpg"), "JPEG", quality=85)

# 一张纯色损坏文件测健壮性
with open(os.path.join(OUT, "broken.jpg"), "wb") as f:
    f.write(b"\xff\xd8\xff\xe0 not really a jpeg")
print("生成样本:", len(os.listdir(OUT)), "项 ->", os.path.abspath(OUT))
