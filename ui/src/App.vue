<script setup>
import { ref, computed, onMounted, onBeforeUnmount, watch } from 'vue'
import Icon from './components/Icon.vue'
import TitleBar from './components/TitleBar.vue'
import ConfirmDialog from './components/ConfirmDialog.vue'
import LibraryView from './views/LibraryView.vue'
import TimelineView from './views/TimelineView.vue'
import PlacesView from './views/PlacesView.vue'
import FavoritesView from './views/FavoritesView.vue'
import DuplicatesView from './views/DuplicatesView.vue'
import QualityView from './views/QualityView.vue'
import RestoreView from './views/RestoreView.vue'
import OrganizeView from './views/OrganizeView.vue'
import RecycleView from './views/RecycleView.vue'
import SettingsView from './views/SettingsView.vue'
import { api, native, waitForBackend } from './api'
import { photoUrl, videoUrl } from './thumb'
import {
  viewerState, close as closeViewer, step as stepViewer,
  toggleOriginal, requestRestore, gotoRestore as viewerGotoRestore,
  previewState, closeImage,
  zoomBy, setZoom, startDrag, onDrag, endDrag,
  previewZoomBy, previewStartDrag, previewOnDrag, previewEndDrag, previewReset,
  markFavorite, rotateBy,
} from './viewer'

/* 查看器里当前这张是不是视频 */
const isVideoRow = computed(() => viewerState.row?.kind === 'video')

/* 视频用系统播放器打开（软件内不带解码器，安装包才能保持 4 MB） */
async function openVideoExternal() {
  const p = viewerState.row?.path
  if (p) await native.openPath(p)
}

