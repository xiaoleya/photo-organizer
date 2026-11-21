/* 后端 API + Tauri 原生能力封装 */

const BASE = 'http://127.0.0.1:18921'

/* 是否运行在 Tauri 里：必须运行时判断，
   模块加载时 window.__TAURI__ 可能还没注入 */
export function inTauri() {
  return typeof window !== 'undefined' &&
         (!!window.__TAURI__ || !!window.__TAURI_INTERNALS__)
}

async function req(path, opts = {}) {
  try {
    const r = await fetch(BASE + path, {
      headers: { 'Content-Type': 'application/json', 'Cache-Control': 'no-cache' },
      cache: 'no-store',
      ...opts,
    })
    if (!r.ok) throw new Error('HTTP ' + r.status)
    return await r.json()
  } catch (e) {
    return null
  }
}

/* ---- Tauri 原生命令 ---- */
async function tauriInvoke(cmd, args = {}) {
  const g = window.__TAURI__
  const internals = window.__TAURI_INTERNALS__
  try {
    if (g && g.core && typeof g.core.invoke === 'function') {
      return await g.core.invoke(cmd, args)
    }
    if (internals && typeof internals.invoke === 'function') {
      return await internals.invoke(cmd, args)
    }
    console.warn('Tauri API 不可用，跳过', cmd)
    return null
  } catch (e) {
    console.error('tauri invoke failed:', cmd, e)
    return null
  }
}

export const api = {
  stats: () => req('/api/stats'),
  photos: (q = {}) => req('/api/photos?' + new URLSearchParams(q)),
  photo: (id) => req('/api/photo?id=' + encodeURIComponent(id)),
  tags: () => req('/api/tags'),
  timeline: () => req('/api/timeline'),
  places: () => req('/api/places'),
  events: () => req('/api/events'),
  duplicates: () => req('/api/duplicates'),
  roots: () => req('/api/roots'),
  ping: () => req('/api/ping'),
  progress: () => req('/api/scan/progress'),
  backfillPlaces: () => req('/api/places/backfill', {
    method: 'POST', body: '{}',
  }),
  presets: () => req('/api/restore/presets'),
  settings: () => req('/api/settings'),
  reclassify: () => req('/api/reclassify', { method: 'POST', body: '{}' }),
  favorite: (id, favorite) => req('/api/favorite', {
    method: 'POST', body: JSON.stringify({ id, favorite }),
  }),
  resetIndex: (keepRoots = true) => req('/api/index/reset', {
    method: 'POST', body: JSON.stringify({ keep_roots: keepRoots }),
  }),
  saveSettings: (settings) => req('/api/settings', {
    method: 'POST', body: JSON.stringify({ settings }),
  }),
  scan: (roots) => req('/api/scan', {
    method: 'POST', body: JSON.stringify({ roots }),
  }),
  scanStart: (roots) => req('/api/scan/start', {
    method: 'POST', body: JSON.stringify({ roots }),
  }),
  cleanDuplicates: (opts) => req('/api/dups/clean', {
    method: 'POST', body: JSON.stringify(opts || {}),
  }),
  deletePhotos: (ids, opts) => req('/api/photos/delete', {
    method: 'POST', body: JSON.stringify({ ids, ...(opts || {}) }),
  }),
  restore: (id, preset, options) => req('/api/restore', {
    method: 'POST', body: JSON.stringify({ id, preset, options }),
  }),
  restoreApply: (id, preset, options) => req('/api/restore/apply', {
    method: 'POST', body: JSON.stringify({ id, preset, options }),
  }),
  restoreBatch: (ids, dest, preset, options) => req('/api/restore/batch', {
    method: 'POST', body: JSON.stringify({ ids, dest, preset, options }),
  }),
  organize: (opts) => req('/api/organize', {
    method: 'POST', body: JSON.stringify(opts),
  }),
  addRoot: (path) => req('/api/roots', {
    method: 'POST', body: JSON.stringify({ path }),
  }),
  removeRoot: (path) => req('/api/roots/remove', {
    method: 'POST', body: JSON.stringify({ path }),
  }),
}

/* ---- 原生能力：选文件夹 / 打开路径 ---- */
export const native = {
  async pickFolder() {
    const r = await tauriInvoke('pick_folder')
    if (r) return r
    if (!inTauri()) {
      const manual = window.prompt('请输入文件夹完整路径：')
      return manual && manual.trim() ? manual.trim() : null
    }
    return null
  },
  async openPath(p) {
    if (!p) return null
    const r = await tauriInvoke('open_path', { path: p })
    if (!inTauri()) {
      window.alert('请手动打开：' + p)
    }
    return r
  },
}

/* ---- 等待后端就绪 ----
   后端是 PyInstaller 打包的，冷启动要 2-4 秒。
   这里给 25 秒窗口、250ms 轮询，并且只在成功时返回 true。 */
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

/* ---- 后端状态单例：全局共享，避免每个视图各等一遍 ---- */
let _backendReady = null
export function backendReady() {
  if (!_backendReady) {
    _backendReady = waitForBackend()
  }
  return _backendReady
}
export function resetBackendReady() {
  _backendReady = null
}
