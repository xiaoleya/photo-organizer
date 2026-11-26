<script setup>
/* PhotosPanel：统一的照片墙。
   - 卡片高度固定，图片用 contain 完整显示（不裁切）
   - 不嵌套滚动容器，跟随页面滚动
   - 滚动到底部自动继续加载，直到全部照片都出来
   - 点击打开大图，右键/双击调用系统看片器 */
import { ref, watch, onMounted, onBeforeUnmount, computed, nextTick } from 'vue'
import Icon from './Icon.vue'
import ConfirmDialog from './ConfirmDialog.vue'
import { api, native } from '../api'
import { thumbUrl } from '../thumb'
import { openViewer } from '../viewer'

const props = defineProps({
  query: { type: Object, default: () => ({}) },
  pageSize: { type: Number, default: 200 },
  thumbHeight: { type: Number, default: 168 },
  emptyTitle: { type: String, default: '没有符合条件的照片' },
  emptyHint: { type: String, default: '换个筛选条件，或先添加照片目录' },
  showHint: { type: Boolean, default: true },
  showRepair: { type: Boolean, default: false },
  /* 是否启用多选。默认关闭，只有照片库打开，避免影响其他 6 处调用 */
  selectable: { type: Boolean, default: false },
})

const emit = defineEmits(['count', 'repair', 'favorite', 'selection-change'])

const photos = ref([])
const total = ref(0)
const loading = ref(false)
const failed = ref({})
const sentinel = ref(null)
const autoLoad = ref(true)

let seq = 0
let observer = null

