<script setup>
import { ref, computed, onMounted, watch } from 'vue'
import Icon from './components/Icon.vue'
import TitleBar from './components/TitleBar.vue'
import LibraryView from './views/LibraryView.vue'
import TimelineView from './views/TimelineView.vue'
import PlacesView from './views/PlacesView.vue'
import FavoritesView from './views/FavoritesView.vue'
import DuplicatesView from './views/DuplicatesView.vue'
import QualityView from './views/QualityView.vue'
import RestoreView from './views/RestoreView.vue'
import OrganizeView from './views/OrganizeView.vue'
import SettingsView from './views/SettingsView.vue'
import { api, native, waitForBackend } from './api'
import { photoUrl } from './thumb'
import {
  viewerState, close as closeViewer, step as stepViewer,
  toggleOriginal, requestRestore, gotoRestore as viewerGotoRestore,
  previewState, closeImage,
  zoomBy, setZoom, startDrag, onDrag, endDrag,
  previewZoomBy, previewStartDrag, previewOnDrag, previewEndDrag, previewReset,
  markFavorite,
} from './viewer'

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

/* 已识别比例，用于标题栏里那条细进度条 */
const analyzedPct = computed(() => {
  const s = stats.value
  if (!s || !s.total) return 0
  return Math.round(100 * (s.analyzed || 0) / s.total)
})
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
  window.addEventListener('opo:index-reset', refreshStats)
  window.addEventListener('opo:scan-done', refreshStats)
  window.addEventListener('opo:roots-changed', refreshFolders)
})

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
  topBusy.value = true
  topMsg.value = '正在连接处理引擎…'
  const ok = await waitForBackend(30000)
  if (!ok) {
    topBusy.value = false
    topMsg.value = '处理引擎未响应。请确认 backend 文件夹与本程序在同一目录'
    return
  }
  const roots = await api.roots()
  const list = roots && roots.items ? roots.items : []
  if (!list.length) {
    topBusy.value = false
    topMsg.value = '请先点「选择文件夹」添加照片目录'
    select('settings')
    return
  }
  const r = await api.scanStart(list)
  if (!r || !r.ok) {
    topBusy.value = false
    topMsg.value = '扫描失败：' + (r?.error || '后端无响应')
    return
  }
  topMsg.value = ''
  progress.value = { phase: '准备中', percent: 0, done: 0, total: 0, detail: '' }
  scanOpen.value = true          // 弹窗显示进度
  scanDone.value = null
  pollTimer = setInterval(pollScan, 700)
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
    await refreshStats()
    refreshFolders()
    // 保留弹窗，切换成"扫描完成"确认态
    scanDone.value = { total: stats.value?.total || 0 }
  }
}

/* 用户点确认后才关闭弹窗并跳转照片库 */
async function confirmScanDone() {
  scanOpen.value = false
  scanDone.value = null
  progress.value = null
  select('library')
  const key = current.value
  current.value = ''
  setTimeout(() => { current.value = key }, 40)
  topMsg.value = '扫描完成，共 ' + (stats.value?.total || 0) + ' 张照片'
  setTimeout(() => { topMsg.value = '' }, 5000)
}

