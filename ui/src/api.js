/* 后端 API + Tauri 原生能力封装
 *
 * 引擎已从「Python HTTP sidecar」改为进程内 Rust（po-core），
 * 所以这里不再 fetch，而是走 tauri::invoke。
 * 对外暴露的函数签名与返回值结构保持不变，视图组件无需改动。
 */

/* 是否运行在 Tauri 里：必须运行时判断，
   模块加载时 window.__TAURI__ 可能还没注入 */
export function inTauri() {
  return typeof window !== 'undefined' &&
         (!!window.__TAURI__ || !!window.__TAURI_INTERNALS__)
}

/* ---- Tauri invoke：优先全局对象，其次 internals ---- */
async function invoke(cmd, args = {}) {
  const g = typeof window !== 'undefined' ? window.__TAURI__ : null
  const internals = typeof window !== 'undefined' ? window.__TAURI_INTERNALS__ : null
  try {
    if (g && g.core && typeof g.core.invoke === 'function') {
      return await g.core.invoke(cmd, args)
    }
    if (internals && typeof internals.invoke === 'function') {
      return await internals.invoke(cmd, args)
    }
    if (g && typeof g.invoke === 'function') {
      return await g.invoke(cmd, args)
    }
    console.warn('Tauri API 不可用，跳过', cmd)
    return null
  } catch (e) {
    console.error('invoke failed:', cmd, e)
    return null
  }
}

/* ---- 查询 ---- */
export const api = {
  ping: () => invoke('ping'),
  stats: () => invoke('get_stats'),
  photos: (q = {}) => invoke('get_photos', { query: normalizeQuery(q) }),
  photo: (id) => invoke('get_photo', { id: Number(id) }),
  timeline: () => invoke('get_timeline'),
  places: () => invoke('get_places'),
  duplicates: () => invoke('get_duplicates'),
  roots: () => invoke('get_roots'),
  settings: () => invoke('get_settings'),
  presets: () => invoke('get_restore_presets'),
  progress: () => invoke('get_scan_progress'),
  editPhoto: (id, fields = {}) =>
    invoke('edit_photo', { req: { id: Number(id), ...fields } }),
  setPhotoTags: (id, tags) =>
    invoke('set_photo_tags', { id: Number(id), tags: (tags || []).map(String) }),

  /* ---- 写操作 ---- */
  saveSettings: (settings) => invoke('save_settings', { settings }),
  addRoot: (path) => invoke('add_root', { path }),
  removeRoot: (path) => invoke('remove_root', { path }),
  scanStart: (roots, reanalyzeAll = false) =>
    invoke('scan_start', { req: { roots: roots || null, reanalyze_all: reanalyzeAll } }),
  scanCancel: () => invoke('scan_cancel'),
  favorite: (id, favorite) => invoke('set_favorite', { req: { id: Number(id), favorite } }),
  /* 批量收藏：一次调用搞定，不用前端循环 */
  favoriteMany: (ids, favorite = true) =>
    invoke('set_favorite', { req: { ids: (ids || []).map(Number), favorite } }),
  /* 批量导出到指定目录 */
  exportPhotos: (ids, dest, mode = 'copy') =>
    invoke('export_photos', { req: { ids: (ids || []).map(Number), dest, mode } }),
  reclassify: () => invoke('reclassify'),
  backfillPlaces: (redo = true) => invoke('backfill_places', { redo }),
  resetIndex: (keepRoots = true) => invoke('reset_index', { keepRoots }),

  /* ---- 重复清理 ---- */
  setDupKeeper: (groupId, photoId) =>
    invoke('set_dup_keeper', { groupId: Number(groupId), photoId: Number(photoId) }),
  deletePhotos: (ids, opts) =>
    invoke('delete_photos', { req: { ids: (ids || []).map(Number), ...(opts || {}) } }),

  /* ---- 修复 ---- */
  restoreApply: (id, preset, options) =>
    invoke('restore_apply', { req: { id: Number(id), preset, options } }),
  restoreBatch: (ids, dest, preset, options) =>
    invoke('restore_batch', {
      req: { ids: (ids || []).map(Number), dest, preset, options },
    }),

  /* ---- 整理 ---- */
  organize: (opts) => invoke('organize', { req: opts || {} }),

  /* ---- 回收站 ---- */
  recycle: () => invoke('get_recycle'),
  // 注意：这里的字段名必须与 Rust 侧 RecycleAction 完全一致（snake_case），
  // Tauri 不会自动把 logIds 映射成 log_ids —— 写成 camelCase 会导致取不到 id。
  restoreRecycle: (logIds, all = false) =>
    invoke('restore_recycle', { req: { log_ids: (logIds || []).map(Number), all } }),
  purgeRecycle: (logIds, all = false) =>
    invoke('purge_recycle', { req: { log_ids: (logIds || []).map(Number), all } }),
}

/* 把 URL 查询风格的键值（全是字符串）转成命令层期望的类型。
   PhotoQuery 里的数字字段是 Option<i64>/Option<f64>，
   传字符串会被 serde 拒绝，所以这里显式转一次。 */
const NUMERIC_KEYS = {
  dup_group: 'i64',
  year: 'i64',
  month: 'i64',
  limit: 'i64',
  offset: 'i64',
  min_quality: 'f64',
  max_quality: 'f64',
  event_id: 'i64',
  year_from: 'i64',
  year_to: 'i64',
  album_id: 'i64',
}

function normalizeQuery(q) {
  const out = {}
  for (const [k, v] of Object.entries(q)) {
    if (v === undefined || v === null || v === '') continue
    const kind = NUMERIC_KEYS[k]
    if (kind === 'i64') {
      const n = parseInt(v, 10)
      if (!Number.isNaN(n)) out[k] = n
    } else if (kind === 'f64') {
      const n = parseFloat(v)
      if (!Number.isNaN(n)) out[k] = n
    } else {
      out[k] = String(v)
    }
  }
  return out
}

/* ---- 原生能力：选文件夹 / 打开路径 ---- */
export const native = {
  async pickFolder() {
    const r = await invoke('pick_folder')
    if (r) return r
    if (!inTauri()) {
      const manual = window.prompt('请输入文件夹完整路径：')
      return manual && manual.trim() ? manual.trim() : null
    }
    return null
  },
  async openPath(p) {
    if (!p) return null
    const r = await invoke('open_path', { path: p })
    if (!inTauri()) {
      window.alert('请手动打开：' + p)
    }
    return r
  },
  /* 在资源管理器里定位并选中文件 */
  async reveal(p) {
    if (!p) return null
    return await invoke('reveal_in_explorer', { path: p })
  },
}

/* ---- 等待后端就绪 ----
   引擎在同一进程内，理论上立刻就绪；
   但保留轮询是为了兼容「数据库首次打开较慢」的情况。 */
export async function waitForBackend(timeoutMs = 25000, onTick) {
  const start = Date.now()
  let attempts = 0
  while (Date.now() - start < timeoutMs) {
    attempts++
    const r = await api.ping()
    if (r && r.ok) {
      if (onTick) onTick({ ready: true, attempts })
      return true
    }
    if (onTick) onTick({ ready: false, attempts, elapsed: Date.now() - start })
    await new Promise(res => setTimeout(res, 250))
  }
  return false
}