/* 视频时长角标：90 -> 1:30，3700 -> 1:01:40 */
function fmtDur(sec) {
  const t = Math.round(Number(sec) || 0)
  if (t <= 0) return ''
  const h = Math.floor(t / 3600)
  const m = Math.floor((t % 3600) / 60)
  const s = t % 60
  const pad = n => String(n).padStart(2, '0')
  return h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${m}:${pad(s)}`
}

/* ---------------- 右键菜单 ---------------- */
const menu = ref({ show: false, x: 0, y: 0, row: null })

function showMenu(e, row) {
  // 靠近右/下边缘时往回收，避免菜单被裁掉
  const W = 200, H = 190
  const x = Math.min(e.clientX, window.innerWidth - W - 8)
  const y = Math.min(e.clientY, window.innerHeight - H - 8)
  menu.value = { show: true, x: Math.max(4, x), y: Math.max(4, y), row }
}

function closeMenu() {
  menu.value = { ...menu.value, show: false }
}

async function menuReveal() {
  const row = menu.value.row
  closeMenu()
  if (row) await native.reveal(row.path)
}

async function menuOpenDefault() {
  const row = menu.value.row
  closeMenu()
  if (row) await native.openPath(row.path)
}

async function menuFav() {
  const row = menu.value.row
  closeMenu()
  if (row) toggleFav(row)
}

/* 右键删除单张：走同一个确认弹窗 */
function menuDelete() {
  const row = menu.value.row
  closeMenu()
  if (!row) return
  confirmState.value = {
    show: true,
    title: '删除这个文件？',
    message: '文件会移到「照片整理」自己的回收站（不是 Windows 回收站），可以随时取回。',
    detail: `${row.name || ''} · ${fmtSize(row.size || 0)}`,
    warn: '',
    ids: [row.id],
    permanent: false,
    primary: false,
  }
}

async function menuCopyPath() {
  const row = menu.value.row
  closeMenu()
  if (!row) return
  try {
    await navigator.clipboard.writeText(row.path)
  } catch {
    // 剪贴板不可用时退回手动提示
    window.prompt('复制下面的路径：', row.path)
  }
}

// 滚动或改变窗口大小时关掉菜单，避免它跟内容错位
function onScrollClose() { if (menu.value.show) closeMenu() }
onMounted(() => {
  window.addEventListener('scroll', onScrollClose, true)
  window.addEventListener('resize', onScrollClose)
})
onBeforeUnmount(() => {
  window.removeEventListener('scroll', onScrollClose, true)
  window.removeEventListener('resize', onScrollClose)
})

async function load(reset = true) {
  const my = ++seq
  loading.value = true
  if (reset) { photos.value = []; total.value = 0 }
  const q = { ...props.query, limit: props.pageSize, offset: reset ? 0 : photos.value.length }
  Object.keys(q).forEach(k => {
    if (q[k] === '' || q[k] === null || q[k] === undefined) delete q[k]
  })
  const r = await api.photos(q)
  if (my !== seq) return
  loading.value = false
  if (r && r.items) {
    photos.value = reset ? r.items : photos.value.concat(r.items)
    total.value = r.total ?? photos.value.length
    emit('count', total.value)
    await nextTick()
    // 如果一页填不满屏幕，继续拉，直到填满或取完
    if (autoLoad.value && photos.value.length < total.value) maybeLoadMore()
  }
}

function maybeLoadMore() {
  if (loading.value) return
  if (photos.value.length >= total.value) return
  load(false)
}

function setupObserver() {
  if (typeof IntersectionObserver === 'undefined' || !sentinel.value) return
  if (observer) observer.disconnect()
  observer = new IntersectionObserver(entries => {
    if (entries.some(e => e.isIntersecting) && autoLoad.value) maybeLoadMore()
  }, { rootMargin: '300px' })
  observer.observe(sentinel.value)
}

function onImgError(id) {
  failed.value = { ...failed.value, [id]: true }
}

/* ---------------- 多选 ---------------- */
/* 用 Set 存选中 id：切页/加载更多时不会丢，也不需要和 photos 数组同步 */
const picked = ref(new Set())
const selecting = computed(() => picked.value.size > 0)
/* Shift 范围选择的锚点（已加载列表中的索引） */
let shiftAnchor = null

function togglePick(id) {
  const next = new Set(picked.value)
  if (next.has(id)) next.delete(id)
  else next.add(id)
  picked.value = next
  shiftAnchor = photos.value.findIndex(p => p.id === id)
  emit('selection-change', [...next])
}

function isPicked(id) {
  return picked.value.has(id)
}

function clearPick() {
  picked.value = new Set()
  shiftAnchor = null
  emit('selection-change', [])
}

/* 全选当前已加载的照片（不是全库，避免误删） */
function pickAllLoaded() {
  picked.value = new Set(photos.value.map(p => p.id))
  emit('selection-change', [...picked.value])
}

function pickInvert() {
  const next = new Set()
  for (const p of photos.value) {
    if (!picked.value.has(p.id)) next.add(p.id)
  }
  picked.value = next
  emit('selection-change', [...next])
}

/* 已选照片的完整对象，供父组件取用（含 path/name 等） */
const pickedRows = computed(() =>
  photos.value.filter(p => picked.value.has(p.id))
)
const pickedBytes = computed(() =>
  pickedRows.value.reduce((s, p) => s + (p.size || 0), 0)
)

/* 点击卡片：已进入多选时切换勾选，否则打开大图 */
function onCardClick(p, e) {
  // Shift 连选：即使还没选过任何一张，也应该进入多选并把这张当起点，
  // 而不是打开大图（那会让用户以为 Shift 没生效）
  if (props.selectable && e && e.shiftKey) {
    if (shiftAnchor == null) {
      togglePick(p.id)
    } else {
      shiftSelectTo(p)
    }
    return
  }
  if (props.selectable && selecting.value) {
    togglePick(p.id)
    return
  }
  if (props.selectable && e && (e.ctrlKey || e.metaKey)) {
    togglePick(p.id)
    return
  }
  openViewer(p, photos.value)
}

/* Shift 范围选择：锚点到当前照片之间（按当前已加载顺序） */
function shiftSelectTo(p) {
  const idx = photos.value.findIndex(x => x.id === p.id)
  if (idx < 0 || shiftAnchor == null) return
  const from = Math.min(idx, shiftAnchor)
  const to = Math.max(idx, shiftAnchor)
  const next = new Set(picked.value)
  for (let i = from; i <= to; i++) next.add(photos.value[i].id)
  picked.value = next
  emit('selection-change', [...next])
}

/* ---------------- 批量操作实现 ---------------- */
const bulkBusy = ref(false)

/* 确认弹窗的共享状态。所有删除都要经过它。 */
const confirmState = ref({
  show: false, title: '', message: '', detail: '', warn: '',
  ids: [], permanent: false, primary: false,
})

function askDelete(ids, single = false) {
  const rows = photos.value.filter(p => ids.includes(p.id))
  const bytes = rows.reduce((s, p) => s + (p.size || 0), 0)
  confirmState.value = {
    show: true,
    title: single ? '删除这个文件？' : `删除选中的 ${ids.length} 个文件？`,
    message: '文件会移到「照片整理」自己的回收站（不是 Windows 回收站），可以随时取回。',
    detail: `${ids.length} 个文件 · ${fmtSize(bytes)}`,
    warn: '',
    ids,
    permanent: false,
    primary: false,
  }
}

async function doDelete() {
  const st = confirmState.value
  if (!st.ids.length) return
  bulkBusy.value = true
  const r = await api.deletePhotos(st.ids)
  bulkBusy.value = false
  confirmState.value = { ...st, show: false }
  if (r && r.ok) {
    clearPick()
    await load(true)
    window.dispatchEvent(new CustomEvent('opo:index-reset'))
  } else {
    alert('删除失败：' + (r?.error || '未知错误'))
  }
}

function fmtSize(n) {
  if (!n) return '0 B'
  const u = ['B', 'KB', 'MB', 'GB']
  let i = 0
  let v = Number(n)
  while (v >= 1024 && i < 3) { v /= 1024; i++ }
  return (i === 0 ? v.toFixed(0) : v.toFixed(1)) + ' ' + u[i]
}

async function bulkFav(want) {
  if (bulkBusy.value) return
  const ids = [...picked.value]
  if (!ids.length) return
  bulkBusy.value = true
  const r = await api.favoriteMany(ids, want)
  bulkBusy.value = false
  if (r && r.ok) {
    // 就地更新，避免整页重载丢滚动位置
    const set = new Set(ids)
    photos.value = photos.value.map(p =>
      set.has(p.id) ? { ...p, favorite: want ? 1 : 0 } : p
    )
    window.dispatchEvent(new CustomEvent('opo:favorite-changed', {
      detail: { favorite: want, ids },
    }))
  }
}

async function bulkDelete() {
  if (bulkBusy.value || !picked.value.size) return
  askDelete([...picked.value])
}

async function bulkExport() {
  if (bulkBusy.value || !picked.value.size) return
  const dest = await native.pickFolder()
  if (!dest) return
  bulkBusy.value = true
  const r = await api.exportPhotos([...picked.value], dest, 'copy')
  bulkBusy.value = false
  if (r && r.ok) {
    alert(`已导出 ${r.done} 张到：${r.dest}` + (r.errors ? `\n失败 ${r.errors} 张` : ''))
    clearPick()
  } else {
    alert('导出失败：' + (r?.error || '未知错误'))
  }
}

function bulkRepair() {
  // 交给父组件跳转到修复页并带上选中项
  emit('repair', pickedRows.value)
}

/* 收藏 / 取消收藏：先就地更新，再落库，失败时回滚 */
async function toggleFav(p) {
  const next = !p.favorite
  const idx = photos.value.findIndex(x => x.id === p.id)
  if (idx >= 0) photos.value[idx] = { ...photos.value[idx], favorite: next ? 1 : 0 }
  const r = await api.favorite(p.id, next)
  if (!r || !r.ok) {
    if (idx >= 0) photos.value[idx] = { ...photos.value[idx], favorite: next ? 0 : 1 }
    return
  }
  emit('favorite', { id: p.id, favorite: next })
  // 广播给收藏页等其它视图，保持数量同步
  window.dispatchEvent(new CustomEvent('opo:favorite-changed',
    { detail: { id: p.id, favorite: next } }))
}

watch(() => JSON.stringify(props.query || {}), () => load(true))

onMounted(async () => {
  await load(true)
  setupObserver()
})

onBeforeUnmount(() => {
  if (observer) observer.disconnect()
  observer = null
})

defineExpose({
  reload: () => load(true),
  loadAll: () => { autoLoad.value = true; maybeLoadMore() },
  photos, total,
  clearPick, pickedRows,
})

const hasMore = computed(() => photos.value.length < total.value)
</script>

<template>
  <div class="panel">
    <div v-if="showHint" class="panel-head">
      <span>共 {{ total }} 个文件<template v-if="hasMore">（已显示 {{ photos.length }}）</template></span>
    </div>

    <!-- 空态与加载态统一用 base.css 的 .empty-state / .loading-state -->
    <div v-if="!photos.length && !loading" class="empty-state">
      <div class="es-ico"><Icon name="photos" :size="26" /></div>
      <h3 class="es-title">{{ emptyTitle }}</h3>
      <p class="es-desc">{{ emptyHint }}</p>
    </div>
    <div v-else-if="!photos.length && loading" class="loading-state">
      <span class="ls-spin"></span>正在加载照片…
    </div>

    <div v-else class="panel-grid">
      <article
        v-for="p in photos"
        :key="p.id"
        class="pcard"
        :class="{ picked: selectable && isPicked(p.id) }"
        @click="onCardClick(p, $event)"
        @contextmenu.prevent="showMenu($event, p)"
      >
        <!-- 多选勾选框：自绘，不用原生 checkbox（原生在各浏览器/缩放
             下外观不一致，未选中时只是个空白方块，和"已选"对比不明显） -->
        <button
          v-if="selectable"
          class="pick" :class="{ on: isPicked(p.id) }"
          :title="isPicked(p.id) ? '取消选择' : '选择这张'"
          @click.stop.prevent="togglePick(p.id)"
        >
          <Icon v-if="isPicked(p.id)" name="check" :size="13" />
        </button>
        <div class="pthumb" :style="{ height: thumbHeight + 'px' }">
          <img
            v-if="!failed[p.id]"
            :src="thumbUrl(p, 640)"
            :alt="p.name"
            loading="lazy"
            decoding="async"
            @error="onImgError(p.id)"
          />
          <div v-else class="pthumb-fail"><Icon name="image" :size="20" /></div>

          <!-- 视频：时长角标 + 播放图标 -->
          <template v-if="p.kind === 'video'">
            <span class="pvideo-badge">
              <Icon name="play" :size="11" />
              <template v-if="p.duration">{{ fmtDur(p.duration) }}</template>
            </span>
          </template>
        </div>
        <div class="pmeta">
          <span class="pname" :title="p.name">{{ p.name }}</span>
          <span class="psub">
            {{ p.dt ? p.dt.slice(0, 10) : '无日期' }}<template v-if="p.place && p.place !== '未知地点'"> · {{ p.place }}</template>
          </span>
        </div>

        <!-- 底部操作条：收藏（星标）在左，去修复在右，都常驻显示 -->
        <div class="pactions">
          <button
            class="pact fav" :class="{ on: p.favorite }"
            :title="p.favorite ? '取消收藏' : '加入收藏'"
            @click.stop="toggleFav(p)"
          >
            <Icon :name="p.favorite ? 'starFilled' : 'star'" :size="14" />
          </button>
          <button
            v-if="showRepair" class="pact repair" title="去修复这张照片"
            @click.stop="emit('repair', p)"
          >
            <Icon name="wrench" :size="14" />
            <span>修复</span>
          </button>
        </div>
      </article>
    </div>

    <div ref="sentinel" class="panel-foot">
      <button v-if="hasMore" class="more" :disabled="loading" @click="maybeLoadMore">
        {{ loading ? '加载中…' : `加载更多（剩余 ${total - photos.length} 张）` }}
      </button>
      <span v-else-if="photos.length" class="done">已显示全部 {{ photos.length }} 张</span>
      <span v-else-if="loading" class="done">加载中…</span>
    </div>

    <!-- 批量操作条：有选中项时浮在底部 -->
    <teleport to="body">
      <div v-if="selectable && selecting" class="bulk-bar">
        <span class="bulk-count">
          已选 <b>{{ picked.size }}</b> 张<template v-if="pickedBytes"> · {{ fmtSize(pickedBytes) }}</template>
        </span>
        <span class="bulk-sep"></span>
        <button class="bulk-btn" @click="pickAllLoaded">全选已加载</button>
        <button class="bulk-btn" @click="pickInvert">反选</button>
        <button class="bulk-btn" @click="clearPick">取消选择</button>
        <span class="bulk-spacer"></span>
        <button class="bulk-btn" @click="bulkFav(true)">
          <Icon name="star" :size="14" /> 收藏
        </button>
        <button class="bulk-btn" @click="bulkFav(false)">
          <Icon name="starFilled" :size="14" /> 取消收藏
        </button>
        <button v-if="showRepair" class="bulk-btn" @click="bulkRepair">
          <Icon name="wrench" :size="14" /> 修复
        </button>
        <button class="bulk-btn" @click="bulkExport">
          <Icon name="folder" :size="14" /> 导出
        </button>
        <button class="bulk-btn danger" @click="bulkDelete">
          <Icon name="trash" :size="14" /> 删除
        </button>
      </div>
    </teleport>

    <!-- 右键菜单：fixed 定位，用视口坐标 -->
    <teleport to="body">
      <div
        v-if="menu.show"
        class="ctx-mask" @click="closeMenu" @contextmenu.prevent="closeMenu"
      ></div>
      <div
        v-if="menu.show" class="ctx-menu"
        :style="{ left: menu.x + 'px', top: menu.y + 'px' }"
        @click.stop
      >
        <button class="ctx-item" @click="menuReveal">
          <Icon name="folder" :size="14" /> 在资源管理器中打开
        </button>
        <button class="ctx-item" @click="menuOpenDefault">
          <Icon name="image" :size="14" /> 用默认程序打开
        </button>
        <div class="ctx-sep"></div>
        <button class="ctx-item" @click="menuFav">
          <Icon :name="menu.row?.favorite ? 'starFilled' : 'star'" :size="14" />
          {{ menu.row?.favorite ? '取消收藏' : '收藏' }}
        </button>
        <button class="ctx-item" @click="menuCopyPath">
          <Icon name="list" :size="14" /> 复制文件路径
        </button>
        <div class="ctx-sep"></div>
        <button class="ctx-item danger" @click="menuDelete">
          <Icon name="trash" :size="14" /> 删除（移到本软件回收站）
        </button>
      </div>
    </teleport>

    <!-- 删除确认弹窗 -->
    <ConfirmDialog
      :show="confirmState.show"
      :title="confirmState.title"
      :message="confirmState.message"
      :detail="confirmState.detail"
      :warn="confirmState.warn"
      confirm-text="移到回收站"      :busy="bulkBusy"
      level="danger"
      @confirm="doDelete"
      @cancel="confirmState = { ...confirmState, show: false }"
    />
  </div>
</template>

<style scoped>
/* ---------- 多选勾选框 ---------- */
/* 未选：空心圆环 + 半黑底，悬停才显出
   已选：实心品牌蓝 + 白色对勾，且常驻显示 */
.pick {
  position: absolute; left: 9px; top: 9px; z-index: 4;
  width: 24px; height: 24px;
  display: grid; place-items: center;
  border-radius: 50%;
  background: rgba(0, 0, 0, .42);
  border: 1.5px solid rgba(255, 255, 255, .82);
  color: #fff;
  opacity: 0;
  transform: scale(.88);
  transition: opacity .14s ease, transform .14s ease,
              background .14s ease, border-color .14s ease;
  backdrop-filter: blur(3px);
}
.pcard:hover .pick { opacity: 1; transform: scale(1); }
.pick:hover { background: rgba(0, 0, 0, .62); transform: scale(1.06); }
.pick.on {
  opacity: 1; transform: scale(1);
  background: var(--brand-500);
  border-color: var(--brand-400);
  box-shadow: 0 2px 8px rgba(59, 149, 247, .5);
}
.pcard.picked {
  outline: 2px solid var(--brand-500);
  outline-offset: -2px;
}

/* ---------- 批量操作条 ---------- */
.bulk-bar {
  position: fixed; left: 50%; bottom: 22px; transform: translateX(-50%);
  z-index: 2500;
  display: flex; align-items: center; gap: 8px;
  padding: 9px 14px;
  border-radius: var(--r-lg);
  background: var(--surface);
  border: 1px solid var(--line-strong);
  box-shadow: var(--shadow-lg);
  max-width: calc(100vw - 64px);
  overflow-x: auto;
}
.bulk-count { font-size: var(--fs-sm); color: var(--text-secondary); white-space: nowrap; }
.bulk-count b { color: var(--brand-400); font-size: var(--fs-md); }
.bulk-sep { width: 1px; height: 18px; background: var(--line); flex: none; }
.bulk-spacer { flex: 1; min-width: 12px; }
.bulk-btn {
  display: inline-flex; align-items: center; gap: 5px;
  padding: 6px 11px; border-radius: var(--r-sm);
  background: var(--surface-2); color: var(--text-secondary);
  font-size: var(--fs-sm); white-space: nowrap;
  transition: background var(--dur) var(--ease), color var(--dur) var(--ease);
}
.bulk-btn:hover { background: var(--surface-3); color: var(--text-primary); }
.bulk-btn.danger { background: rgba(239,68,68,.14); color: var(--danger); }
.bulk-btn.danger:hover { background: rgba(239,68,68,.24); }

/* ---------- 右键菜单 ---------- */
.ctx-mask {
  position: fixed; inset: 0; z-index: 3000;
  background: transparent;
}
.ctx-menu {
  position: fixed; z-index: 3001; min-width: 188px;
  padding: 5px; border-radius: 10px;
  background: var(--surface);
  border: 1px solid var(--line);
  box-shadow: 0 10px 32px rgba(0,0,0,.42);
  display: flex; flex-direction: column;
}
.ctx-item {
  display: flex; align-items: center; gap: 9px;
  width: 100%; padding: 8px 10px; border-radius: 7px;
  font-size: 13px; color: var(--text-primary);
  text-align: left; white-space: nowrap;
}
.ctx-item:hover { background: var(--surface-hover); }
.ctx-item.danger { color: var(--danger); }
.ctx-item.danger:hover { background: rgba(239,68,68,.14); }
.ctx-item svg { flex: none; opacity: .8; }
.ctx-sep { height: 1px; margin: 4px 6px; background: var(--line); }

.panel { display: flex; flex-direction: column; }
.panel-head { font-size: var(--fs-sm); color: var(--text-tertiary); margin-bottom: 10px; }
.panel-grid {
  display: grid; gap: 14px; align-content: start;
  grid-auto-rows: min-content;
  grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
}
.pcard {
  position: relative; display: flex; flex-direction: column;
  border-radius: var(--r-lg); background: var(--surface);
  box-shadow: var(--surface-shadow);
  border: 1px solid var(--surface-stroke);
  cursor: pointer; overflow: hidden;
  transition: transform var(--dur) var(--ease),
              box-shadow var(--dur) var(--ease),
              border-color var(--dur) var(--ease);
}
.pcard:hover {
  transform: translateY(-3px);
  box-shadow: var(--surface-shadow-hover);
  border-color: var(--surface-stroke-hover);
}
.pthumb {
  width: 100%; flex: none; display: flex; align-items: center; justify-content: center;
  background: #0c0e12; overflow: hidden; position: relative;
}
/* 视频时长角标：左下角半透明黑底 */
.pvideo-badge {
  position: absolute; left: 6px; bottom: 6px;
  display: inline-flex; align-items: center; gap: 3px;
  padding: 2px 6px; border-radius: 5px;
  background: rgba(0,0,0,.68); color: #fff;
  font-size: 11px; line-height: 1.4;
  pointer-events: none;
}
/* 小图/长条图按原比例放大到能看清，但不裁切、不拉伸变形 */
.pthumb img {
  max-width: 100%; max-height: 100%; width: auto; height: auto;
  object-fit: contain; display: block; image-rendering: auto;
}
.pthumb-fail {
  display: grid; place-items: center; width: 100%; height: 100%;
  color: var(--text-tertiary); background: var(--surface-3);
}
.pmeta { padding: 9px 11px 4px; display: flex; flex-direction: column; gap: 2px; min-width: 0; }
.pname {
  font-size: var(--fs-base); font-weight: 600; color: var(--text-primary);
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
}
.psub {
  font-size: var(--fs-xs); color: var(--text-tertiary);
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
}

/* 常驻操作条：收藏在左、修复在右，不再浮在图片右上角遮挡内容 */
.pactions {
  display: flex; align-items: center; gap: 6px;
  padding: 0 9px 9px;
}
.pact {
  display: inline-flex; align-items: center; gap: 4px;
  height: 26px; padding: 0 9px; border-radius: var(--r-xs);
  background: var(--surface-2); color: var(--text-tertiary);
  font-size: var(--fs-xs); transition: all var(--dur) var(--ease);
}
.pact:hover { background: var(--surface-3); color: var(--text-primary); }
.pact.fav.on { background: rgba(245,158,11,.16); color: #f5a623; }
.pact.repair { margin-left: auto; }
.pact.repair:hover { background: var(--tint-strong); color: var(--brand-300); }
.panel-foot { display: flex; justify-content: center; padding: 16px 0 6px; }
.more {
  padding: 9px 22px; border-radius: var(--r-pill);
  background: var(--surface-2); color: var(--text-secondary); font-size: var(--fs-sm);
}
.more:hover { background: var(--surface-3); color: var(--text-primary); }
.more:disabled { opacity: .6; }
.done { font-size: var(--fs-xs); color: var(--text-tertiary); }
</style>
