<script setup>
/* 照片修复：预设 + 可按项勾选，实时前后对比，支持自选照片与批量。
   前端显示的都是后端真正执行过的结果，不是占位图。 */
import { ref, computed, onMounted, watch } from 'vue'
import Icon from '../components/Icon.vue'
import { api, native } from '../api'
import { thumbUrl, photoUrl } from '../thumb'
import { openImage } from '../viewer'

const props = defineProps({
  pick: { type: Object, default: null },   // 从别的页面“去修复”跳过来时带上的照片
})

const photos = ref([])
const selected = ref(null)
const preset = ref('标准修复')
const presetList = ref([])
const optionsMeta = ref([])
const opts = ref({})
const busy = ref(false)
const msg = ref('')
const result = ref(null)          // { image, before, after, gain }
const beforeUrl = ref('')
const total = ref(0)
const page = ref(1)
const pageSize = 60
const search = ref('')
const picked = ref([])            // 批量修复的勾选
const batchMsg = ref('')
const savedMsg = ref('')
const screenLike = ref(false)

onMounted(async () => {
  const pr = await api.presets()
  if (pr) {
    presetList.value = pr.presets || []
    optionsMeta.value = pr.options || []
    opts.value = { ...(pr.defaults || {}) }
  }
  await loadPhotos(true)
  if (props.pick) applyPick(props.pick)
})

/* 输入即搜：防抖 250ms，不用按回车 */
let searchTimer = null
watch(search, () => {
  if (searchTimer) clearTimeout(searchTimer)
  searchTimer = setTimeout(() => loadPhotos(true), 250)
})

async function loadPhotos(reset = true) {
  if (reset) { photos.value = []; page.value = 1; total.value = 0 }
  const q = { limit: pageSize, offset: (page.value - 1) * pageSize, sort: 'age_score DESC' }
  if (search.value.trim()) q.q = search.value.trim()
  const r = await api.photos(q)
  if (r && r.items) {
    photos.value = reset ? r.items : photos.value.concat(r.items)
    total.value = r.total || photos.value.length
  }
}

function more() {
  if (photos.value.length >= total.value) return
  page.value += 1
  loadPhotos(false)
}

function applyPick(p) {
  selected.value = p
  result.value = null
  // 关键：修复前也要用原尺寸图，否则小图放大看起来比修复后糊。
  // 之前用 900px 缩略图，而修复后是原分辨率，两边不对等。
  beforeUrl.value = photoUrl(p)
  msg.value = ''
}

function pick(p) {
  applyPick(p)
}

async function applyPreset(name) {
  preset.value = name
  const pr = await api.presets()
  if (pr && pr.presets) {
    optionsMeta.value = pr.options || optionsMeta.value
  }
  // 重新取一次该预设的默认开关
  const defaults = await api.presets()
  if (defaults) opts.value = { ...(defaults.defaults || {}) }
}

async function runRestore() {
  if (!selected.value) { msg.value = '请先在左侧选一张照片'; return }
  busy.value = true
  msg.value = '正在修复…'
  result.value = null
  savedMsg.value = ''
  const r = await api.restoreApply(selected.value.id, preset.value, opts.value)
  busy.value = false
  if (r && !r.error) {
    result.value = r
    screenLike.value = !!r.screen_like
    msg.value = r.screen_like
      ? '修复完成（这张是截图/文档，已自动跳过裁边与纠偏，避免把图转斜）'
      : '修复完成，记得点「另存到…」把结果保存下来'
  } else {
    msg.value = '修复失败：' + (r?.error || '后端无响应')
  }
}

async function saveCopy() {
  if (!result.value) { msg.value = '请先点「应用修复」'; return }
  const d = await native.pickFolder()
  if (!d) return
  const r2 = await api.restoreBatch([selected.value.id], d, preset.value, opts.value)
  if (r2 && r2.ok) {
    savedMsg.value = '已保存到：' + d
    msg.value = '已保存 1 张修复结果'
  } else {
    msg.value = '保存失败：' + (r2?.error || '后端无响应')
  }
}

