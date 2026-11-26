<script setup>
/* 回收站：列出被移到回收目录的文件，可一键取回或永久删除。
 *
 * 删除时并没有真的删照片记录，只是把路径改到回收目录并标记 missing，
 * 所以取回时标签、日期、地点等元数据全都还在，不用重新扫描。 */
import { ref, computed, onMounted } from 'vue'
import Icon from '../components/Icon.vue'
import ConfirmDialog from '../components/ConfirmDialog.vue'
import { api, native } from '../api'
import { thumbUrl, photoUrl } from '../thumb'

const items = ref([])
const total = ref(0)
const bytes = ref(0)
const loading = ref(true)
const busy = ref(false)
const msg = ref('')
const msgType = ref('info')
const picked = ref(new Set())
/* 搜索：文件名 / 原位置 / 回收位置 */
const search = ref('')
const failedThumb = ref(new Set())

/* 只显示命中关键词的条目；多个词按 AND 处理 */
const visible = computed(() => {
  const words = search.value.trim().toLowerCase().split(/\s+/).filter(Boolean)
  if (!words.length) return items.value
  return items.value.filter(it => {
    const hay = `${it.name} ${it.src} ${it.dst}`.toLowerCase()
    return words.every(w => hay.includes(w))
  })
})

/* 缩略图地址：照片按回收目录里的实际路径取，视频用库里预生成的封面 */
function itemThumb(it) {
  if (it.thumb) return thumbUrl({ thumb: it.thumb }, 160)
  if (it.kind === 'video') return ''
  return thumbUrl({ path: it.dst, id: null }, 160)
}

function onThumbError(id) {
  failedThumb.value = new Set([...failedThumb.value, id])
}

/* 原位置的上级目录，列表里显示得短一些 */
function srcFolder(src) {
  if (!src) return ''
  return src.replace(/[\\/][^\\/]+$/, '')
}

/* ---------------- 放大预览 ---------------- */
/* 只有图片能直接预览；视频不带解码器，交给系统播放器 */
const preview = ref(null)   // { item, src, zoom }
const previewFailed = ref(false)

function canPreview(it) {
  // 注意：这里是 JS 函数不是模板，ref 必须手动 .value
  return it.kind !== 'video' && !failedThumb.value.has(it.log_id)
}

function openPreview(it) {
  previewFailed.value = false
  preview.value = {
    item: it,
    // 用原图地址而不是缩略图，放大后才看得清
    src: photoUrl({ path: it.dst }),
    zoom: 1,
  }
}

function closePreview() {
  preview.value = null
  previewFailed.value = false
}

function previewZoom(delta) {
  if (!preview.value) return
  const next = Math.min(6, Math.max(1, preview.value.zoom * delta))
  preview.value.zoom = Math.round(next * 100) / 100
}

/* 预览里直接取回 / 打开所在位置，省得关掉再找 */
async function previewOpenFolder() {
  const it = preview.value?.item
  if (it) await native.reveal(it.dst)
}

async function load() {
  loading.value = true
  const r = await api.recycle()
  loading.value = false
  if (r && r.items) {
    items.value = r.items
    total.value = r.total ?? r.items.length
    bytes.value = r.bytes ?? 0
    // 列表刷新后清掉已不存在的选中项
    const alive = new Set(r.items.map(x => x.log_id))
    picked.value = new Set([...picked.value].filter(id => alive.has(id)))
  } else {
    items.value = []
    total.value = 0
    bytes.value = 0
  }
}

onMounted(load)

function fmtSize(n) {
  if (!n) return '0 B'
  const u = ['B', 'KB', 'MB', 'GB']
  let i = 0
  let v = Number(n)
  while (v >= 1024 && i < 3) { v /= 1024; i++ }
  return (i === 0 ? v.toFixed(0) : v.toFixed(1)) + ' ' + u[i]
}

