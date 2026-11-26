<script setup>
/* 重复照片：每组显示全部照片，可逐张勾选要删除的副本（不是固定“非最佳”）。
   每组默认勾选除最佳外的副本，用户可以改选。删除时移到回收目录，可撤销。 */
import { ref, computed, onMounted } from 'vue'
import Icon from '../components/Icon.vue'
import ConfirmDialog from '../components/ConfirmDialog.vue'
import { api, native } from '../api'
import { thumbUrl } from '../thumb'
import { openViewer } from '../viewer'

const groups = ref([])
const loading = ref(true)
const busy = ref(false)
const msg = ref('')
const recycleDir = ref('')
/* key = photo id, value = true 表示要删除 */
const marks = ref({})
/* 每组手动指定过的保留项：gid -> photo id，切换策略时不覆盖 */
const manualKeep = ref({})
const strategy = ref('quality')

const STRATEGIES = [
  { id: 'quality', label: '质量最好', hint: '清晰度与整体质量分最高' },
  { id: 'resolution', label: '分辨率最高', hint: '像素尺寸最大' },
  { id: 'size', label: '文件最大', hint: '通常保留信息最多' },
  { id: 'earliest', label: '拍摄时间最早', hint: '适合保留原始拍摄那张' },
  { id: 'shortest', label: '路径最短', hint: '更接近原始存放位置' },
  { id: 'favorite', label: '收藏优先', hint: '已收藏的优先保留' },
]

const props = defineProps({
  search: { type: String, default: '' },
})

onMounted(load)

async function load() {
  loading.value = true
  const r = await api.duplicates()
  loading.value = false
  if (r && r.items) {
    groups.value = r.items
    marks.value = {}
    manualKeep.value = {}
    applyStrategy()
  }
}

/* 顶部搜索框过滤：组里有任意一张照片的名字/地点命中就保留整组 */
const visibleGroups = computed(() => {
  const kw = (props.search || '').trim().toLowerCase()
  if (!kw) return groups.value
  return groups.value.filter(g =>
    g.photos.some(p =>
      (p.name || '').toLowerCase().includes(kw) ||
      (p.place || '').toLowerCase().includes(kw) ||
      (p.path || '').toLowerCase().includes(kw)))
})

function toggle(id) {
  marks.value = { ...marks.value, [id]: !marks.value[id] }
}

/* 保留项打分：分数越高越该保留 */
function keepScore(p) {
  const num = v => (Number.isFinite(Number(v)) ? Number(v) : 0)
  const time = p.dt ? Date.parse(p.dt.replace(' ', 'T')) : Number.POSITIVE_INFINITY
  switch (strategy.value) {
    case 'resolution':
      return [num(p.megapixels), num(p.width) * num(p.height), num(p.size)]
    case 'size':
      return [num(p.size), num(p.megapixels)]
    case 'earliest':
      return [Number.isFinite(time) ? -time : Number.NEGATIVE_INFINITY, num(p.megapixels)]
    case 'shortest':
      return [-(p.path || '').length, num(p.megapixels)]
    case 'favorite':
      return [p.favorite ? 1 : 0, num(p.quality_score), num(p.megapixels), num(p.size)]
    default:
      return [num(p.quality_score), num(p.megapixels), num(p.size)]
  }
}

function better(a, b) {
  const ka = keepScore(a)
  const kb = keepScore(b)
  for (let i = 0; i < Math.max(ka.length, kb.length); i++) {
    const x = ka[i] ?? 0
    const y = kb[i] ?? 0
    if (x === y) continue
    return x > y ? a : b
  }
  // 完全打平时用 id 兜底，保证每次选择稳定可复现
  return a.id < b.id ? a : b
}

/* 某组当前应当保留的照片：手动指定优先于自动策略 */
function keeperOf(g) {
  const manualId = manualKeep.value[g.id]
  if (manualId) {
    const hit = g.photos.find(p => p.id === manualId)
    if (hit) return hit
  }
  return g.photos.reduce((best, p) => (best ? better(best, p) : p), null)
}

