# 照片整理（by 小乐）

面向大量照片/老照片的本地整理工具。纯本地运行，不上传任何照片；支持按时间、地点浏览，重复检测，质量待修，照片修复与归档输出。

> 默认所有照片数据只保存在本机 `%LOCALAPPDATA%\OldPhotoOrganizer\`，不会联网上传。

> ⚠️ **当前是 v1.1.0 测试版**：功能与界面仍在调整，建议先用一份照片副本试跑。安装包请到 [Releases](../../releases) 下载。
>
> v1.1 起引擎已从 Python 重写为 Rust 并并入主程序：单进程、无需 Python 运行时，安装包 4.1 MB。

---

## 功能一览

### 照片库
- 照片墙展示全部照片，滚动到底自动加载更多
- 顶部按「全部 / 仅重复 / 缺日期 / 模糊 / 褪色 / 泛黄 / 过暗 / 噪点」筛选
- 按时间、质量、年代感排序
- 全局搜索：文件名、路径、地点
- 点击卡片打开大图：支持滚轮缩放、按钮缩放、键盘 `+`/`-`/`0`、拖拽平移、上一张/下一张、收藏、去修复
- 双击卡片用系统默认程序打开原图

### 时间轴
- 按年份分组，默认按月展开
- 可切换月份排序：`12 → 1`（最近在前）或 `1 → 12`

### 地点
- GPS 坐标反查城市（内置离线城市表，含县级点位）
- 文件夹名/文件名里的城市名也能识别
- 无地点信息统一归入「未知地点」
- 按「省份/地区 → 地点 → 年份」分层展示
- 页面内附说明：哪些照片才有地点（微信传输会清除 GPS、截图无定位等）

### 收藏
- 卡片左下角星标收藏
- 单独的收藏页，支持搜索和取消收藏

### 重复照片
- 感知哈希检测完全相同的副本与高度相似的照片
- 每组逐张勾选要删除的副本，可一键「选本组副本」「选择全部副本」
- 删除时移到回收目录（不直接销毁），可打开回收目录

### 质量待修（实验室功能，默认关闭）
- 判定：模糊、过暗、过亮、褪色、泛黄、噪点重、低分辨率
- 截图/文档走专门的宽松判定标准，避免把正常截图全标成问题照片
- 可展开「识别标准」，用大白话调整各项阈值
- 「按新标准重新判定」可在不重新扫描的情况下刷新已有照片的标记
- 仅此页保留「去修复」入口；未开启照片修复功能时会提示去实验室开启

### 照片修复（实验室功能，默认关闭）
- 预设：轻度修复、标准修复、重度修复、仅去噪、仅调色、仅裁边纠偏、去泛黄、去划痕、增强清晰度
- 可自由勾选：自动裁边、纠偏、白平衡、去泛黄、自动色阶、降噪、去划痕、局部对比、褪色补色、锐化、放大 2 倍
- 截图/文档自动识别并走「保清晰」专用流程，不裁边、不纠偏、不糊字
- 修复前后并排对比，两侧均可点开放大；右上角显示量化提升
- 修完提示另存到指定目录；支持多选批量修复

### 整理归档
- 层级自由组合：年 / 月 / 地点
- 收藏照片处理方式：收藏夹+分类 / 仅收藏夹 / 仅分类
- 复制、移动两种模式；保持原名或加日期前缀
- 无时间/地点的照片放入占位文件夹，不会丢失

### 设置
- 照片目录：添加、逐个删除、在资源管理器打开
- 开始扫描 / 清除扫描记录（只清索引，不动原文件）
- 外观主题：深色 / 浅色，强调色蓝 / 青 / 紫 / 橙 / 绿
- 实验室功能：只控制「质量待修」「照片修复」两项，其余功能常驻

---

## 下载与运行

安装包和绿色版在 [Releases](../../releases) 里，仓库本身只放源码。
附件名是英文的（GitHub 会清洗中文附件名），装完仍是「照片整理」。

### Windows 安装包
运行 `PhotoOrganizer-Setup-v1.1.0-beta.exe`，一路下一步即可。

### 绿色版
解压 `PhotoOrganizer-Portable-v1.1.0-beta.zip`，双击 `照片整理.exe` 即可。

> v1.1 起引擎已并入主程序，**不再需要 `backend/photo-backend.exe`**，
> 也不再依赖 Python 运行时，解压后只有一个 exe。

第一次启动约 1~2 秒。打开后：
1. 点顶部「选择文件夹」
2. 点「开始扫描」
3. 扫描完成后点「确定，去看照片」

---

## 从源码运行

### 环境
- Rust 1.80+
- Node.js 20+
- （可选）MSVC Build Tools

### 桌面应用
```bash
cd ui
npm install
npm run tauri dev             # 开发模式
npm run tauri build           # 打包安装包
```

### 引擎 CLI
引擎（`crates/po-core`）可脱离界面单独使用：

```bash
cd 源码
cargo build --release