/* 视频时长格式化：90 -> 1:30 */
function fmtDur(sec) {
  const t = Math.round(Number(sec) || 0)
  if (t <= 0) return ''
  const h = Math.floor(t / 3600)
  const m = Math.floor((t % 3600) / 60)
  const s = t % 60
  const pad = n => String(n).padStart(2, '0')
  return h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${m}:${pad(s)}`
}


/* 在资源管理器里定位当前查看的文件 */
async function revealCurrent() {
  const p = viewerState.row?.path
  if (p) await native.reveal(p)
}

const NAV_GROUPS = [
  {
    label: '浏览',
    items: [
      { id: 'library', icon: 'photos', label: '照片库', hint: '浏览、筛选全部照片', lab: null },
      { id: 'timeline', icon: 'timeline', label: '时间轴', hint: '按年份浏览照片', lab: null },
      { id: 'places', icon: 'mapPin', label: '地点', hint: '按拍摄地点浏览', lab: 'lab_places' },
      { id: 'favorites', icon: 'starFilled', label: '收藏', hint: '标星的照片单独展示', lab: 'lab_favorites' },
    ],
  },
  {
    label: '处理',
    items: [
      { id: 'duplicates', icon: 'duplicate', label: '重复照片', hint: '找出并清理副本', lab: 'lab_duplicates' },
      { id: 'quality', icon: 'quality', label: '质量待修', hint: '模糊 / 过暗 / 褪色', lab: 'lab_quality' },
      { id: 'restore', icon: 'restore', label: '照片修复', hint: '去黄、去划痕、增强', lab: 'lab_restore' },
    ],
  },
  {
    label: '整理',
    items: [
      { id: 'organize', icon: 'organize', label: '整理归档', hint: '按地点与时间分类输出', lab: 'lab_organize' },
      { id: 'recycle', icon: 'trash', label: '回收站', hint: '删除的文件可从这取回', lab: null },
    ],
  },
]

/* 导航一律常驻：只有「质量待修」「照片修复」受实验室开关控制 */
const visibleNav = computed(() =>
  NAV_GROUPS.map(g => ({
    ...g,
    items: g.items.filter(it => {
      if (it.id === 'quality') return !!(lab.value && lab.value.lab_quality)
      if (it.id === 'restore') return !!(lab.value && lab.value.lab_restore)
      return true
    }),
  })).filter(g => g.items.length))

const SETTINGS_ITEM = { id: 'settings', icon: 'settings', label: '设置', hint: '目录与识别阈值' }

const META = {
  library: { title: '照片库', sub: '浏览、筛选全部照片' },
  timeline: { title: '时间轴', sub: '按年份浏览照片' },
  places: { title: '地点', sub: '按拍摄地点浏览照片' },
  favorites: { title: '收藏', sub: '标星的照片单独展示' },
  duplicates: { title: '重复照片', sub: '找出并清理重复副本' },
  quality: { title: '质量待修', sub: '模糊、过暗、褪色等问题照片' },
  restore: { title: '照片修复', sub: '去黄、去划痕、增强对比' },
  organize: { title: '整理归档', sub: '按地点与时间分类输出' },
  settings: { title: '设置', sub: '目录与识别阈值' },
}

const current = ref('library')
const search = ref('')
const tagFilter = ref(null)
const activeFolder = ref('')
const folderCount = ref(0)
/* 顶部要能展开显示全部已选目录，所以这里保存完整列表 */
const roots = ref([])
const restorePick = ref(null)

const meta = computed(() => META[current.value] || META.library)
/* 只有列照片的页面需要顶部搜索；修复/归档/设置自己处理 */
const SEARCHLESS = new Set(['restore', 'organize', 'settings'])
const showTopSearch = computed(() => !SEARCHLESS.has(current.value))

const hoverId = ref('')
const topMsg = ref('')
const topBusy = ref(false)
const picking = ref(false)
const progress = ref(null)
const stats = ref(null)
const resetting = ref(false)
/* 扫描弹窗状态 */
const scanOpen = ref(false)
const scanDone = ref(null)
const scanMode = ref('incremental')
const cancelling = ref(false)

/* ---------------- 信息编辑弹窗 ---------------- */
const editOpen = ref(false)
const editBusy = ref(false)
const editRow = ref(null)
const editForm = ref({ date: '', place: '', note: '', tags: [] })
const editTagText = ref('')

function openEdit(row) {
  if (!row) return
  editRow.value = row
  const d = (row.dt || '').slice(0, 10)
  editForm.value = {
    date: d,
    place: row.place || '',
    note: row.note || '',
    tags: [...(row.tags || [])],
  }
  editTagText.value = ''
  editOpen.value = true
}

function addEditTag() {
  const t = editTagText.value.trim()
  if (t && !editForm.value.tags.includes(t)) {
    editForm.value.tags = [...editForm.value.tags, t]
  }
  editTagText.value = ''
}

function removeEditTag(t) {
  editForm.value.tags = editForm.value.tags.filter(x => x !== t)
}

async function saveEdit() {
  if (!editRow.value || editBusy.value) return
  editBusy.value = true
  const id = editRow.value.id
  const f = editForm.value
  const payload = {}
  if (!f.date) payload.clear_date = true
  else payload.date = f.date
  if (!f.place) payload.clear_place = true
  else payload.place = f.place
  payload.note = f.note
  const r = await api.editPhoto(id, payload)
  if (r?.ok) {
    await api.setPhotoTags(id, f.tags)
    // 就地同步查看器与已打开列表里的这张
    if (viewerState.row?.id === id) {
      viewerState.row = { ...viewerState.row, dt: f.date ? `${f.date} 00:00:00` : null, place: f.place || null, note: f.note || null }
    }
    topMsg.value = '已保存修改'
    window.dispatchEvent(new CustomEvent('opo:photo-edited', { detail: { id } }))
  } else {
    topMsg.value = '保存失败：' + (r?.error || '未知错误')
  }
  editBusy.value = false
  editOpen.value = false
  setTimeout(() => { topMsg.value = '' }, 4000)
}
const lab = ref({})

/* 只关心这两项实验室开关；其余功能常驻 */
async function refreshLab() {
  const r = await api.settings()
  const s = (r && r.settings) || {}
  lab.value = {
    lab_quality: !!s.lab_quality,
    lab_restore: !!s.lab_restore,
  }
}

function select(id) {
  if (current.value === id) return
  tagFilter.value = null
  current.value = id
}

function filterByTag(name) {
  tagFilter.value = name
  current.value = 'library'
}

function clearTagFilter() { tagFilter.value = null }

/* 大图里的收藏按钮 */
async function toggleFavInViewer() {
  const row = viewerState.row
  if (!row) return
  const next = !row.favorite
  const r = await api.favorite(row.id, next)
  if (r && r.ok) {
    markFavorite(next)
    window.dispatchEvent(new CustomEvent('opo:favorite-changed',
      { detail: { id: row.id, favorite: next } }))
    topMsg.value = next ? '已加入收藏' : '已取消收藏'
    setTimeout(() => { topMsg.value = '' }, 2500)
  }
}

/* 从大图/其它页面点“去修复”：先关大图，再切到修复页并选中那张照片 */
function goRestore(row) {
  if (!row) return
  if (!lab.value.lab_restore) {
    closeViewer()
    topMsg.value = '照片修复未启用，请去「设置 → 实验室功能」里打开'
    setTimeout(() => { topMsg.value = '' }, 6000)
    return
  }
  closeViewer()
  restorePick.value = row
  current.value = 'restore'
}
requestRestore(goRestore)

/* 从已选目录里移除一个。
 *
 * 只删「目录登记」，不动磁盘文件，也不删已扫描的照片记录
 * （照片仍在库里，只是下次扫描不会再从这个目录找新文件）。
 * 想让它的照片也一起消失，用「清除扫描记录」。 */
async function removeFolderAt(path) {
  if (!path) return
  const r = await api.removeRoot(path)
  if (!r || !r.ok) {
    topMsg.value = '移除失败：' + (r?.error || '未知错误')
    return
  }
  roots.value = r.roots || roots.value.filter(x => x !== path)
  folderCount.value = roots.value.length
  // 被删的正好是当前显示的目录时，换成最后一个（或清空）
  if (activeFolder.value === path) {
    activeFolder.value = roots.value[roots.value.length - 1] || ''
  }
  topMsg.value = '已移除目录：' + path
  window.dispatchEvent(new CustomEvent('opo:roots-changed'))
  refreshStats()
}

async function refreshFolders() {
  const r = await api.roots()
  if (r && r.items) {
    roots.value = r.items
    folderCount.value = r.items.length
    activeFolder.value = r.items[r.items.length - 1] || ''
  }
}

async function refreshStats() {
  const st = await api.stats()
  if (st) stats.value = st
}

onMounted(async () => {
  const ok = await waitForBackend(30000)
  if (!ok) { console.warn('后端未在 30 秒内就绪'); return }
  await refreshLab()
  refreshFolders()
  refreshStats()
  window.addEventListener('opo:lab-changed', refreshLab)
  // 索引重置会同时影响统计与目录清单（可能连目录一起清了），两者都要刷
  window.addEventListener('opo:index-reset', () => { refreshStats(); refreshFolders() })
  window.addEventListener('opo:scan-done', refreshStats)
  window.addEventListener('opo:roots-changed', refreshFolders)
  // 设置页的「开始扫描」也走顶部那套进度弹窗
  window.addEventListener('opo:scan-request', onScanRequest)
  // 有历史索引时直接恢复；清库只在设置/重置中显式进行。
  window.dispatchEvent(new CustomEvent('opo:scan-done'))
})

onBeforeUnmount(() => {
  if (pollTimer) clearInterval(pollTimer)
})

/* 「没添加目录就点了开始扫描」的提示弹窗 */
const noFolderAsk = ref(false)

/* 有历史索引时直接恢复；重置由设置页或顶部按钮显式执行。 */

async function addFolderTop() {
  if (picking.value || topBusy.value) return
  picking.value = true
  topMsg.value = '正在打开文件夹选择框…'
  try {
    const dir = await native.pickFolder()
    if (!dir) { topMsg.value = ''; return }
    const ok = await waitForBackend(30000)
    if (!ok) {
      topMsg.value = '处理引擎未响应，请稍后再试'
      return
    }
    const r = await api.addRoot(dir)
    if (r && r.ok) {
      roots.value = r.roots || roots.value
      folderCount.value = roots.value.length
      activeFolder.value = roots.value[roots.value.length - 1] || ''
      topMsg.value = '已添加文件夹：' + dir + '，点「开始扫描」建立索引'
      // 不再自动跳到设置页：用户从顶部添加，就留在当前页面
      if (!roots.value.length) roots.value = [dir]
      window.dispatchEvent(new CustomEvent('opo:roots-changed'))
    } else {
      topMsg.value = '添加失败：' + (r?.error || '后端无响应')
    }
    setTimeout(() => { topMsg.value = '' }, 7000)
  } finally {
    picking.value = false
  }
}

/* 开始识别：后台任务 + 轮询进度条，避免界面卡死看不出在干活 */
let pollTimer = null
async function runAllTop() {
  if (topBusy.value) return
  const roots = await api.roots()
  const list = roots && roots.items ? roots.items : []
  if (!list.length) {
    // 不跳转页面，就地弹窗提示（跳走会打断用户当前在做的事）
    noFolderAsk.value = true
    return
  }
  await startScan(list)
}

/* 设置页发来的扫描请求 */
function onScanRequest(e) {
  const list = e?.detail?.roots
  if (Array.isArray(list) && list.length) startScan(list)
}

/* 真正启动扫描。顶部按钮与设置页共用这一条路径，
   进度弹窗也就自然一致了。 */
async function startScan(list) {
  if (topBusy.value) return
  topBusy.value = true
  topMsg.value = '正在连接处理引擎…'
  const ok = await waitForBackend(30000)
  if (!ok) {
    topBusy.value = false
    topMsg.value = '处理引擎未响应，请稍后再试'
    return
  }
  const r = await api.scanStart(list, scanMode.value === 'full')
  if (!r || !r.ok) {
    topBusy.value = false
    topMsg.value = '扫描失败：' + (r?.error || '后端无响应')
    return
  }
  topMsg.value = ''
  progress.value = { phase: '准备中', percent: 0, done: 0, total: 0, detail: '' }
  scanOpen.value = true          // 弹窗显示进度
  scanDone.value = null
  cancelling.value = false
  window.dispatchEvent(new CustomEvent('opo:scan-started'))
  if (pollTimer) clearInterval(pollTimer)
  pollTimer = setInterval(pollScan, 700)
}

async function cancelScan() {
  if (cancelling.value) return
  cancelling.value = true
  const r = await api.scanCancel()
  if (!r?.ok) {
    cancelling.value = false
    topMsg.value = r?.error || '取消扫描失败'
  }
}

async function pollScan() {
  const p = await api.progress()
  if (!p) return
  progress.value = p
  if (!p.running) {
    clearInterval(pollTimer)
    pollTimer = null
    topBusy.value = false
    if (p.error) {
      scanOpen.value = false
      progress.value = null
      topMsg.value = '扫描失败：' + p.error
      setTimeout(() => { topMsg.value = '' }, 6000)
      return
    }
    cancelling.value = false
    await refreshStats()
    refreshFolders()
    window.dispatchEvent(new CustomEvent('opo:scan-done'))
    scanDone.value = { total: stats.value?.total || 0, cancelled: !!p.cancelled }
  }
}

/* 用户点确认后才关闭弹窗并跳转照片库 */
async function confirmScanDone() {
  const wasCancelled = !!scanDone.value?.cancelled
  scanOpen.value = false
  scanDone.value = null
  progress.value = null
  select('library')
  const key = current.value
  current.value = ''
  setTimeout(() => { current.value = key }, 40)
  topMsg.value = wasCancelled
    ? '已取消扫描，已完成的分析已保留，可继续增量扫描'
    : '扫描完成，共 ' + (stats.value?.total || 0) + ' 张照片'
  setTimeout(() => { topMsg.value = '' }, 5000)
}

/* 顶部重置按钮：确认弹窗状态（与照片删除的确认框同一套样式） */
const resetAsk = ref(false)

/* 顶部重置按钮：走确认弹窗 */
async function resetAllTop() {
  if (resetting.value) return
  resetAsk.value = false
  resetting.value = true
  const r = await api.resetIndex(false)
  resetting.value = false
  if (r && r.ok) {
    roots.value = []
    folderCount.value = 0
    activeFolder.value = ''
    await refreshStats()
    window.dispatchEvent(new CustomEvent('opo:index-reset'))
    window.dispatchEvent(new CustomEvent('opo:roots-changed'))
    // 关键：强制重建当前视图，照片库立即变空，不用切页再切回来
    rebuildCurrentView()
    topMsg.value = '已重置：清除 ' + r.removed + ' 条记录，目录已移除'
  } else {
    topMsg.value = '重置失败：' + (r?.error || '后端无响应')
  }
  setTimeout(() => { topMsg.value = '' }, 6000)
}

/* 通过"清空 key 再设回"触发当前视图整体重建 */
function rebuildCurrentView() {
  const key = current.value
  current.value = ''
  setTimeout(() => { current.value = key }, 20)
}

watch(current, (v) => { if (v === 'library' || v === 'places') refreshStats() })
</script>

<template>
  <div class="app-frame">
    <div class="app-body">
      <TitleBar
        :folder-count="folderCount"
        :photo-count="stats?.total || 0"
        :analyzed="stats?.analyzed || 0"
      />
      <div class="shell">
    <aside class="rail">
      <div class="brand">
        <svg width="26" height="26" viewBox="0 0 24 24" fill="none">
          <rect x="2.5" y="6" width="19" height="13" rx="4" fill="#3b95f7" />
          <circle cx="12" cy="12.5" r="3.6" stroke="#fff" stroke-width="1.7" />
          <rect x="8" y="3.4" width="5" height="3" rx="1.2" fill="#3b95f7" />
        </svg>
        <span class="brand-name">照片整理</span>
      </div>

      <nav class="rail-nav">
        <template v-for="g in visibleNav" :key="g.label">
          <button
            v-for="item in g.items" :key="item.id" class="rail-item"
            :class="{ active: current === item.id }"
            @click="select(item.id)"
            @mouseenter="hoverId = item.id"
            @mouseleave="hoverId = ''"
          >
            <span class="ri-icon"><Icon :name="item.icon" :size="21" /></span>
            <span class="ri-text">
              <span class="ri-label">{{ item.label }}</span>
              <span class="ri-hint">{{ item.hint }}</span>
            </span>
          </button>
        </template>
      </nav>

      <div class="rail-foot">
        <button
          class="rail-item" :class="{ active: current === SETTINGS_ITEM.id }"
          @click="select(SETTINGS_ITEM.id)"
          @mouseenter="hoverId = SETTINGS_ITEM.id"
          @mouseleave="hoverId = ''"
        >
          <span class="ri-icon"><Icon :name="SETTINGS_ITEM.icon" :size="21" /></span>
          <span class="ri-text">
            <span class="ri-label">{{ SETTINGS_ITEM.label }}</span>
            <span class="ri-hint">{{ SETTINGS_ITEM.hint }}</span>
          </span>
        </button>
      </div>
    </aside>

    <div class="main">
      <header class="header">
        <div class="header-left">
          <h1>{{ meta.title }}</h1>
          <!-- 路径徽标：悬停/聚焦时展开显示全部目录 -->
          <div v-if="roots.length" class="folder-wrap" tabindex="0">
            <p class="folder-chip" :title="roots.join('\n')">
              <Icon name="folder" :size="13" />
              <span class="ellipsis">{{ activeFolder }}</span>
              <span v-if="folderCount > 1" class="more">共 {{ folderCount }} 个</span>
            </p>
            <div v-if="folderCount > 0" class="folder-pop">
              <div class="folder-pop-head">已选 {{ folderCount }} 个目录</div>
              <div v-for="(r, i) in roots" :key="r" class="folder-pop-row">
                <span class="idx">{{ i + 1 }}</span>
                <span class="path" :title="r">{{ r }}</span>
                <button
                  class="folder-pop-del" title="移除这个目录"
                  @click.stop.prevent="removeFolderAt(r)"
                >
                  <Icon name="close" :size="12" />
                </button>
              </div>
            </div>
          </div>
          <p v-else>{{ meta.sub }}</p>
        </div>

        <div class="header-right">
          <label v-if="showTopSearch" class="search">
            <Icon name="search" :size="17" />
            <input v-model="search" placeholder="搜索文件名、目录或地点（空格分隔多个词）" />
            <button v-if="search" class="search-clear" @click="search = ''">✕</button>
          </label>
          <button class="btn btn-ghost" :disabled="picking || topBusy" @click="addFolderTop">
            <Icon name="folder" :size="17" /> {{ picking ? '选择中…' : '选择文件夹' }}
          </button>
          <button class="btn btn-primary" :disabled="topBusy || picking" @click="topBusy ? (scanOpen = true) : runAllTop()">
            <Icon name="sparkle" :size="17" /> {{ topBusy ? '查看扫描进度' : '开始扫描' }}
          </button>
          <button
            class="btn btn-ghost" :disabled="resetting || topBusy"
            title="重置：清除所有扫描记录与目录（不删除原文件）"
            @click="resetAsk = true"
          >
            <Icon name="rotateLeft" :size="16" />
          </button>
        </div>
      </header>

      <!-- 重置确认：与其他确认框同一套样式。图标用循环箭头，语义是"回到初始" -->
      <ConfirmDialog
        :show="resetAsk"
        title="重置所有扫描记录？"
        message="将清除全部扫描记录和已添加的照片目录，回到刚安装的状态。原始照片文件不受影响。"
        detail="清除后需要重新添加目录并扫描。"
        confirm-text="重置"
        level="danger"
        icon="rotateLeft"
        :busy="resetting"
        @confirm="resetAllTop"
        @cancel="resetAsk = false"
      />

      <!-- 没添加目录就点开始扫描：就地提示，不跳页 -->
      <div v-if="noFolderAsk" class="scan-mask" @click.self="noFolderAsk = false">
        <div class="scan-dialog startup-dialog">
          <div class="scan-title">
            <Icon name="folder" :size="18" />
            还没有添加照片目录
          </div>
          <div class="scan-detail">
            先选一个装着照片或视频的文件夹，再回来点「开始扫描」。
          </div>
          <div class="scan-actions startup-actions">
            <button class="btn" @click="noFolderAsk = false">知道了</button>
            <button class="btn btn-primary" @click="noFolderAsk = false; addFolderTop()">
              <Icon name="folder" :size="16" /> 现在选择文件夹
            </button>
          </div>
        </div>
      </div>

      <!-- 扫描进度弹窗：进行中显示进度，完成后转确认态 -->
      <div v-if="scanOpen" class="scan-mask">
        <div class="scan-dialog">
          <template v-if="!scanDone">
            <div class="scan-title">
              <span class="scan-spin"></span>
              正在扫描照片
            </div>
            <div class="scan-bar">
              <div class="scan-fill" :style="{ width: (progress?.percent || 0) + '%' }"></div>
            </div>
            <div class="scan-meta">
              <span class="scan-pct">{{ Math.round(progress?.percent || 0) }}%</span>
              <span class="scan-phase">
                {{ progress?.phase || '准备中' }}
                <template v-if="progress?.total"> · {{ progress.done }}/{{ progress.total }}</template>
              </span>
            </div>
            <div class="scan-mode">
              <select v-model="scanMode" :disabled="cancelling" aria-label="扫描方式">
                <option value="incremental">增量扫描（只分析新增和已修改）</option>
                <option value="full">重新全部分析</option>
              </select>
            </div>
            <div v-if="progress?.detail" class="scan-detail">{{ progress.detail }}</div>
            <div class="scan-tip">
              {{ cancelling ? '正在停止，已完成的分析会保留…' : '扫描期间可以切换到其他页面，或取消后继续整理' }}
            </div>
            <div class="scan-actions">
              <button class="btn" :disabled="cancelling" @click="scanOpen = false">后台运行</button>
              <button class="btn btn-danger" :disabled="cancelling" @click="cancelScan">
                {{ cancelling ? '正在取消…' : '取消扫描' }}
              </button>
            </div>
          </template>

          <template v-else>
            <div class="scan-done-ico"><Icon :name="scanDone.cancelled ? 'clock' : 'check'" :size="26" /></div>
            <div class="scan-title done">{{ scanDone.cancelled ? '扫描已取消' : '扫描完成' }}</div>
            <div class="scan-detail">
              {{ scanDone.cancelled ? '已完成的分析已保留，下次增量扫描会处理剩余照片。' : `共识别 ${scanDone.total} 个文件（照片与视频）` }}
            </div>
            <div class="scan-actions">
              <button class="btn btn-primary" @click="confirmScanDone">
                <Icon name="check" :size="16" /> 确定，去看照片
              </button>
            </div>
          </template>
        </div>
      </div>

      <transition name="toast">
        <div v-if="topMsg" class="toast">{{ topMsg }}</div>
      </transition>

      <main class="content">
        <LibraryView
          v-if="current === 'library'"
          :search="search" :tag-filter="tagFilter"
          @clear-tag="clearTagFilter" @repair="goRestore"
        />
        <TimelineView v-else-if="current === 'timeline'" :search="search" />
        <PlacesView v-else-if="current === 'places'" :search="search" />
        <FavoritesView v-else-if="current === 'favorites'" :search="search" />
        <DuplicatesView v-else-if="current === 'duplicates'" :search="search" />
        <QualityView v-else-if="current === 'quality'" :search="search" @repair="goRestore" />
        <RestoreView v-else-if="current === 'restore'" :pick="restorePick" />
        <OrganizeView v-else-if="current === 'organize'" />
        <RecycleView v-else-if="current === 'recycle'" />
        <SettingsView v-else-if="current === 'settings'" />
      </main>

      <!-- 大图查看器 -->
      <div
        v-if="viewerState.visible" class="viewer-overlay"
        @click.self="closeViewer"
        tabindex="0"
        @keydown.esc="closeViewer"
        @keydown.left="stepViewer(-1)"
        @keydown.right="stepViewer(1)"
        @keydown.plus.prevent="zoomBy(1.25)"
        @keydown.equal.prevent="zoomBy(1.25)"
        @keydown.minus.prevent="zoomBy(0.8)"
        @keydown.0.prevent="setZoom(1)"
        @keydown.r.prevent="rotateBy(90)"
        @keydown.shift.r.prevent="rotateBy(-90)"
      >
        <div class="viewer-dialog">
          <!-- 顶栏：只留文件名、分页和关闭。操作按钮全部下移到悬浮组件 -->
          <div class="viewer-bar">
            <strong class="ellipsis">
              <Icon v-if="isVideoRow" name="video" :size="14" />
              {{ viewerState.row?.name || '照片' }}
            </strong>
            <span class="viewer-pager">{{ viewerState.index + 1 }} / {{ viewerState.list.length }}</span>
            <button class="viewer-close" title="关闭（Esc）" @click="closeViewer">
              <Icon name="close" :size="15" />
            </button>
          </div>

          <!-- 标签移到顶部，紧贴文件信息 -->
          <div v-if="viewerState.tags.length" class="viewer-tags top">
            <span v-for="t in viewerState.tags" :key="t">{{ t }}</span>
          </div>

          <!-- 视频：不内嵌播放（不带解码器），点击用系统播放器打开 -->
          <div v-if="isVideoRow" class="viewer-video">
            <div class="video-holder">
              <div class="video-ico"><Icon name="video" :size="46" /></div>
              <strong class="video-name">{{ viewerState.row?.name }}</strong>
              <p class="video-tip">
                视频由系统播放器打开
              </p>
              <button class="btn btn-primary" @click="openVideoExternal">
                <Icon name="play" :size="16" /> 用系统播放器打开
              </button>
            </div>
          </div>

          <!-- 左右切换：照片与视频都能用，绝对定位在内容区两侧 -->
          <button
            v-if="viewerState.list.length > 1"
            class="nav-arrow left" title="上一张（←）"
            @click.stop="stepViewer(-1)"
          >
            <Icon name="chevronLeft" :size="26" />
          </button>
          <button
            v-if="viewerState.list.length > 1"
            class="nav-arrow right" title="下一张（→）"
            @click.stop="stepViewer(1)"
          >
            <Icon name="chevronRight" :size="26" />
          </button>

          <div
            v-if="!isVideoRow"
            class="viewer-image" :class="{ original: viewerState.showOriginal, grab: viewerState.zoom > 1 }"
            @wheel.prevent="zoomBy($event.deltaY < 0 ? 1.12 : 0.89)"
            @mousedown="startDrag"
            @mousemove="onDrag"
            @mouseup="endDrag"
            @mouseleave="endDrag"
          >
            <img
              :src="photoUrl(viewerState.row)"
              :alt="viewerState.row?.name || ''"
              :style="{
                transform: `translate(${viewerState.offsetX}px, ${viewerState.offsetY}px) ` +
                           `scale(${viewerState.zoom}) rotate(${viewerState.rotate}deg)`,
                transition: (viewerState.dragging || viewerState.noTransition)
                  ? 'none' : 'transform .16s ease',
              }"
            />
          </div>
          <!-- 底部悬浮操作组件：毛玻璃胶囊，悬停图片时更实 -->
          <div class="viewer-dock">
            <div class="dock-meta">
              <span>{{ (viewerState.row?.dt || '无日期').slice(0, 19) }}</span>
              <span v-if="viewerState.row?.place">{{ viewerState.row.place }}</span>
              <span v-if="isVideoRow && viewerState.row?.duration">时长 {{ fmtDur(viewerState.row.duration) }}</span>
              <span v-else>{{ viewerState.row?.width }} × {{ viewerState.row?.height }}</span>
            </div>
            <div class="dock-acts">
              <template v-if="!isVideoRow">
                <button class="dock-btn" title="向左旋转（Shift+R）" @click="rotateBy(-90)">
                  <Icon name="rotateLeft" :size="16" />
                </button>
                <button class="dock-btn" title="向右旋转（R）" @click="rotateBy(90)">
                  <Icon name="rotateRight" :size="16" />
                </button>
                <span class="dock-sep"></span>
                <button class="dock-btn" title="缩小（-）" @click="zoomBy(0.8)">−</button>
                <button class="dock-btn pct" title="点击复位 100%" @click="setZoom(1)">
                  {{ Math.round(viewerState.zoom * 100) }}%
                </button>
                <button class="dock-btn" title="放大（+）" @click="zoomBy(1.25)">＋</button>
                <button class="dock-btn" title="切换原始尺寸/适应窗口" @click="toggleOriginal">
                  <Icon name="eye" :size="16" />
                </button>
                <span class="dock-sep"></span>
              </template>
              <button
                class="dock-btn" :class="{ on: viewerState.row?.favorite }"
                :title="viewerState.row?.favorite ? '取消收藏' : '收藏'"
                @click="toggleFavInViewer"
              >
                <Icon :name="viewerState.row?.favorite ? 'starFilled' : 'star'" :size="16" />
              </button>
              <button class="dock-btn" title="在资源管理器中定位" @click="revealCurrent">
                <Icon name="folder" :size="16" />
              </button>
              <button class="dock-btn" title="编辑日期、地点、标签与备注" @click="openEdit(viewerState.row)">
                <Icon name="edit" :size="16" />
              </button>
              <button v-if="!isVideoRow" class="dock-btn" title="去修复" @click="viewerGotoRestore()">
                <Icon name="wrench" :size="16" />
              </button>
            </div>
          </div>
        </div>
      </div>

      <!-- 单图预览（修复前后对比等） -->
      <div
        v-if="previewState.visible" class="viewer-overlay"
        @click.self="closeImage" tabindex="0"
        @keydown.esc="closeImage"
        @keydown.plus.prevent="previewZoomBy(1.25)"
        @keydown.equal.prevent="previewZoomBy(1.25)"
        @keydown.minus.prevent="previewZoomBy(0.8)"
      >
        <div class="viewer-dialog">
          <div class="viewer-bar">
            <strong class="ellipsis">{{ previewState.title || '预览' }}</strong>
            <div class="viewer-actions">
              <button @click="previewZoomBy(0.8)" title="缩小">−</button>
              <button class="zoom-label" title="复位" @click="previewReset">
                {{ Math.round(previewState.zoom * 100) }}%
              </button>
              <button @click="previewZoomBy(1.25)" title="放大">＋</button>
              <button @click="closeImage">关闭</button>
            </div>
          </div>
          <div
            class="viewer-image" :class="{ grab: previewState.zoom > 1 }"
            @wheel.prevent="previewZoomBy($event.deltaY < 0 ? 1.12 : 0.89)"
            @mousedown="previewStartDrag"
            @mousemove="previewOnDrag"
            @mouseup="previewEndDrag"
            @mouseleave="previewEndDrag"
          >
            <img
              :src="previewState.src" :alt="previewState.title"
              :style="{ transform: `translate(${previewState.offsetX}px, ${previewState.offsetY}px) scale(${previewState.zoom})` }"
            />
          </div>
        </div>
      </div>
      </div>
    </div>
    </div>
  </div>

  <!-- 信息编辑弹窗 -->
  <div v-if="editOpen" class="edit-mask" @click.self="editOpen = false">
    <div class="edit-dialog card">
      <div class="edit-head">
        <strong>编辑照片信息</strong>
        <button class="edit-close" @click="editOpen = false"><Icon name="close" :size="15" /></button>
      </div>
      <div class="edit-name">{{ editRow?.name || '照片' }}</div>

      <label class="edit-field">
        <span>拍摄日期</span>
        <input v-model="editForm.date" type="date" placeholder="YYYY-MM-DD" />
      </label>
      <label class="edit-field">
        <span>地点</span>
        <input v-model="editForm.place" type="text" placeholder="例如：北京 · 故宫" />
      </label>
      <label class="edit-field">
        <span>备注</span>
        <textarea v-model="editForm.note" rows="2" placeholder="补充说明，仅保存在本机"></textarea>
      </label>

      <div class="edit-field">
        <span>标签</span>
        <div class="edit-tags">
          <span v-for="t in editForm.tags" :key="t" class="edit-tag">
            {{ t }} <button @click="removeEditTag(t)">×</button>
          </span>
          <input
            v-model="editTagText" class="edit-tag-input" placeholder="输入后回车添加"
            @keydown.enter.prevent="addEditTag" @blur="addEditTag"
          />
        </div>
      </div>

      <p class="edit-tip">修改只写进本软件数据库，不改动原照片文件；重新扫描时会保留你的修改。</p>
      <div class="edit-actions">
        <button class="btn" :disabled="editBusy" @click="editOpen = false">取消</button>
        <button class="btn btn-primary" :disabled="editBusy" @click="saveEdit">
          {{ editBusy ? '保存中…' : '保存' }}
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
/* 无边框窗口的圆角外框。
 *
 * 之前外层留了 6px 透明 padding 来「放阴影」，但窗口本身是透明的：
 * 那 6px 在深色桌面上就是一圈黑边，看起来像窗口没对齐。
 * 现在铺满整个窗口，圆角直接由窗口自身裁切，不再留透明缝。
 */
.app-frame {
  height: 100vh; width: 100vw;
  padding: 0;
  margin: 0;
  background: transparent;
  overflow: hidden;
  box-sizing: border-box;
}
.app-body {
  height: 100%; width: 100%;
  display: flex; flex-direction: column;
  border-radius: 12px;
  overflow: hidden;
  background: var(--page-bg);
  /* 细描边改成向内收，避免在浅色桌面上出现深色轮廓 */
  border: 0;
  box-shadow: inset 0 0 0 1px var(--line-strong);
  box-sizing: border-box;
}
/* 最大化时连圆角也去掉，彻底铺满 */
:global(.app-maximized) .app-frame { padding: 0; }
:global(.app-maximized) .app-body {
  border-radius: 0;
  box-shadow: none;
}
.shell { display: flex; flex: 1; min-height: 0; background: var(--page-bg); }
.rail {
  position: relative; z-index: 30; display: flex; width: 72px; flex: none;
  flex-direction: column; padding: 14px 0 12px; gap: 4px;
  overflow: visible; background: var(--rail-bg, var(--page-bg));
}
.rail-nav { display: flex; flex-direction: column; align-items: center; gap: 4px; flex: 1; overflow: visible; width: 72px; }
.brand { display: flex; align-items: center; justify-content: center; height: 48px; margin-bottom: 8px; flex: none; }
.brand svg { flex: none; }
.brand-name { display: none; }
.rail-item {
  position: relative; display: flex; align-items: center; justify-content: center;
  flex: none; width: 46px; height: 46px; padding: 0; border-radius: var(--r-md);
  color: var(--text-tertiary);
  transition: background var(--dur) var(--ease), color var(--dur) var(--ease);
}
.rail-item:hover { background: var(--surface-hover); color: var(--text-primary); }
.rail-item.active { background: var(--tint-strong); color: var(--brand-400); box-shadow: inset 0 0 0 1px rgba(59,149,247,.28); }
.rail-item.active::before {
  content: ''; position: absolute; left: 0; top: 50%; transform: translateY(-50%);
  width: 3px; height: 22px; border-radius: 0 var(--r-pill) var(--r-pill) 0;
  background: var(--brand-400);
}
.ri-icon { display: grid; place-items: center; width: 26px; height: 26px; flex: none; }
.ri-text {
  position: absolute; left: 54px; top: 50%;
  transform: translateY(-50%) translateX(-6px);
  display: flex; flex-direction: column; gap: 1px; min-width: 150px;
  padding: 9px 14px; border-radius: var(--r-sm); background: var(--surface-3);
  box-shadow: var(--shadow-lg); opacity: 0; pointer-events: none; white-space: nowrap;
  transition: opacity .15s var(--ease), transform .18s var(--ease); z-index: 50;
}
.rail-item:hover .ri-text { opacity: 1; transform: translateY(-50%) translateX(0); }
.ri-label { font-size: var(--fs-sm); font-weight: 600; color: var(--text-primary); }
.ri-hint { font-size: var(--fs-xs); font-weight: 400; color: var(--text-secondary); }
.rail-foot { flex: none; display: flex; justify-content: center; padding: 10px 0 0; margin-top: 6px; border-top: 1px solid var(--line); width: 72px; }
.main { display: flex; flex: 1; min-width: 0; flex-direction: column; padding: 14px 16px 0 10px; }
/* 顶部栏：与页面底色一致的一体式，不再用"更亮的面+边框"造成两种配色 */
.header {
  display: flex; height: var(--header-h); flex: none; align-items: center;
  justify-content: space-between; gap: 20px; background: transparent;
  border-radius: 0; border: 0; border-bottom: 1px solid var(--line);
  padding: 0 4px 0 2px; margin-bottom: 14px;
}
.header-left { display: flex; flex-direction: column; gap: 5px; min-width: 0; }
.header-left h1 {
  margin: 0; font-family: var(--font-display); font-size: var(--fs-xl);
  font-weight: 700; letter-spacing: -.02em; line-height: 1.25;
}
.header-left p { margin: 0; font-size: var(--fs-sm); color: var(--text-tertiary); line-height: 1.4; }
.folder-chip {
  display: inline-flex; align-items: center; gap: 6px; max-width: 460px;
  padding: 4px 11px; border-radius: var(--r-pill); background: var(--tint);
  color: var(--brand-300) !important; font-size: var(--fs-xs); cursor: default;
}
.folder-chip svg { flex: none; color: var(--brand-400); }
.folder-chip .ellipsis { min-width: 0; }
.folder-chip .more { flex: none; padding: 0 6px; border-radius: var(--r-pill); background: var(--tint-strong); color: var(--brand-300); font-weight: 600; }

/* 路径徽标悬停展开：显示全部已选目录 */
.folder-wrap { position: relative; }
.folder-pop {
  position: absolute; left: 0; top: calc(100% + 6px); z-index: 60;
  min-width: 320px; max-width: 620px; max-height: 320px; overflow-y: auto;
  padding: 8px; border-radius: var(--r-md);
  background: var(--surface-3); box-shadow: var(--shadow-lg);
  opacity: 0; visibility: hidden; transform: translateY(-4px);
  transition: opacity .15s var(--ease), transform .15s var(--ease), visibility .15s;
}
.folder-wrap:hover .folder-pop,
.folder-wrap:focus-within .folder-pop { opacity: 1; visibility: visible; transform: translateY(0); }
.folder-pop-head {
  padding: 3px 8px 7px; font-size: var(--fs-xs); color: var(--text-tertiary);
  border-bottom: 1px solid var(--line); margin-bottom: 5px;
}
.folder-pop-row { display: flex; gap: 8px; padding: 5px 8px; border-radius: var(--r-xs); align-items: center; }
.folder-pop-row:hover { background: var(--surface-hover); }
.folder-pop-row .idx {
  flex: none; width: 18px; color: var(--text-tertiary);
  font-size: var(--fs-xs); text-align: right;
}
.folder-pop-row .path {
  flex: 1; min-width: 0; font-size: var(--fs-xs); color: var(--text-secondary);
  word-break: break-all; line-height: 1.5;
}
/* 每行的移除按钮：默认淡显，悬停该行时明显 */
.folder-pop-del {
  flex: none; width: 20px; height: 20px;
  display: grid; place-items: center;
  border-radius: 4px; color: var(--text-tertiary);
  opacity: 0; transition: opacity .12s ease, background .12s ease, color .12s ease;
}
.folder-pop-row:hover .folder-pop-del { opacity: 1; }
.folder-pop-del:hover { background: rgba(239,68,68,.16); color: var(--danger); }

.btn-danger-ghost { background: rgba(239,68,68,.12); color: var(--danger); padding: 0 12px; }
.btn-danger-ghost:hover { background: rgba(239,68,68,.22); }
.btn-danger-ghost:disabled { opacity: .55; }

/* 扫描进度弹窗 */
.scan-mask {
  position: fixed; inset: 0; z-index: 120; display: grid; place-items: center;
  background: rgba(6,8,12,.72); backdrop-filter: blur(6px);
}
.scan-dialog {
  width: min(460px, 90vw); padding: 26px 28px;
  border-radius: var(--r-xl); background: var(--surface);
  box-shadow: var(--shadow-lg); text-align: center;
}
.scan-title {
  display: flex; align-items: center; justify-content: center; gap: 10px;
  font-size: var(--fs-lg); font-weight: 600; margin-bottom: 18px;
}
.scan-title.done { color: var(--success); }

/* 启动询问弹窗 */
.startup-dialog { text-align: left; }
.startup-dialog .scan-title { justify-content: flex-start; margin-bottom: 14px; }
.startup-dialog .scan-detail { line-height: 1.9; margin-bottom: 12px; }
.startup-tip {
  font-size: var(--fs-xs); color: var(--text-secondary);
  line-height: 1.6; margin-bottom: 18px;
  padding: 8px 10px; border-radius: var(--r-sm);
  background: rgba(245,158,11,.10);
  border-left: 2px solid var(--warning);
}
.startup-actions {
  justify-content: flex-end; gap: 10px; display: flex;
}
/* 清除记录时的「同时清目录」勾选项 */
.startup-opt {
  display: flex; align-items: center; gap: 8px;
  margin-bottom: 16px; padding: 8px 10px;
  border-radius: var(--r-sm);
  background: var(--surface-2);
  font-size: var(--fs-xs); color: var(--text-secondary);
  cursor: pointer; user-select: none;
}
.startup-opt:hover { background: var(--surface-3); }
.startup-opt input { width: 14px; height: 14px; accent-color: var(--danger); cursor: pointer; flex: none; }
.btn-danger {
  background: rgba(239,68,68,.14);
  color: var(--danger);
  border: 1px solid rgba(239,68,68,.35);
}
.btn-danger:hover {
  background: rgba(239,68,68,.24);
}
.scan-spin {
  width: 16px; height: 16px; border-radius: 50%; flex: none;
  border: 2px solid var(--tint-strong); border-top-color: var(--brand-400);
  animation: opo-spin .9s linear infinite;
}
@keyframes opo-spin { to { transform: rotate(360deg); } }
.scan-bar {
  height: 8px; border-radius: var(--r-pill); background: var(--surface-2);
  overflow: hidden; margin-bottom: 10px;
}
.scan-fill {
  height: 100%; border-radius: var(--r-pill);
  background: var(--grad-blue); transition: width .3s var(--ease);
}
.scan-meta { display: flex; align-items: center; justify-content: space-between; gap: 10px; }
.scan-pct { font-size: var(--fs-xl); font-weight: 700; color: var(--brand-300); font-variant-numeric: tabular-nums; }
.scan-phase { font-size: var(--fs-xs); color: var(--text-tertiary); text-align: right; }
.scan-detail { margin-top: 8px; font-size: var(--fs-xs); color: var(--text-tertiary); word-break: break-all; }
.scan-detail b { color: var(--text-primary); font-size: var(--fs-md); }
.scan-tip { margin-top: 14px; font-size: var(--fs-xs); color: var(--text-dimmed, var(--text-tertiary)); }
.scan-done-ico {
  display: grid; place-items: center; width: 54px; height: 54px; margin: 0 auto 14px;
  border-radius: 50%; background: rgba(34,197,94,.14); color: var(--success);
}
.scan-actions { display: flex; justify-content: center; gap: 10px; margin-top: 20px; }
.scan-mode { margin-top: 16px; text-align: left; }
.scan-mode select {
  width: 100%; padding: 9px 11px; border-radius: var(--r-sm);
  background: var(--surface-2); color: var(--text-primary);
  border: 1px solid var(--line); font-size: var(--fs-sm);
}

/* 信息编辑弹窗 */
.edit-mask {
  position: fixed; inset: 0; z-index: 200;
  display: grid; place-items: center;
  background: rgba(6,8,12,.72); backdrop-filter: blur(6px);
}
.edit-dialog {
  width: min(420px, 92vw); padding: 22px 24px;
  border-radius: var(--r-xl); background: var(--surface);
  box-shadow: var(--shadow-lg);
  max-height: 88vh; overflow-y: auto;
}
.edit-head {
  display: flex; align-items: center; justify-content: space-between;
  margin-bottom: 6px;
}
.edit-head strong { font-size: var(--fs-lg); }
.edit-close {
  display: grid; place-items: center; width: 26px; height: 26px;
  border-radius: var(--r-xs); color: var(--text-tertiary);
}
.edit-close:hover { background: var(--surface-3); color: var(--text-primary); }
.edit-name {
  margin-bottom: 14px; font-size: var(--fs-sm); color: var(--text-tertiary);
  word-break: break-all;
}
.edit-field {
  display: flex; flex-direction: column; gap: 6px;
  margin-bottom: 12px; font-size: var(--fs-sm); color: var(--text-secondary);
}
.edit-field input, .edit-field select, .edit-field textarea {
  padding: 9px 11px; border-radius: var(--r-sm);
  background: var(--surface-2); color: var(--text-primary);
  border: 1px solid var(--line); font-size: var(--fs-sm);
  font-family: inherit; resize: vertical;
}
.edit-tags {
  display: flex; flex-wrap: wrap; gap: 6px; align-items: center;
  padding: 7px 9px; border-radius: var(--r-sm);
  background: var(--surface-2); border: 1px solid var(--line);
}
.edit-tag {
  display: inline-flex; align-items: center; gap: 5px;
  padding: 3px 8px; border-radius: var(--r-pill);
  background: var(--tint-strong); color: var(--brand-300); font-size: var(--fs-xs);
}
.edit-tag button { color: var(--brand-300); font-size: 13px; line-height: 1; }
.edit-tag-input {
  flex: 1; min-width: 90px; padding: 3px 6px;
  background: transparent; border: 0; color: var(--text-primary);
  font-size: var(--fs-xs);
}
.edit-tip {
  margin: 2px 0 16px; font-size: var(--fs-xs);
  color: var(--text-tertiary); line-height: 1.6;
}
.edit-actions { display: flex; justify-content: flex-end; gap: 10px; }
.header-right { display: flex; align-items: center; gap: 10px; }
/* 搜索框等控件整体左移一点，给右侧垃圾桶留出呼吸感 */
.header { padding-right: 10px; }
.header-right { margin-right: 6px; }

/* 一体式信息徽标：和参考图里"磁盘容量条"结构一致——
   图标 + 数值 + 细进度条 + 补充说明，全部装在一个胶囊里 */
.stat-chip {
  display: flex; align-items: center; gap: 9px;
  height: 40px; padding: 0 14px; flex: none;
  border-radius: var(--r-md);
  background: var(--surface-2);
  font-size: var(--fs-sm); color: var(--text-secondary);
}
.stat-chip svg { color: var(--text-tertiary); flex: none; }
.stat-chip-label { font-weight: 600; color: var(--text-primary); white-space: nowrap; }
.stat-chip-bar {
  width: 84px; height: 4px; flex: none;
  border-radius: var(--r-pill); background: var(--surface-3); overflow: hidden;
}
.stat-chip-fill {
  display: block; height: 100%; border-radius: var(--r-pill);
  background: var(--brand-400); transition: width .3s var(--ease);
}
.stat-chip-sub { font-size: var(--fs-xs); color: var(--text-tertiary); white-space: nowrap; }
.search {
  display: flex; align-items: center; gap: 9px; height: 40px; width: 280px;
  padding: 0 14px; border-radius: var(--r-md); background: var(--surface);
  color: var(--text-tertiary); box-shadow: var(--shadow-sm);
  transition: box-shadow var(--dur) var(--ease);
}
.search:focus-within { box-shadow: var(--shadow-md), 0 0 0 2px var(--tint-strong); color: var(--brand-400); }
.search input { flex: 1; min-width: 0; font-size: var(--fs-md); color: var(--text-primary); }
.search input::placeholder { color: var(--text-tertiary); }
.search-clear {
  display: grid; place-items: center; width: 18px; height: 18px; flex: none;
  border-radius: 50%; background: var(--surface-3); color: var(--text-tertiary);
  font-size: 10px;
}
.search-clear:hover { color: var(--text-primary); }
.progress { flex: none; display: flex; align-items: center; gap: 10px; margin-bottom: 10px; font-size: var(--fs-xs); color: var(--text-tertiary); }
.progress-bar { width: 240px; height: 6px; border-radius: var(--r-pill); background: var(--surface-2); overflow: hidden; flex: none; }
.progress-fill { height: 100%; border-radius: var(--r-pill); background: var(--grad-blue); transition: width .3s var(--ease); }
.progress-pct { min-width: 38px; color: var(--brand-300); font-variant-numeric: tabular-nums; font-weight: 600; }
.progress-text { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.content {
  flex: 1; min-height: 0; overflow: hidden; padding-bottom: 16px;
  position: relative;
}
/* 环境光晕：随强调色变化的一层极淡背景光，让"换主题"有整体氛围变化 */
.content::before {
  content: ''; position: absolute; inset: 0; pointer-events: none;
  background: radial-gradient(120% 60% at 50% -10%, var(--ambient-glow, transparent), transparent 70%);
}
.toast {
  position: fixed; left: 50%; bottom: 28px; transform: translateX(-50%); z-index: 90;
  padding: 11px 22px; border-radius: var(--r-pill); background: var(--surface-3);
  color: var(--text-primary); font-size: var(--fs-sm); box-shadow: var(--shadow-lg);
  white-space: nowrap;
}
.toast-enter-active, .toast-leave-active { transition: opacity .2s var(--ease), transform .2s var(--ease); }
.toast-enter-from, .toast-leave-to { opacity: 0; transform: translateX(-50%) translateY(8px); }
.viewer-overlay {
  position: fixed; inset: 0; z-index: 95; display: grid; place-items: center;
  padding: 12px; background: rgba(6,8,12,.86); backdrop-filter: blur(8px);
}
.viewer-dialog {
  position: relative;
  display: flex; flex-direction: column; width: min(1680px, 97vw);
  height: min(1040px, 96vh); border-radius: var(--r-xl);
  background: var(--surface); box-shadow: var(--shadow-lg); overflow: hidden;
}
.viewer-bar {
  display: flex; align-items: center; gap: 14px;
  flex: none; padding: 12px 16px;
}
.viewer-bar strong { flex: 1; min-width: 0; font-size: var(--fs-md); font-weight: 600; }
.viewer-pager {
  flex: none; font-size: var(--fs-xs); color: var(--text-tertiary);
  font-variant-numeric: tabular-nums;
}
.viewer-close {
  flex: none; width: 30px; height: 30px;
  display: grid; place-items: center;
  border-radius: var(--r-sm);
  color: var(--text-tertiary);
  transition: background var(--dur) var(--ease), color var(--dur) var(--ease);
}
.viewer-close:hover { background: rgba(239,68,68,.16); color: var(--danger); }
.viewer-image {
  position: relative; flex: 1; min-height: 0; display: grid; place-items: center;
  /* 不要内边距：图片用 inset:0 撑满，任何 padding 都会在四周露出一圈深色底，
     看起来就是"图片放不满、边框发黑" */
  padding: 0; background: #07090d; overflow: auto;
}

/* 左右切换箭头：贴边垂直居中，平时半透明，悬停变实 */
.nav-arrow {
  position: absolute; top: 50%; transform: translateY(-50%);
  width: 44px; height: 76px; z-index: 5;
  display: grid; place-items: center;
  border-radius: 10px;
  background: rgba(0,0,0,.42); color: #fff;
  opacity: .35;
  transition: opacity .15s ease, background .15s ease;
}
.nav-arrow.left { left: 8px; }
.nav-arrow.right { right: 8px; }
.nav-arrow:hover { opacity: 1; background: rgba(0,0,0,.72); }
.nav-arrow:active { transform: translateY(-50%) scale(.94); }

/* 视频区：不内嵌播放器（软件不带解码器），显示引导卡片 */
.viewer-video {
  position: relative; flex: 1; min-height: 0;
  display: flex; align-items: center; justify-content: center;
  background: #07090d; padding: 24px;
}
.video-holder {
  display: flex; flex-direction: column; align-items: center; gap: 14px;
  max-width: 460px; text-align: center;
}
.video-ico {
  display: grid; place-items: center;
  width: 88px; height: 88px;
  border-radius: 24px;
  background: linear-gradient(145deg, rgba(90,174,254,.18), rgba(59,149,247,.08));
  border: 1px solid rgba(90,174,254,.22);
  color: var(--brand-300);
}
.video-name {
  font-size: var(--fs-lg); font-weight: 600; color: #e8e8ea;
  word-break: break-all;
}
.video-tip {
  margin: 0; font-size: var(--fs-sm); line-height: 1.65;
  color: var(--text-tertiary);
}
/* 关键：用 absolute 撑满容器再 contain，百分比高度在滚动容器里不可靠，
   高分辨率图会被顶出可视区（之前"显示不全"就是这个原因）。 */
.viewer-image img {
  position: absolute; inset: 0; margin: auto;
  width: 100%; height: 100%;
  object-fit: contain;
  /* 图片本身不要圆角：留着会在四角透出容器的深色底 */
  border-radius: 0;
  transform-origin: center center;
  user-select: none; -webkit-user-drag: none;
  transition: transform .08s linear;
}
.viewer-image.grab { cursor: grab; }
.viewer-image.grab:active { cursor: grabbing; }
.viewer-image.original { display: block; overflow: auto; }
.viewer-image.original img {
  position: static; inset: auto; margin: 0;
  width: auto; height: auto; max-width: none; max-height: none;
}
.viewer-tags { display: flex; flex-wrap: wrap; gap: 7px; flex: none; padding: 0 16px 13px; }
.viewer-tags span { padding: 4px 10px; border-radius: var(--r-pill); background: var(--tint); color: var(--brand-300); font-size: var(--fs-xs); }
/* 顶部标签：紧贴文件名下方，间距收紧 */
.viewer-tags.top { padding: 0 16px 10px; }

/* ==========================================================================
   底部悬浮操作组件
   --------------------------------------------------------------------------
   原来是横跨整个窗口的信息条，占了很大一块。改成悬浮在图片上的毛玻璃胶囊：
   平时半透明融入画面，鼠标移入变实，不遮挡照片也不抢视线。
   ========================================================================== */
.viewer-dock {
  position: absolute; left: 50%; bottom: 18px; transform: translateX(-50%);
  z-index: 8;
  display: flex; flex-direction: column; align-items: center; gap: 7px;
  padding: 9px 12px;
  border-radius: var(--r-lg);
  background: rgba(20, 23, 28, .72);
  backdrop-filter: blur(18px) saturate(1.6);
  -webkit-backdrop-filter: blur(18px) saturate(1.6);
  border: 1px solid rgba(255, 255, 255, .10);
  box-shadow: 0 10px 34px rgba(0, 0, 0, .48);
  opacity: .92;
  transition: opacity .18s ease, background .18s ease;
  max-width: calc(100% - 120px);
}
.viewer-dock:hover { opacity: 1; background: rgba(20, 23, 28, .92); }

.dock-meta {
  display: flex; align-items: center; gap: 12px; flex-wrap: wrap;
  justify-content: center;
  font-size: var(--fs-xs); color: rgba(255,255,255,.62);
  max-width: 100%;
}
.dock-meta span { white-space: nowrap; }

.dock-acts { display: flex; align-items: center; gap: 4px; }
.dock-btn {
  display: grid; place-items: center;
  min-width: 32px; height: 32px; padding: 0 8px;
  border-radius: var(--r-sm);
  color: rgba(255,255,255,.80);
  font-size: var(--fs-sm);
  transition: background .14s ease, color .14s ease;
}
.dock-btn:hover { background: rgba(255,255,255,.14); color: #fff; }
.dock-btn.on { color: #fbbf24; }
.dock-btn.pct {
  min-width: 52px;
  color: #8ac6ff;
  font-variant-numeric: tabular-nums;
  font-weight: 600;
}
.dock-sep {
  width: 1px; height: 18px; flex: none;
  background: rgba(255,255,255,.14);
  margin: 0 3px;
}
</style>