/* 按当前策略重新勾选非保留项，保留每张手动指定 */
function applyStrategy() {
  const m = {}
  for (const g of groups.value) {
    const keep = keeperOf(g)
    for (const p of g.photos) {
      if (!keep || p.id !== keep.id) m[p.id] = true
    }
  }
  marks.value = m
  msg.value = '已按「' + (STRATEGIES.find(s => s.id === strategy.value)?.label || '') + '」重新勾选副本'
}

/* 手动指定保留：先把这张设为保留，再取消它的删除勾选 */
async function setKeeper(g, p) {
  const r = await api.setDupKeeper(g.id, p.id)
  if (!r?.ok) { msg.value = '设置保留失败，请稍后重试'; return }
  manualKeep.value = { ...manualKeep.value, [g.id]: p.id }
  const m = { ...marks.value }
  m[p.id] = false
  for (const other of g.photos) {
    if (other.id !== p.id) m[other.id] = true
  }
  marks.value = m
  msg.value = `已保留：${p.name}`
}

function toggleGroup(g, on) {
  const m = { ...marks.value }
  for (const p of g.photos) m[p.id] = on
  marks.value = m
}

/* 只勾选本组副本：保留项由当前策略或手动指定决定 */
function selectCopies(g) {
  const m = { ...marks.value }
  const keep = keeperOf(g)
  for (const p of g.photos) m[p.id] = keep ? p.id !== keep.id : false
  marks.value = m
}

function groupMarked(g) {
  return g.photos.filter(p => marks.value[p.id]).length
}

const markedIds = computed(() =>
  Object.keys(marks.value).filter(k => marks.value[k]).map(Number))

const markedBytes = computed(() => {
  let n = 0
  for (const g of groups.value) {
    for (const p of g.photos) {
      if (marks.value[p.id]) n += p.size || 0
    }
  }
  return n
})

function fmt(n) {
  if (!n) return '0 B'
  const u = ['B', 'KB', 'MB', 'GB']; let i = 0; n = Number(n)
  while (n >= 1024 && i < 3) { n /= 1024; i++ }
  return n.toFixed(1) + ' ' + u[i]
}

/* 删除确认：用统一弹窗，明确告诉用户文件是「移到回收站」而不是直接销毁 */
const deleteAsk = ref(false)
/* 被整组勾选（会把某组删光）的照片名，用于额外警告 */
const wholeGroupWarn = ref([])

function groupWouldBeEmpty(g) {
  const marked = g.photos.filter(p => marks.value[p.id])
  return marked.length > 0 && marked.length === g.photos.length
}

function removeMarked() {
  const ids = markedIds.value
  if (!ids.length) { msg.value = '还没有勾选要删除的照片'; return }
  // 找出被整组勾选的组：这些组删完就一张不剩了
  wholeGroupWarn.value = groups.value
    .filter(groupWouldBeEmpty)
    .map(g => `组 #${g.id}（${g.photos[0]?.name || ''} 等 ${g.photos.length} 张）`)
  deleteAsk.value = true
}

async function doDelete() {
  const ids = markedIds.value
  if (!ids.length) return
  const allowWhole = wholeGroupWarn.value.length > 0
  deleteAsk.value = false
  busy.value = true
  msg.value = '正在移到回收站…'
  // permanent 不传，后端默认走回收目录，可随时取回；
  // allowWhole 只在用户看到「整组会被删光」提示并确认后才为 true
  const r = await api.deletePhotos(ids, { allow_whole_group: allowWhole })
  busy.value = false
  if (r && r.ok) {
    recycleDir.value = r.dest || ''
    const protectedNote = r.protected
      ? `，${r.protected} 张因「每组至少保留一张」被留下`
      : ''
    msg.value = `已移到回收站 ${r.moved} 张，可在「回收站」里取回${protectedNote}`
    wholeGroupWarn.value = []
    await load()
  } else {
    msg.value = '删除失败：' + (r?.error || '后端无响应')
  }
}

async function openRecycle() {
  if (recycleDir.value) await native.openPath(recycleDir.value)
}

function keepMarked() {
  marks.value = {}
  msg.value = '已取消全部勾选'
}

/* 一键选中所有组里除保留项之外的副本 */
function selectAllCopies() {
  applyStrategy()
}
</script>

