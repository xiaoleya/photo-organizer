<script setup>
/* 重复照片：每组显示全部照片，可逐张勾选要删除的副本（不是固定“非最佳”）。
   每组默认勾选除最佳外的副本，用户可以改选。删除时移到回收目录，可撤销。 */
import { ref, computed, onMounted } from 'vue'
import Icon from '../components/Icon.vue'
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
    const m = {}
    for (const g of r.items) {
      const best = g.photos.find(p => p.is_best) || g.photos[0]
      for (const p of g.photos) {
        if (p.id !== best.id) m[p.id] = true
      }
    }
    marks.value = m
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

function toggleGroup(g, on) {
  const m = { ...marks.value }
  for (const p of g.photos) m[p.id] = on
  marks.value = m
}

/* 只勾选本组里除「最佳」之外的副本：最常用的操作 */
function selectCopies(g) {
  const m = { ...marks.value }
  const best = g.photos.find(p => p.is_best) || g.photos[0]
  for (const p of g.photos) m[p.id] = (p.id !== best.id)
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

async function removeMarked() {
  const ids = markedIds.value
  if (!ids.length) { msg.value = '还没有勾选要删除的照片'; return }
  if (!confirm(`将删除 ${ids.length} 张照片到回收目录，可释放约 ${fmt(markedBytes.value)}。继续？`)) return
  busy.value = true
  msg.value = '正在删除…'
  const r = await api.deletePhotos(ids)
  busy.value = false
  if (r && r.ok) {
    recycleDir.value = r.dest || ''
    msg.value = `已移动 ${r.moved} 张到回收目录`
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

/* 一键选中所有组里除「最佳」之外的副本 */
function selectAllCopies() {
  const m = {}
  for (const g of groups.value) {
    const best = g.photos.find(p => p.is_best) || g.photos[0]
    for (const p of g.photos) {
      if (p.id !== best.id) m[p.id] = true
    }
  }
  marks.value = m
  msg.value = '已选中全部 ' + Object.keys(m).length + ' 张重复副本'
}
</script>

<template>
  <div class="wrap">
    <div class="hero">
      <div class="hero-left">
        <span class="hl">重复照片</span>
        <strong>{{ groups.length }} <small>组</small></strong>
        <span class="sub">默认勾选每组除「最佳」外的副本，可逐张调整</span>
      </div>
      <div class="hero-actions">
        <span class="tally">已选 {{ markedIds.length }} 张 · {{ fmt(markedBytes) }}</span>
        <button class="ghost" :disabled="busy" @click="selectAllCopies">选择全部副本</button>
        <button class="ghost" :disabled="busy" @click="keepMarked">全部取消</button>
        <button class="primary" :disabled="busy || !markedIds.length" @click="removeMarked">
          <Icon name="trash" :size="15" /> 删除选中
        </button>
      </div>
    </div>

    <div v-if="msg" class="msg">
      {{ msg }}
      <button v-if="recycleDir" class="link" @click="openRecycle">打开回收目录</button>
    </div>

    <div v-if="!groups.length && !loading" class="empty card">
      <Icon name="duplicate" :size="30" />
      <h3>没有发现重复照片</h3>
      <p>感知哈希可识别缩放、调亮、裁边后的副本</p>
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
            <button class="mini accent" @click="selectCopies(g)" title="只勾选本组里除「最佳」以外的副本">
              选本组副本
            </button>
          </div>
        </header>
        <div class="gbody">
          <figure
            v-for="p in g.photos" :key="p.id"
            class="slot" :class="{ marked: marks[p.id], best: p.is_best }"
          >
            <label class="tick">
              <input type="checkbox" :checked="!!marks[p.id]" @change="toggle(p.id)" />
            </label>
            <div class="slot-img" @click="openViewer(p, g.photos)" title="查看大图">
              <img :src="thumbUrl(p, 420)" :alt="p.name" loading="lazy" />
            </div>
            <figcaption>
              <span v-if="p.is_best" class="tagbest">最佳</span>
              <span class="pname" :title="p.name">{{ p.name }}</span>
              <span class="psize">{{ fmt(p.size) }}</span>
            </figcaption>
          </figure>
        </div>
      </section>
    </div>
  </div>
</template>

<style scoped>
.wrap { height: 100%; overflow-y: auto; display: flex; flex-direction: column; gap: 14px; padding-bottom: 24px; }
.hero {
  position: relative; display: flex; flex-wrap: wrap; align-items: center;
  justify-content: space-between; gap: 16px; padding: 18px 22px;
  border-radius: var(--r-xl); color: #fff; overflow: hidden; flex: none;
  box-shadow: var(--shadow-md); background: var(--grad-blue);
}
.hero::after {
  content: ''; position: absolute; inset: 0; pointer-events: none;
  background: radial-gradient(90% 130% at 80% 0%, rgba(255,255,255,.28), transparent 65%);
}
.hero-left { display: flex; flex-direction: column; gap: 2px; z-index: 1; }
.hl { font-size: var(--fs-sm); color: rgba(255,255,255,.85); }
.hero-left strong { font-family: var(--font-display); font-size: var(--fs-3xl); line-height: 1; }
.hero-left strong small { font-size: var(--fs-md); font-weight: 500; }
.sub { font-size: var(--fs-sm); color: rgba(255,255,255,.85); }
.hero-actions { display: flex; flex-wrap: wrap; align-items: center; gap: 9px; z-index: 1; }
.tally { font-size: var(--fs-sm); color: rgba(255,255,255,.9); }
.hero-actions button { display: inline-flex; align-items: center; gap: 6px; padding: 8px 14px; border-radius: var(--r-pill); font-size: var(--fs-sm); }
.ghost { background: rgba(255,255,255,.18); color: #fff; }
.ghost:hover { background: rgba(255,255,255,.28); }
.primary { background: #fff; color: var(--brand-600); font-weight: 600; }
.primary:disabled, .ghost:disabled { opacity: .55; cursor: default; }
.msg { padding: 11px 15px; border-radius: var(--r-sm); background: var(--tint); font-size: var(--fs-sm); }
.link { color: var(--brand-400); text-decoration: underline; margin-left: 8px; }
.empty { display: flex; flex-direction: column; align-items: center; justify-content: center; padding: 60px 0; gap: 10px; color: var(--text-tertiary); }
.empty h3 { margin: 0; color: var(--text-secondary); }
.empty p { margin: 0; font-size: var(--fs-sm); }
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
</style>