# 扫描 → 分析 → 打标 → 地点 → 事件 → 去重
./target/release/po run D:\照片

# 其他命令
./target/release/po stats              # 统计
./target/release/po list --limit 50    # 列出照片
./target/release/po organize --dest out --scheme year_month
./target/release/po rollback           # 撤销上次整理
./target/release/po restore --out fixed --preset 标准修复
./target/release/po report --html r.html --csv p.csv
./target/release/po check              # 校验资源库完整性
```

### 测试
```bash
cd 源码
cargo test --workspace
cd ui/src-tauri && cargo test           # 命令层
```

## 项目结构

```
.
├── crates/
│   ├── po-core/             引擎（lib，无 UI 依赖）
│   │   ├── config.rs        路径、默认设置、settings.json
│   │   ├── util.rs          日期解析、安全文件名、sha1
│   │   ├── exif.rs          EXIF / GPS 解析
│   │   ├── imaging.rs       质量指标、缩略图、感知哈希
│   │   ├── library.rs       SQLite 数据层
│   │   ├── scanner.rs       目录遍历与并行分析
│   │   ├── classify.rs      打标、事件聚类、去重
│   │   ├── geo.rs           离线城市表与地点推断
│   │   ├── restore.rs       修复算法与预设
│   │   ├── organizer.rs     归档计划、执行与回滚
│   │   ├── report.rs        HTML / CSV 导出
│   │   └── pipeline.rs      全流程编排与进度
│   └── po-cli/              命令行工具（`po`）
├── ui/                      Vue 3 + Tauri 前端
│   ├── src/
│   │   ├── views/           照片库、时间轴、地点、收藏、重复、质量、修复、归档、设置
│   │   ├── components/      PhotosPanel、TitleBar、Icon 等
│   │   ├── api.js           Tauri invoke 封装
│   │   └── thumb.js         图片 URL（自定义 `po://` 协议）
│   └── src-tauri/
│       ├── src/main.rs      窗口、`po://` 协议、命令注册
│       └── src/commands.rs  命令层（原 HTTP 接口）
├── samples/                 测试样图
├── tools/make_samples.py    样图生成器（需要 Pillow + numpy）
└── Cargo.toml               workspace
```

---

## 架构演进（v1.1）

v1.0 是「Tauri + Vue 前端 + Python HTTP 后端（`photo-backend.exe` sidecar）」。
v1.1 把引擎整体重写为 Rust 并并入主程序：

| | v1.0 | v1.1 |
| --- | --- | --- |
| 引擎 | Python（Pillow + numpy） | Rust |
| 进程 | 2 个（主程序 + 后端 sidecar） | 1 个 |
| 数据通道 | 本机 HTTP（端口 18921，26 个 REST 接口） | 进程内 `tauri::command` |
| 图片 | `/thumb` `/photo` HTTP | `po://local/thumb` 自定义协议 |
| 运行时依赖 | Python 3.12 + WebView2 | 仅 WebView2 |
| 启动耗时 | 3~5 秒（冷启） | 1~2 秒 |

**兼容性**：SQLite schema、`settings.json` 键名、缩略图缓存键（`thumb_key`）
都与 v1.0 完全一致，升级后可直接复用原有索引库，无需重新扫描。

---

## 隐私说明

- 所有照片分析、缩略图生成、修复计算都在本机完成
- 地点反查使用内置离线城市表，不请求任何地图 API
- 索引数据（SQLite）与缩略图缓存保存在 `%LOCALAPPDATA%\OldPhotoOrganizer\`
- 「清除扫描记录」只删索引和缓存，不删除你的原始照片文件
- 「重复照片」删除时会先移到回收目录，不直接销毁

---

## 已知限制

- 没有 GPS、目录名也无地名的照片（微信转发的图、截图等）无法自动识别地点，统一显示「未知地点」
- 质量判定是统计特征 heuristic，无法做到 100% 准确；阈值可在「质量待修 → 识别标准」里调整
- 照片修复针对老照片调校，对现代数码照片效果有限；截图/文档会自动跳过裁边与纠偏

---

MIT License