/* 清除上一次的修复结果与选中状态，回到干净状态 */
function clearSession() {
  selected.value = null
  result.value = null
  beforeUrl.value = ''
  msg.value = ''
  savedMsg.value = ''
  screenLike.value = false
  picked.value = []
  batchMsg.value = ''
}

/* 从照片库"去修复"跳进来时，替换掉上一次选中的照片 */
watch(() => props.pick, (p) => {
  if (p) applyPick(p)
})

function togglePick(id) {
  picked.value = picked.value.includes(id)
    ? picked.value.filter(x => x !== id)
    : picked.value.concat(id)
}

async function batchRestore() {
  if (!picked.value.length) { batchMsg.value = '请先勾选要修复的照片'; return }
  const d = await native.pickFolder()
  if (!d) return
  busy.value = true
  batchMsg.value = '正在批量修复…'
  const r = await api.restoreBatch(picked.value, d, preset.value, opts.value)
  busy.value = false
  if (r && r.ok) batchMsg.value = `完成 ${r.done} 张，失败 ${r.errors} 张 → ${d}`
  else batchMsg.value = '批量修复失败：' + (r?.error || '后端无响应')
}

const gainText = computed(() => {
  if (!result.value || !result.value.gain) return ''
  const g = result.value.gain
  return `对比度 ${g.contrast >= 0 ? '+' : ''}${g.contrast}% · 细节 ${g.detail >= 0 ? '+' : ''}${g.detail}% · 泛黄 ${g.sepia >= 0 ? '-' : '+'}${Math.abs(g.sepia)}%`
})

const showOpts = computed(() => optionsMeta.value.filter(o => o.key !== 'upscale'))
</script>