function fmtTime(ts) {
  if (!ts) return ''
  const d = new Date(ts * 1000)
  const p = n => String(n).padStart(2, '0')
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`
}

function togglePick(id) {
  const next = new Set(picked.value)
  if (next.has(id)) next.delete(id)
  else next.add(id)
  picked.value = next
}

function isPicked(id) {
  return picked.value.has(id)
}

function pickAll() {
  // 全选只作用于当前筛选结果，避免搜了关键词却把整个回收站都选上
  picked.value = new Set(visible.value.map(x => x.log_id))
}

function clearPick() {
  picked.value = new Set()
}

const pickedCount = computed(() => picked.value.size)
const pickedBytes = computed(() =>
  items.value.filter(x => picked.value.has(x.log_id)).reduce((s, x) => s + (x.size || 0), 0)
)

function setMsg(text, type = 'info') {
  msg.value = text
  msgType.value = type
}

/* ---------------- 确认弹窗 ---------------- */
const cfm = ref({
  show: false, title: '', message: '', detail: '', warn: '',
  confirmText: '', level: 'normal', all: false, purge: false,
})

function askRestore(all) {
  const ids = all ? [] : [...picked.value]
  if (!all && !ids.length) return
  const n = all ? total.value : ids.length
  const bytes = all ? bytes.value
    : items.value.filter(x => picked.value.has(x.log_id)).reduce((s, x) => s + (x.size || 0), 0)
  cfm.value = {
    show: true,
    title: all ? '取回全部文件？' : `取回选中的 ${n} 个文件？`,
    message: '文件会移回它原来的位置，照片的标签、日期、地点都会保留，不需要重新扫描。',
    detail: `${n} 个文件 · ${fmtSize(bytes)}`,
    warn: '',
    confirmText: '取回',
    level: 'normal',
    all,
    purge: false,
  }
}

function askPurge(all) {
  const ids = all ? [] : [...picked.value]
  if (!all && !ids.length) return
  const n = all ? total.value : ids.length
  const bytes = all ? bytes.value
    : items.value.filter(x => picked.value.has(x.log_id)).reduce((s, x) => s + (x.size || 0), 0)
  cfm.value = {
    show: true,
    title: all ? `永久删除全部 ${n} 个文件？` : `永久删除选中的 ${n} 个文件？`,
    message: '这些文件会从磁盘上被彻底删除。',
    detail: `${n} 个文件 · ${fmtSize(bytes)}`,
    warn: '此操作无法撤销，删除后无法恢复。如果只是想清理空间，建议先用「取回」把文件放回原位再决定。',
    confirmText: '永久删除',
    level: 'danger',
    all,
    purge: true,
  }
}

async function runConfirm() {
  const c = cfm.value
  if (busy.value) return
  busy.value = true
  const label = c.purge ? '永久删除' : '取回'
  try {
    const r = c.purge
      ? await api.purgeRecycle(c.all ? [] : [...picked.value], c.all)
      : await api.restoreRecycle(c.all ? [] : [...picked.value], c.all)
    cfm.value = { ...cfm.value, show: false }
    if (r && r.ok) {
      if (r.done === 0 && !r.errors) {
        // 一个都没动：多半是列表已经过期（文件被外部删了）
        setMsg('没有可取回的文件，回收站可能已被外部清理', 'warn')
        await load()
        return
      }
      setMsg(
        `已${label} ${r.done} 个文件` +
          (r.errors ? `，失败 ${r.errors} 个${r.message ? '：' + r.message : ''}` : ''),
        r.errors ? 'warn' : 'ok',
      )
      clearPick()
      await load()
      if (!c.purge) window.dispatchEvent(new CustomEvent('opo:index-reset'))
    } else {
      setMsg(`${label}失败：` + (r?.error || '未知错误'), 'err')
    }
  } catch (e) {
    setMsg(`${label}失败：` + (e?.message || e), 'err')
  } finally {
    // 必须保证复位：否则一旦抛异常，总线会卡在 busy，之后所有按钮都点不动
    busy.value = false
  }
}

async function openRecycleDir() {
  const first = items.value[0]
  if (first?.dst) {
    const dir = first.dst.replace(/[\\/][^\\/]+$/, '')
    await native.openPath(dir)
  }
}
</script>

<template>
  <div class="page">
    <div class="page-head card">
      <div class="page-head-main">
        <div class="page-head-ico grad-amber"><Icon name="trash" :size="20" /></div>
        <div class="page-head-text">
          <strong>回收站 · {{ total }} 个文件</strong>
          <span v-if="total">共 {{ fmtSize(bytes) }}，取回后照片的标签、日期、地点都还在</span>
          <span v-else>删除照片时会先移到这里，可随时取回</span>
        </div>
      </div>
      <div class="page-head-acts">
        <button class="btn btn-ghost" :disabled="busy" @click="load">
          <Icon name="sortDown" :size="15" /> 刷新
        </button>
        <button v-if="total" class="btn btn-ghost" :disabled="busy" @click="openRecycleDir">
          <Icon name="folder" :size="15" /> 打开目录
        </button>
      </div>
    </div>

    <div v-if="msg" class="msgbox" :class="msgType">{{ msg }}</div>

    <div v-if="loading" class="loading-state">
      <span class="ls-spin"></span>正在读取回收站…
    </div>

    <div v-else-if="!items.length" class="empty-state fill">
      <div class="es-ico"><Icon name="trash" :size="26" /></div>
      <h3 class="es-title">回收站是空的</h3>
      <p class="es-desc">删除照片或清理重复副本时，文件会先移到这里，不会直接销毁</p>
    </div>

    <template v-else>
      <!-- 搜索与操作条 -->
      <div class="bar card">
        <label class="search">
          <Icon name="search" :size="15" />
          <input v-model="search" placeholder="搜索文件名或原位置" />
          <button v-if="search" class="search-clear" @click="search = ''">
            <Icon name="close" :size="11" />
          </button>
        </label>
        <span class="count">
          已选 <b>{{ pickedCount }}</b> 个<template v-if="pickedBytes"> · {{ fmtSize(pickedBytes) }}</template>
          <template v-if="search"> · 匹配 {{ visible.length }}/{{ total }}</template>
        </span>
        <button class="btn btn-ghost" @click="pickAll">全选{{ search ? '匹配项' : '' }}</button>
        <button class="btn btn-ghost" :disabled="!pickedCount" @click="clearPick">取消选择</button>
        <span class="spacer"></span>
        <button class="btn btn-primary" :disabled="busy || !pickedCount" @click="askRestore(false)">
          <Icon name="check" :size="15" /> 取回选中
        </button>
        <button class="btn btn-ghost" :disabled="busy" @click="askRestore(true)">
          全部取回
        </button>
        <button class="btn btn-danger" :disabled="busy || !pickedCount" @click="askPurge(false)">
          <Icon name="trash" :size="15" /> 永久删除
        </button>
      </div>

      <div v-if="search && !visible.length" class="empty-state fill">
        <div class="es-ico"><Icon name="search" :size="26" /></div>
        <h3 class="es-title">没有匹配的文件</h3>
        <p class="es-desc">试着换个关键词，或清空搜索看全部 {{ total }} 个文件</p>
      </div>

      <!-- 文件列表 -->
      <div v-else class="list card">
        <label
          v-for="it in visible" :key="it.log_id"
          class="row" :class="{ on: isPicked(it.log_id) }"
        >
          <input
            type="checkbox" class="tick"
            :checked="isPicked(it.log_id)"
            @change="togglePick(it.log_id)"
          />
          <!-- 缩略图：能直观认出是哪张，比只看文件名快得多。
               右上角小按钮可放大查看，避免为了确认而先把文件取回。 -->
          <span class="thumb" :class="{ vid: it.kind === 'video' }">
            <img
              v-if="!failedThumb.has(it.log_id)"
              :src="itemThumb(it)" :alt="it.name"
              loading="lazy" @error="onThumbError(it.log_id)"
            />
            <Icon v-else :name="it.kind === 'video' ? 'video' : 'image'" :size="18" />
            <button
              v-if="canPreview(it)" class="thumb-zoom"
              title="放大查看" @click.stop.prevent="openPreview(it)"
            >
              <Icon name="search" :size="12" />
            </button>
          </span>
          <span class="info">
            <span class="name ellipsis" :title="it.name">
              {{ it.name }}
              <span v-if="it.kind === 'video'" class="vtag">视频</span>
            </span>
            <span class="origin ellipsis" :title="it.src">
              原位置：{{ srcFolder(it.src) || '未知' }}
            </span>
          </span>
          <span class="meta">{{ fmtTime(it.ts) }}</span>
          <span class="size">{{ fmtSize(it.size) }}</span>
        </label>
      </div>
    </template>

    <!-- 放大预览：确认这张是不是要取回的那张 -->
    <div
      v-if="preview" class="rc-mask" @click.self="closePreview"
      tabindex="0" @keydown.esc="closePreview"
    >
      <div class="rc-preview">
        <div class="rc-bar">
          <strong class="ellipsis">{{ preview.item.name }}</strong>
          <span class="rc-pager">
            {{ preview.item.kind === 'video' ? '视频' : '图片' }}
            · {{ fmtSize(preview.item.size) }}
          </span>
          <button class="rc-zoom" title="缩小" @click="previewZoom(0.8)">−</button>
          <button class="rc-zoom" title="复位 100%" @click="preview.zoom = 1">
            {{ Math.round(preview.zoom * 100) }}%
          </button>
          <button class="rc-zoom" title="放大" @click="previewZoom(1.25)">＋</button>
          <button class="rc-close" title="关闭（Esc）" @click="closePreview">
            <Icon name="close" :size="15" />
          </button>
        </div>

        <div
          class="rc-body"
          @wheel.prevent="previewZoom($event.deltaY < 0 ? 1.12 : 0.89)"
        >
          <img
            v-if="!previewFailed"
            :src="preview.src" :alt="preview.item.name"
            :style="{ transform: `scale(${preview.zoom})` }"
            @error="previewFailed = true"
          />
          <div v-else class="rc-fail">
            <Icon name="image" :size="30" />
            <span>这张图无法预览，可能原始文件已损坏</span>
          </div>
        </div>

        <div class="rc-foot">
          <span class="rc-origin" :title="preview.item.src">
            原位置：{{ preview.item.src }}
          </span>
          <button class="btn btn-ghost" @click="previewOpenFolder">
            <Icon name="folder" :size="14" /> 在资源管理器打开
          </button>
        </div>
      </div>
    </div>

    <ConfirmDialog
      :show="cfm.show"
      :title="cfm.title"
      :message="cfm.message"
      :detail="cfm.detail"
      :warn="cfm.warn"
      :confirm-text="cfm.confirmText"
      :level="cfm.level"
      :busy="busy"
      @confirm="runConfirm"
      @cancel="cfm = { ...cfm, show: false }"
    />
  </div>
</template>

<style scoped>
/* 根容器/页头/空态/加载态已统一到 base.css */
.msgbox {
  flex: none; padding: 10px 14px; border-radius: var(--r-sm);
  background: var(--surface-2); font-size: var(--fs-sm); color: var(--text-secondary);
}
.msgbox.ok { background: rgba(34,197,94,.12); color: var(--success); }
.msgbox.err { background: rgba(239,68,68,.12); color: var(--danger); }
.msgbox.warn { background: rgba(245,158,11,.12); color: var(--warning); }

.bar {
  flex: none; display: flex; flex-wrap: wrap; align-items: center; gap: 10px;
  padding: 12px 16px;
}
.bar .count { font-size: var(--fs-sm); color: var(--text-secondary); }
.bar .count b { color: var(--brand-400); font-size: var(--fs-md); }
.bar .spacer { flex: 1; min-width: 8px; }
.bar .btn { height: 34px; padding: 0 14px; font-size: var(--fs-sm); }
.btn-danger { background: rgba(239,68,68,.14); color: var(--danger); }
.btn-danger:hover { background: rgba(239,68,68,.24); }

/* 搜索框：和页头搜索同款胶囊 */
.bar .search {
  display: flex; align-items: center; gap: 8px;
  flex: 1; min-width: 180px; max-width: 320px;
  height: 34px; padding: 0 12px; border-radius: var(--r-md);
  background: var(--surface-2); color: var(--text-tertiary);
}
.bar .search input {
  flex: 1; min-width: 0; background: transparent; border: 0;
  font-size: var(--fs-sm); color: var(--text-primary);
}
.search-clear {
  display: grid; place-items: center; width: 16px; height: 16px; flex: none;
  border-radius: 50%; background: var(--surface-3); color: var(--text-tertiary);
}

.list { flex: none; overflow: hidden; }
.row {
  display: flex; align-items: center; gap: 12px;
  padding: 10px 16px; cursor: pointer;
  border-bottom: 1px solid var(--line);
  transition: background var(--dur) var(--ease);
}
.row:last-child { border-bottom: 0; }
.row:hover { background: var(--surface-hover); }
.row.on { background: var(--tint); }
.tick { width: 14px; height: 14px; flex: none; accent-color: var(--brand-500); cursor: pointer; }

/* 缩略图：一眼认出是哪张照片，比只看文件名快得多 */
.thumb {
  position: relative;
  display: grid; place-items: center;
  width: 54px; height: 54px; flex: none;
  border-radius: var(--r-xs); overflow: hidden;
  background: var(--surface-3); color: var(--text-tertiary);
}
.thumb.vid { background: rgba(139,92,246,.14); color: var(--brand-300); }
.thumb img { width: 100%; height: 100%; object-fit: cover; display: block; }

/* 放大按钮：平时淡显，悬停该行才完全亮起，避免列表太花 */
.thumb-zoom {
  position: absolute; right: 2px; top: 2px;
  width: 18px; height: 18px;
  display: grid; place-items: center;
  border-radius: 4px;
  background: rgba(0,0,0,.62); color: #fff;
  opacity: 0; transition: opacity .12s ease, background .12s ease;
}
.row:hover .thumb-zoom { opacity: .9; }
.thumb-zoom:hover { opacity: 1; background: var(--brand-500); }

/* ---------- 放大预览 ---------- */
.rc-mask {
  position: fixed; inset: 0; z-index: 220;
  display: grid; place-items: center;
  padding: 12px;
  background: rgba(6,8,12,.88); backdrop-filter: blur(8px);
}
.rc-preview {
  display: flex; flex-direction: column;
  width: min(1500px, 96vw); height: min(960px, 95vh);
  border-radius: var(--r-xl); overflow: hidden;
  background: var(--surface); box-shadow: var(--shadow-lg);
}
.rc-bar {
  display: flex; align-items: center; gap: 12px;
  flex: none; padding: 10px 14px;
  border-bottom: 1px solid var(--line);
}
.rc-bar strong { flex: 1; min-width: 0; font-size: var(--fs-sm); }
.rc-pager { flex: none; font-size: var(--fs-xs); color: var(--text-tertiary); }
.rc-zoom {
  flex: none; min-width: 30px; height: 28px; padding: 0 8px;
  border-radius: var(--r-xs); background: var(--surface-2);
  color: var(--text-secondary); font-size: var(--fs-xs);
  font-variant-numeric: tabular-nums;
}
.rc-zoom:hover { background: var(--surface-3); color: var(--text-primary); }
.rc-close {
  flex: none; width: 30px; height: 28px;
  display: grid; place-items: center;
  border-radius: var(--r-xs); color: var(--text-tertiary);
}
.rc-close:hover { background: rgba(239,68,68,.16); color: var(--danger); }

.rc-body {
  flex: 1; min-height: 0; position: relative;
  display: grid; place-items: center;
  background: #07090d; overflow: hidden;
}
.rc-body img {
  max-width: 100%; max-height: 100%;
  width: auto; height: auto;
  object-fit: contain; display: block;
  transition: transform .1s linear;
  user-select: none; -webkit-user-drag: none;
}
.rc-fail {
  display: flex; flex-direction: column; align-items: center; gap: 10px;
  color: var(--text-tertiary); font-size: var(--fs-sm);
}

.rc-foot {
  flex: none; display: flex; align-items: center; gap: 12px;
  padding: 10px 14px; border-top: 1px solid var(--line);
}
.rc-origin {
  flex: 1; min-width: 0; font-size: var(--fs-xs); color: var(--text-tertiary);
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
}

.info { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 3px; }
.name {
  font-size: var(--fs-sm); color: var(--text-primary);
  display: flex; align-items: center; gap: 7px; min-width: 0;
}
.vtag {
  flex: none; padding: 1px 6px; border-radius: var(--r-pill);
  background: rgba(139,92,246,.18); color: var(--brand-300); font-size: 10px;
}
/* 原位置：告诉用户这张原本在哪，取回前能确认 */
.origin { font-size: var(--fs-xs); color: var(--text-tertiary); }

.meta { flex: none; font-size: var(--fs-xs); color: var(--text-tertiary); }
.size { flex: none; width: 76px; text-align: right; font-size: var(--fs-xs); color: var(--text-tertiary); }
</style>