/* 顶部垃圾桶：重置所有扫描结果与目录 */
async function resetAllTop() {
  if (resetting.value) return
  if (!confirm('将清除所有扫描记录和已添加的照片目录（原始照片文件不受影响）。继续？')) return
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
                <span class="path">{{ r }}</span>
              </div>
            </div>
          </div>
          <p v-else>{{ meta.sub }}</p>
        </div>

        <div class="header-right">
          <label v-if="showTopSearch" class="search">
            <Icon name="search" :size="17" />
            <input v-model="search" placeholder="搜索照片、地点或日期" />
            <button v-if="search" class="search-clear" @click="search = ''">✕</button>
          </label>
          <button class="btn btn-ghost" :disabled="picking || topBusy" @click="addFolderTop">
            <Icon name="folder" :size="17" /> {{ picking ? '选择中…' : '选择文件夹' }}
          </button>
          <button class="btn btn-primary" :disabled="topBusy || picking" @click="runAllTop">
            <Icon name="sparkle" :size="17" /> {{ topBusy ? '扫描中…' : '开始扫描' }}
          </button>
          <button
            class="btn btn-danger-ghost" :disabled="resetting || topBusy"
            title="重置：清除所有扫描记录与目录（不删除原文件）"
            @click="resetAllTop"
          >
            <Icon name="trash" :size="17" />
          </button>
        </div>
      </header>

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
            <div v-if="progress?.detail" class="scan-detail">{{ progress.detail }}</div>
            <div class="scan-tip">扫描期间请勿关闭窗口，完成后会提示确认</div>
          </template>

          <template v-else>
            <div class="scan-done-ico"><Icon name="check" :size="26" /></div>
            <div class="scan-title done">扫描完成</div>
            <div class="scan-detail">共识别 <b>{{ scanDone.total }}</b> 张照片</div>
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
      >
        <div class="viewer-dialog">
          <div class="viewer-bar">
            <strong class="ellipsis">{{ viewerState.row?.name || '照片' }}</strong>
            <div class="viewer-actions">
              <button @click="zoomBy(0.8)" title="缩小（-）">−</button>
              <button class="zoom-label" title="点击复位 100%" @click="setZoom(1)">{{ Math.round(viewerState.zoom * 100) }}%</button>
              <button @click="zoomBy(1.25)" title="放大（+）">＋</button>
              <button @click="toggleOriginal">
                <Icon name="eye" :size="14" />
                {{ viewerState.showOriginal ? '适应窗口' : '原始尺寸' }}
              </button>
              <button @click="stepViewer(-1)">← 上一张</button>
              <button @click="stepViewer(1)">下一张 →</button>
              <button @click="toggleFavInViewer">
                <Icon :name="viewerState.row?.favorite ? 'starFilled' : 'star'" :size="14" />
                {{ viewerState.row?.favorite ? '取消收藏' : '收藏' }}
              </button>
              <button @click="viewerGotoRestore()">
                <Icon name="wrench" :size="14" /> 去修复
              </button>
              <button @click="closeViewer">关闭</button>
            </div>
          </div>
          <div
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
              :style="{ transform: `translate(${viewerState.offsetX}px, ${viewerState.offsetY}px) scale(${viewerState.zoom})` }"
            />
          </div>
          <div class="viewer-info">
            <span>{{ (viewerState.row?.dt || '无日期').slice(0, 19) }}</span>
            <span v-if="viewerState.row?.place">{{ viewerState.row.place }}</span>
            <span>{{ viewerState.row?.width }} × {{ viewerState.row?.height }}</span>
            <span class="ellipsis">{{ viewerState.row?.path }}</span>
            <span class="pager">{{ viewerState.index + 1 }} / {{ viewerState.list.length }}</span>
          </div>
          <div v-if="viewerState.tags.length" class="viewer-tags">
            <span v-for="t in viewerState.tags" :key="t">{{ t }}</span>
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
</template>

<style scoped>
/* 无边框窗口的圆角外框：外层透明留出阴影空间，内层才是真正的窗口内容 */
.app-frame {
  height: 100vh; width: 100vw;
  padding: 6px;
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
  border: 1px solid var(--line-strong);
  box-shadow: 0 10px 34px rgba(0, 0, 0, .55);
  box-sizing: border-box;
}
/* 最大化时去掉圆角、边框与留白，铺满屏幕 */
:global(.app-maximized) .app-frame { padding: 0; }
:global(.app-maximized) .app-body {
  border-radius: 0;
  border-color: transparent;
  box-shadow: none;
}
.shell { display: flex; flex: 1; min-height: 0; background: var(--page-bg); }
.rail {
  position: relative; z-index: 30; display: flex; width: 72px; flex: none;
  flex-direction: column; padding: 14px 0 12px; gap: 4px;
  overflow: visible; background: var(--page-bg);
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
.folder-pop-row { display: flex; gap: 8px; padding: 5px 8px; border-radius: var(--r-xs); }
.folder-pop-row:hover { background: var(--surface-hover); }
.folder-pop-row .idx {
  flex: none; width: 18px; color: var(--text-tertiary);
  font-size: var(--fs-xs); text-align: right;
}
.folder-pop-row .path {
  flex: 1; min-width: 0; font-size: var(--fs-xs); color: var(--text-secondary);
  word-break: break-all; line-height: 1.5;
}

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
.scan-actions { display: flex; justify-content: center; margin-top: 20px; }
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
.content { flex: 1; min-height: 0; overflow: hidden; padding-bottom: 16px; }
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
  padding: 20px; background: rgba(6,8,12,.86); backdrop-filter: blur(8px);
}
.viewer-dialog {
  display: flex; flex-direction: column; width: min(1240px, 95vw);
  height: min(900px, 94vh); border-radius: var(--r-xl);
  background: var(--surface); box-shadow: var(--shadow-lg); overflow: hidden;
}
.viewer-bar {
  display: flex; align-items: center; justify-content: space-between;
  gap: 16px; flex: none; padding: 11px 16px; border-bottom: 1px solid var(--line);
}
.viewer-actions { display: flex; gap: 7px; flex: none; }
.viewer-actions button {
  display: inline-flex; align-items: center; gap: 5px; padding: 7px 12px;
  border-radius: var(--r-sm); background: var(--surface-2); font-size: var(--fs-sm);
}
.viewer-actions button:hover { background: var(--surface-3); }
.zoom-label { min-width: 58px; color: var(--brand-300); font-variant-numeric: tabular-nums; }
.viewer-image {
  position: relative; flex: 1; min-height: 0; display: grid; place-items: center;
  padding: 14px; background: #07090d; overflow: auto;
}
/* 关键：用 absolute 撑满容器再 contain，百分比高度在滚动容器里不可靠，
   高分辨率图会被顶出可视区（之前"显示不全"就是这个原因）。 */
.viewer-image img {
  position: absolute; inset: 0; margin: auto;
  width: 100%; height: 100%;
  object-fit: contain;
  border-radius: var(--r-sm);
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
.viewer-info {
  display: flex; gap: 16px; flex: none; padding: 9px 16px;
  font-size: var(--fs-xs); color: var(--text-tertiary);
}
.viewer-info span:nth-last-child(2) { flex: 1; min-width: 0; }
.pager { flex: none; color: var(--text-secondary); }
.viewer-tags { display: flex; flex-wrap: wrap; gap: 7px; flex: none; padding: 0 16px 13px; }
.viewer-tags span { padding: 4px 10px; border-radius: var(--r-pill); background: var(--tint); color: var(--brand-300); font-size: var(--fs-xs); }
</style>