<template>
  <div class="page page-flush">
    <div class="page-head card">
      <div class="page-head-main">
        <div class="page-head-ico grad-violet"><Icon name="wand" :size="20" /></div>
        <div class="page-head-text">
          <strong>照片修复</strong>
          <span>去泛黄、去划痕、增强清晰度，前后对比可批量处理</span>
        </div>
      </div>
    </div>

    <div class="toolbar card">
      <span class="lb">预设</span>
      <select v-model="preset">
        <option v-for="p in presetList" :key="p">{{ p }}</option>
      </select>
      <button class="btn btn-primary" :disabled="busy" @click="runRestore">
        <Icon name="wand" :size="16" /> {{ busy ? '修复中…' : '应用修复' }}
      </button>
      <button class="btn btn-ghost" :disabled="busy || !result" @click="saveCopy">
        <Icon name="download" :size="16" /> 另存到…
      </button>
      <span class="msg">{{ msg }}</span>
      <span v-if="savedMsg" class="saved">{{ savedMsg }}</span>
    </div>

    <div class="opts card">
      <span class="opts-lb">修复项目</span>
      <label v-for="o in showOpts" :key="o.key" class="opt" :title="o.desc">
        <input type="checkbox" v-model="opts[o.key]" />
        <span>{{ o.label }}</span>
      </label>
    </div>

    <div class="body">
      <div class="list card">
        <div class="list-head">
          <span>照片 <b>{{ total }}</b></span>
          <div class="list-actions">
            <button class="mini-btn" :disabled="busy || !picked.length" @click="batchRestore">
              批量修复（{{ picked.length }}）
            </button>
          </div>
        </div>
        <div class="list-search">
          <Icon name="search" :size="14" />
          <input v-model="search" placeholder="搜索文件名或地点（输入即搜）" />
          <button v-if="search" class="clear" @click="search = ''">✕</button>
        </div>
        <div v-if="batchMsg" class="batch-msg">{{ batchMsg }}</div>
        <div class="list-body">
          <div
            v-for="p in photos" :key="p.id" class="row"
            :class="{ on: selected?.id === p.id }"
          >
            <input class="pickbox" type="checkbox" :checked="picked.includes(p.id)" @change="togglePick(p.id)" />
            <button class="pick" @click="pick(p)">
              <div class="rthumb"><img :src="thumbUrl(p, 120)" alt="" loading="lazy" /></div>
              <span class="rname">{{ p.name }}</span>
            </button>
          </div>
          <div v-if="!photos.length" class="nolist">没有照片，请先整理目录</div>
          <button v-if="photos.length < total" class="more" @click="more">加载更多（剩余 {{ total - photos.length }}）</button>
        </div>
      </div>

      <div class="preview card">
        <template v-if="selected">
          <div class="pv-head">
            <strong class="ellipsis">{{ selected.name }}</strong>
            <span class="pv-gain">{{ gainText }}</span>
          </div>
          <div class="pv-grid">
            <div class="pv">
              <div class="pv-label">修复前<button class="zoom" title="放大" @click="openImage(beforeUrl || photoUrl(selected), '修复前 · ' + selected.name)">放大</button></div>
              <img v-if="beforeUrl" :src="beforeUrl" alt="" @click="openImage(beforeUrl, '修复前 · ' + selected.name)" />
              <div v-else class="pv-ph"><Icon name="image" :size="30" /></div>
            </div>
            <div class="pv">
              <div class="pv-label after">修复后<button v-if="result" class="zoom" title="放大" @click="openImage(result.preview || result.image, '修复后 · ' + selected.name)">放大</button></div>
              <img v-if="result" :src="result.preview || result.image" alt="" @click="openImage(result.preview || result.image, '修复后 · ' + selected.name)" />
              <div v-else class="pv-ph hint"><Icon name="wand" :size="30" /><span>点「应用修复」查看效果</span></div>
            </div>
          </div>
          <div v-if="result" class="cmp">
            <span>对比度 {{ result.before.contrast.toFixed(2) }} → {{ result.after.contrast.toFixed(2) }}</span>
            <span>泛黄 {{ result.before.sepia.toFixed(2) }} → {{ result.after.sepia.toFixed(2) }}</span>
            <span>清晰度 {{ Math.round(result.before.sharpness) }} → {{ Math.round(result.after.sharpness) }}</span>
          </div>
        </template>
        <div v-else class="pv-empty">
          <div class="eico"><Icon name="restore" :size="30" /></div>
          <h3>选一张照片开始修复</h3>
          <p>去黄、去划痕、增强对比；也可以从照片库点「去修复」跳过来</p>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