<template>
  <div class="page">
    <div class="page-head card">
      <div class="page-head-main">
        <div class="page-head-ico grad-pink"><Icon name="duplicate" :size="20" /></div>
        <div class="page-head-text">
          <strong>重复照片 · {{ groups.length }} 组</strong>
          <span>按选定的保留依据自动勾选副本，也可以逐张指定保留哪张</span>
        </div>
      </div>
      <div class="page-head-acts">
        <span class="tally">已选 {{ markedIds.length }} 张 · {{ fmt(markedBytes) }}</span>
        <button class="btn btn-ghost" :disabled="busy" @click="selectAllCopies">选择全部副本</button>
        <button class="btn btn-ghost" :disabled="busy" @click="keepMarked">全部取消</button>
        <button class="btn btn-primary" :disabled="busy || !markedIds.length" @click="removeMarked">
          <Icon name="trash" :size="15" /> 删除选中
        </button>
      </div>
    </div>

    <div class="strat card">
      <label class="strat-label">
        保留依据
        <select v-model="strategy" class="strat-select" @change="applyStrategy">
          <option v-for="s in STRATEGIES" :key="s.id" :value="s.id">
            {{ s.label }} — {{ s.hint }}
          </option>
        </select>
      </label>
      <span class="strat-note">手动点「保留这张」的组不会被策略覆盖；同一组不会被全部删光。</span>
    </div>

    <div v-if="msg" class="msg">
      {{ msg }}
      <button v-if="recycleDir" class="link" @click="openRecycle">打开回收目录</button>
    </div>

    <div v-if="!groups.length && !loading" class="empty-state">
      <div class="es-ico"><Icon name="duplicate" :size="26" /></div>
      <h3 class="es-title">没有发现重复照片</h3>
      <p class="es-desc">感知哈希可识别缩放、调亮、裁边后的副本</p>
    </div>

    <div v-else class="groups">
      <section v-for="g in visibleGroups" :key="g.id" class="group card">
        <header>
          <div class="gtitle">
            <span class="gid">组 #{{ g.id }}</span>
            <span class="badge" :class="g.kind">{{ g.kind === 'exact' ? '完全相同' : '高度相似' }}</span>
          </div>
          <span class="gmeta">{{ g.photos.length }} 张 · 已选 {{ groupMarked(g) }}</span>
          <div class="gacts">
            <button class="mini" @click="toggleGroup(g, true)">全选本组</button>
            <button class="mini" @click="toggleGroup(g, false)">全不选</button>
            <button class="mini accent" @click="selectCopies(g)" title="只勾选本组副本，自动留下保留项">
              选本组副本
            </button>
          </div>
        </header>
        <div class="gbody">
          <figure
            v-for="p in g.photos" :key="p.id"
            class="slot"
            :class="{ marked: marks[p.id], best: keeperOf(g)?.id === p.id }"
          >
            <label class="tick">
              <input type="checkbox" :checked="!!marks[p.id]" @change="toggle(p.id)" />
            </label>
            <div class="slot-img" @click="openViewer(p, g.photos)" title="查看大图">
              <img :src="thumbUrl(p, 420)" :alt="p.name" loading="lazy" />
            </div>
            <figcaption>
              <span v-if="keeperOf(g)?.id === p.id" class="tagbest">
                保留<template v-if="manualKeep[g.id]"> · 手动</template>
              </span>
              <span class="pname" :title="p.name">{{ p.name }}</span>
              <span class="psize">{{ fmt(p.size) }}</span>
              <button
                v-if="keeperOf(g)?.id !== p.id"
                class="keep-one" @click="setKeeper(g, p)"
              >保留这张</button>
            </figcaption>
          </figure>
        </div>
      </section>
    </div>
    <ConfirmDialog
      :show="deleteAsk"
      :title="`移走选中的 ${markedIds.length} 个副本？`"
      message="文件会移到「照片整理」自己的回收站，不是 Windows 回收站，可以随时在「回收站」页面取回。"
      :detail="`${markedIds.length} 个文件 · 可释放约 ${fmt(markedBytes)}`"
      :warn="wholeGroupWarn.length
        ? `以下重复组会被整组移走，一张不剩：${wholeGroupWarn.join('、')}。如果其中还有想保留的，请取消勾选后再删。`
        : ''"
      confirm-text="移到回收站"
      level="normal"
      icon="trash"
      :busy="busy"
      @confirm="doDelete"
      @cancel="deleteAsk = false"
    />
  </div>
</template>

<style scoped>
/* 根容器/页头/空态已统一到 base.css，这里保留重复组特有的样式 */
.tally { font-size: var(--fs-xs); color: var(--text-tertiary); white-space: nowrap; }
.msg { padding: 11px 15px; border-radius: var(--r-sm); background: var(--tint); font-size: var(--fs-sm); }
.link { color: var(--brand-400); text-decoration: underline; margin-left: 8px; }
.groups { display: flex; flex-direction: column; gap: 12px; }
.group { padding: 14px 18px; }
.group header { display: flex; flex-wrap: wrap; align-items: center; gap: 10px; margin-bottom: 10px; }
.gtitle { display: flex; align-items: center; gap: 10px; }
.gid { font-weight: 600; font-size: var(--fs-base); }
.badge { padding: 2px 9px; border-radius: var(--r-pill); font-size: var(--fs-xs); font-weight: 600; color: #fff; }
.badge.exact { background: var(--success); }
.badge.similar { background: var(--brand-500); }
.gmeta { font-size: var(--fs-sm); color: var(--text-tertiary); }
.gacts { margin-left: auto; display: flex; gap: 6px; }
.mini { padding: 4px 10px; border-radius: var(--r-xs); background: var(--surface-2); font-size: var(--fs-xs); color: var(--text-secondary); }
.mini:hover { background: var(--surface-3); color: var(--text-primary); }
.mini.accent { background: var(--tint-strong); color: var(--brand-300); }
.mini.accent:hover { background: var(--tint-strong); color: var(--brand-400); }
.mini.keep { background: rgba(34,197,94,.14); color: var(--success); }
.mini.keep:hover { background: rgba(34,197,94,.24); }
.strat {
  display: flex; flex-wrap: wrap; align-items: center; gap: 12px;
  padding: 12px 16px;
}
.strat-label { display: flex; align-items: center; gap: 9px; font-size: var(--fs-sm); color: var(--text-secondary); }
.strat-select {
  height: 34px; padding: 0 10px; border-radius: var(--r-sm);
  background: var(--surface-2); color: var(--text-primary);
  border: 1px solid var(--line); font-size: var(--fs-sm); cursor: pointer;
}
.strat-note { font-size: var(--fs-xs); color: var(--text-tertiary); }
.gbody { display: flex; gap: 10px; overflow-x: auto; padding-bottom: 2px; }
.slot {
  position: relative; flex: none; width: 150px; border-radius: var(--r-sm);
  overflow: hidden; background: var(--surface-2); box-shadow: var(--shadow-sm);
  transition: box-shadow var(--dur) var(--ease);
}
.slot.best { box-shadow: 0 0 0 2px var(--success); }
.slot.marked { box-shadow: 0 0 0 2px var(--danger); }
.slot-img { height: 104px; display: flex; align-items: center; justify-content: center; background: #0c0e12; cursor: pointer; }
.slot-img img { max-width: 100%; max-height: 100%; object-fit: contain; display: block; }
.tick { position: absolute; left: 6px; top: 6px; z-index: 2; display: grid; place-items: center; width: 22px; height: 22px; border-radius: 6px; background: rgba(0,0,0,.55); }
.tick input { width: 14px; height: 14px; accent-color: var(--danger); cursor: pointer; }
.marked .slot-img { opacity: .55; }
.slot figcaption { padding: 6px 8px 8px; display: flex; flex-direction: column; gap: 2px; }
.tagbest { font-size: 10px; font-weight: 700; color: var(--success); }
.pname { font-size: var(--fs-xs); color: var(--text-secondary); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.psize { font-size: 10px; color: var(--text-tertiary); }
.keep-one {
  margin-top: 4px; padding: 3px 7px; border-radius: var(--r-xs);
  background: rgba(34,197,94,.14); color: var(--success);
  font-size: 10px; text-align: center;
}
.keep-one:hover { background: rgba(34,197,94,.24); }
</style>