/* 根容器已统一到 base.css 的 .page（修复页是分栏布局，用 page-flush 去掉底部留白） */
.wrap-inner { min-height: 0; }
.toolbar { display: flex; align-items: center; gap: 10px; padding: 11px 15px; flex: none; }
.lb { font-size: var(--fs-sm); color: var(--text-tertiary); }
select {
  height: 34px; padding: 0 26px 0 11px; border-radius: var(--r-sm);
  background: var(--surface-2); color: var(--text-primary);
  font-size: var(--fs-base); border: 1px solid var(--line); cursor: pointer;
}
.msg { font-size: var(--fs-sm); color: var(--text-tertiary); margin-left: auto; }
.saved { font-size: var(--fs-xs); color: var(--success); }
.opts { display: flex; flex-wrap: wrap; align-items: center; gap: 8px 14px; padding: 10px 15px; flex: none; }
.opts-lb { font-size: var(--fs-sm); color: var(--text-tertiary); }
.opt { display: inline-flex; align-items: center; gap: 5px; font-size: var(--fs-sm); color: var(--text-secondary); cursor: pointer; }
.opt input { width: 14px; height: 14px; accent-color: var(--brand-500); }
.body { flex: 1; min-height: 0; display: grid; grid-template-columns: 300px 1fr; gap: 10px; }
.list { display: flex; flex-direction: column; overflow: hidden; }
.list-head { display: flex; align-items: center; justify-content: space-between; padding: 12px 14px; border-bottom: 1px solid var(--line); font-size: var(--fs-base); font-weight: 600; }
.list-head b { color: var(--text-tertiary); font-weight: 400; }
.mini-btn { padding: 5px 10px; border-radius: var(--r-xs); background: var(--tint-strong); color: var(--brand-300); font-size: var(--fs-xs); }
.mini-btn:disabled { opacity: .5; }
.list-search { display: flex; align-items: center; gap: 7px; padding: 8px 12px; border-bottom: 1px solid var(--line); color: var(--text-tertiary); }
.list-search input { flex: 1; min-width: 0; font-size: var(--fs-sm); color: var(--text-primary); }
.batch-msg { padding: 6px 12px; font-size: var(--fs-xs); color: var(--brand-300); border-bottom: 1px solid var(--line); }
.list-body { flex: 1; overflow-y: auto; padding: 6px; }
.row { display: flex; align-items: center; gap: 6px; border-radius: var(--r-sm); }
.row:hover { background: var(--surface-hover); }
.row.on { background: var(--tint-strong); }
.pickbox { width: 14px; height: 14px; margin-left: 4px; accent-color: var(--brand-500); flex: none; }
.pick { display: flex; align-items: center; gap: 9px; flex: 1; min-width: 0; padding: 5px 6px; text-align: left; }
.rthumb { width: 44px; height: 36px; border-radius: 6px; overflow: hidden; flex: none; background: var(--surface-3); }
.rthumb img { width: 100%; height: 100%; object-fit: cover; display: block; }
.rname { font-size: var(--fs-sm); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.nolist { padding: 20px; font-size: var(--fs-sm); color: var(--text-tertiary); text-align: center; }
.more { width: 100%; margin-top: 8px; padding: 8px; border-radius: var(--r-sm); background: var(--surface-2); font-size: var(--fs-xs); color: var(--text-secondary); }
.preview { display: flex; flex-direction: column; overflow: hidden; padding: 14px; }
.pv-head { display: flex; align-items: center; justify-content: space-between; gap: 12px; margin-bottom: 10px; }
.pv-head strong { font-size: var(--fs-base); }
.pv-gain { font-size: var(--fs-xs); color: var(--success); }
.pv-grid { flex: 1; min-height: 0; display: grid; grid-template-columns: 1fr 1fr; gap: 12px; }
.pv { display: flex; flex-direction: column; min-height: 0; border-radius: var(--r-md); overflow: hidden; background: var(--surface-2); }
.pv-label { padding: 7px 11px; font-size: var(--fs-sm); font-weight: 600; color: var(--text-secondary); display: flex; align-items: center; justify-content: space-between; gap: 8px; }
.pv-label.after { color: var(--brand-400); }
.zoom { padding: 2px 9px; border-radius: var(--r-pill); background: var(--surface-3); font-size: 10px; color: var(--text-secondary); }
.zoom:hover { background: var(--tint-strong); color: var(--brand-300); }
.pv img { flex: 1; min-height: 0; width: 100%; object-fit: contain; cursor: zoom-in; }
.pv-ph { flex: 1; display: grid; place-items: center; color: var(--text-tertiary); }
.pv-ph.hint { display: flex; flex-direction: column; gap: 8px; font-size: var(--fs-sm); }
.cmp { display: flex; flex-wrap: wrap; gap: 16px; padding-top: 10px; font-size: var(--fs-sm); color: var(--text-secondary); }
.pv-empty { flex: 1; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 10px; color: var(--text-tertiary); }
.eico { display: grid; place-items: center; width: 64px; height: 64px; border-radius: var(--r-lg); background: var(--tint); color: var(--brand-400); }
.pv-empty h3 { margin: 0; color: var(--text-secondary); }
.pv-empty p { margin: 0; font-size: var(--fs-sm); }
</style>
