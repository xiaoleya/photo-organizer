<script setup>
/* 整理归档：按地点/时间组合分层输出，未识别照片单独归到一个文件夹。 */
import { ref, computed, onMounted } from 'vue'
import Icon from '../components/Icon.vue'
import { api, native, waitForBackend } from '../api'

const NAMINGS = [
  { id: 'keep', label: '保持原文件名', desc: '不改名，只按文件夹分类' },
  { id: 'date', label: '加上日期前缀', desc: '例：1985-06-12_原名.jpg' },
]
const mode = ref('copy')
const naming = ref('keep')
const dest = ref('')
const plan = ref([])
const total = ref(0)
const msg = ref('')
const msgType = ref('info')
const busy = ref(false)
const done = ref(null)
const unknownPhotos = ref(0)
const knownPlaces = ref(0)

/* 分层依据：勾选什么就按什么分层，顺序固定为 年 > 月 > 地点 */
const layers = ref({ year: true, month: true, place: false })
/* 收藏照片的处理方式：both=收藏夹+分类 / only=仅收藏夹 / regular=仅分类 */
const favMode = ref('both')
const favDir = computed(() => favMode.value !== 'regular')
const favInRegular = computed(() => favMode.value === 'both')

const template = computed(() => naming.value === 'keep' ? '{name}' : '{date}_{name}')

const parts = computed(() => {
  const out = []
  if (layers.value.year) out.push('{year}')
  if (layers.value.month) out.push('{year}-{month}')
  if (layers.value.place) out.push('{place}')
  return out
})

const layerLabel = computed(() => {
  const names = []
  if (layers.value.year) names.push('年')
  if (layers.value.month) names.push('月')
  if (layers.value.place) names.push('地点')
  return names.length ? names.join(' / ') : '全部平铺（不分子文件夹）'
})

onMounted(async () => {
  const ok = await waitForBackend()
  if (!ok) { setMsg('后端启动较慢，请稍候', 'warn'); return }
  const st = await api.stats()
  if (st) knownPlaces.value = st.places || 0
  await preview()
})

function setMsg(t, ty = 'info') { msg.value = t; msgType.value = ty }

async function preview() {
  if (!parts.value.length && !layers.value.place && !layers.value.year && !layers.value.month) {
    // 平铺也是合法选择
  }
  const r = await api.organize({
    scheme: 'flat', template: template.value,
    parts: parts.value, dry_run: true,
    favorite_dir: favDir.value, favorite_in_regular: favInRegular.value,
  })
  if (r && !r.error) {
    plan.value = r.plan || []
    total.value = r.total || 0
    unknownPhotos.value = plan.value.filter(x => x.path.startsWith('未识别地点')).length
  } else {
    setMsg('预览失败：' + (r?.error || '后端无响应'), 'err')
  }
}

async function pickDest() {
  const d = await native.pickFolder()
  if (d) { dest.value = d; setMsg('') }
}
async function openDest() {
  if (dest.value) await native.openPath(dest.value)
}

async function execute() {
  if (!dest.value) { setMsg('请先选择输出文件夹', 'warn'); return }
  busy.value = true; done.value = null; setMsg('正在整理…')
  const r = await api.organize({
    scheme: 'flat', template: template.value, parts: parts.value,
    mode: mode.value, dest: dest.value, dry_run: false,
    favorite_dir: favDir.value, favorite_in_regular: favInRegular.value,
  })
  busy.value = false
  if (r && !r.error) {
    done.value = r
    setMsg(`完成：${r.done} 张已整理`, 'ok')
  } else {
    setMsg('整理失败：' + (r?.error || '后端无响应'), 'err')
  }
}

function toggleLayer(key) {
  layers.value = { ...layers.value, [key]: !layers.value[key] }
  preview()
}
</script>

<template>
  <div class="page">
    <div class="page-head card">
      <div class="page-head-main">
        <div class="page-head-ico grad-green"><Icon name="organize" :size="20" /></div>
        <div class="page-head-text">
          <strong>整理归档</strong>
          <span>按年 / 月 / 地点复制或移动照片到新目录</span>
        </div>
      </div>
    </div>

    <section class="card block">
      <header>
        <span class="bico grad-blue"><Icon name="organize" :size="18" /></span>
        <div class="ht"><strong>怎么分类</strong><p>可多选，文件夹层级固定为：年 / 月 / 地点</p></div>
      </header>

      <div class="schemes">
        <button class="scheme" :class="{ on: layers.year }" @click="toggleLayer('year')">
          <span class="sico grad-blue"><Icon name="calendar" :size="18" /></span>
          <span class="stext"><b>按年份</b><i>文件夹用 4 位年份</i></span>
          <Icon v-if="layers.year" name="check" :size="16" />
        </button>
        <button class="scheme" :class="{ on: layers.month }" @click="toggleLayer('month')">
          <span class="sico grad-green"><Icon name="clock" :size="18" /></span>
          <span class="stext"><b>再按月份</b><i>在年份下再分一层</i></span>
          <Icon v-if="layers.month" name="check" :size="16" />
        </button>
        <button class="scheme" :class="{ on: layers.place }" @click="toggleLayer('place')">
          <span class="sico grad-violet"><Icon name="pinFilled" :size="18" /></span>
          <span class="stext">
            <b>再按地点</b>
            <i>已识别 {{ knownPlaces }} 个地点</i>
          </span>
          <Icon v-if="layers.place" name="check" :size="16" />
        </button>
      </div>

      <div class="hint-row">
        当前层级：<b>{{ layerLabel }}</b>
        <span v-if="!layers.place && !layers.year && !layers.month">（所有照片放在同一个文件夹）</span>
      </div>
      <div class="hint-row muted">
        没有时间或没有地点的照片会放进「无年份 / 00 月 / 未识别地点」这类占位文件夹，不会被漏掉。
      </div>

      <!-- 收藏照片怎么放：下拉选择，节省纵向空间 -->
      <div class="row">
        <label>收藏照片</label>
        <select class="fav-select" v-model="favMode" @change="preview">
          <option value="both">单独放「收藏」文件夹，同时保留在分类里</option>
          <option value="only">只放「收藏」文件夹</option>
          <option value="regular">不单独放，只按分类整理</option>
        </select>
      </div>

      <div class="sub-label">要不要改名</div>
      <div class="namings">
        <button
          v-for="n in NAMINGS" :key="n.id" class="naming"
          :class="{ on: naming === n.id }"
          @click="naming = n.id; preview()"
        >
          <span class="ntitle">{{ n.label }}</span>
          <span class="ndesc">{{ n.desc }}</span>
        </button>
      </div>

      <div class="row">
        <label>输出到</label>
        <div class="destbox" :class="{ empty: !dest }" @click="pickDest">
          <Icon name="folder" :size="16" />
          <span class="ellipsis">{{ dest || '点击选择文件夹' }}</span>
        </div>
        <button class="btn btn-ghost" @click="pickDest">浏览…</button>
      </div>

      <div class="row">
        <label>处理方式</label>
        <div class="modes">
          <button
            v-for="m in [{id:'copy',label:'复制（保留原文件）'},{id:'move',label:'移动'}]"
            :key="m.id" class="mode" :class="{ on: mode === m.id }"
            @click="mode = m.id"
          >{{ m.label }}</button>
        </div>
      </div>

      <!-- 开始整理：与上方配置合并，不再单独一个预览块 -->
      <div class="exec-row">
        <button class="btn btn-primary" :disabled="busy" @click="execute">
          <Icon name="check" :size="16" /> {{ busy ? '整理中…' : '开始整理' }}
        </button>
        <span class="exec-hint">共 {{ total }} 个文件将按 <b>{{ layerLabel }}</b> 整理</span>
      </div>

      <div v-if="msg" class="msgbox" :class="msgType">{{ msg }}</div>

      <div v-if="done" class="donebox">
        <div class="doneleft">
          <strong>整理完成</strong>
          <span>{{ done.done }} 张已处理，跳过 {{ done.skipped }}，失败 {{ done.errors }}</span>
        </div>
        <button class="btn btn-primary" @click="openDest">
          <Icon name="folder" :size="16" /> 打开文件夹
        </button>
      </div>
    </section>
  </div>
</template>

<style scoped>
/* 根容器/页头已统一到 base.css，这里保留归档表单的样式 */
.block { padding: 16px 20px; }
.block header { display: flex; align-items: flex-start; gap: 12px; margin-bottom: 12px; }
.ht { display: flex; flex-direction: column; gap: 2px; padding-top: 1px; }
.ht strong { font-size: var(--fs-md); font-weight: 600; }
.ht p { margin: 0; font-size: var(--fs-xs); color: var(--text-tertiary); }
.bico { display: grid; place-items: center; width: 32px; height: 32px; flex: none; border-radius: var(--r-sm); color: #fff; }
.schemes { display: grid; grid-template-columns: repeat(auto-fill, minmax(180px, 1fr)); gap: 8px; }
.scheme {
  display: flex; align-items: center; gap: 9px; padding: 9px 11px;
  border-radius: var(--r-sm); background: var(--surface-2); text-align: left;
  transition: box-shadow var(--dur) var(--ease), background var(--dur) var(--ease);
}
.scheme:hover { background: var(--surface-3); }
.scheme.on { box-shadow: inset 0 0 0 2px var(--brand-500); background: var(--tint); }
.sico { display: grid; place-items: center; width: 26px; height: 26px; flex: none; border-radius: var(--r-xs); color: #fff; }
.stext { display: flex; flex-direction: column; gap: 0; flex: 1; min-width: 0; }
.stext b { font-size: var(--fs-sm); font-weight: 600; }
.stext i { font-style: normal; font-size: 10px; color: var(--text-tertiary); }
.scheme svg:last-child { color: var(--brand-400); flex: none; }
.hint-row { margin-top: 9px; font-size: var(--fs-sm); color: var(--text-secondary); }
.hint-row b { color: var(--brand-300); }
.hint-row.muted { color: var(--text-tertiary); font-size: var(--fs-xs); margin-top: 3px; }

.fav-select {
  flex: 1; height: 34px; padding: 0 30px 0 12px;
  border-radius: var(--r-sm); background: var(--surface-2);
  color: var(--text-primary); font-size: var(--fs-sm);
  border: 1px solid var(--line); cursor: pointer;
}
.sub-label { margin: 13px 0 7px; font-size: var(--fs-sm); font-weight: 600; color: var(--text-secondary); }
.namings { display: grid; grid-template-columns: 1fr 1fr; gap: 8px; }
.naming {
  display: flex; flex-direction: column; gap: 2px; padding: 9px 12px;
  border-radius: var(--r-sm); background: var(--surface-2); text-align: left;
}
.naming:hover { background: var(--surface-3); }
.naming.on { box-shadow: inset 0 0 0 2px var(--brand-500); background: var(--tint); }
.ntitle { font-size: var(--fs-sm); font-weight: 600; }
.ndesc { font-size: 10px; color: var(--text-tertiary); }
.row { display: flex; align-items: center; gap: 10px; margin-top: 10px; }
.row label { width: 68px; flex: none; font-size: var(--fs-sm); color: var(--text-secondary); }
.destbox {
  flex: 1; display: flex; align-items: center; gap: 9px; height: 34px;
  padding: 0 12px; border-radius: var(--r-sm); background: var(--surface-2);
  font-size: var(--fs-sm); cursor: pointer;
}
.destbox svg { color: var(--brand-400); flex: none; }
.destbox.empty span { color: var(--text-tertiary); }
.modes { display: flex; gap: 7px; }
.mode { padding: 6px 13px; border-radius: var(--r-sm); background: var(--surface-2); font-size: var(--fs-sm); }
.mode.on { background: var(--tint-strong); color: var(--brand-300); box-shadow: inset 0 0 0 1.5px var(--brand-500); }
.push { margin-left: auto; }
.exec-row { display: flex; flex-wrap: wrap; align-items: center; gap: 14px; margin-top: 18px; }
.exec-hint { font-size: var(--fs-sm); color: var(--text-tertiary); }
.exec-hint b { color: var(--brand-300); }
.msgbox { padding: 10px 14px; margin-bottom: 12px; border-radius: var(--r-sm); font-size: var(--fs-sm); background: var(--tint); }
.msgbox.ok { background: rgba(34,197,94,.12); color: var(--success); }
.msgbox.err { background: rgba(239,68,68,.12); color: var(--danger); }
.msgbox.warn { background: rgba(245,158,11,.12); color: var(--warning); }
.donebox {
  display: flex; align-items: center; justify-content: space-between; gap: 16px;
  padding: 13px 16px; margin-bottom: 12px; border-radius: var(--r-md);
  background: rgba(34,197,94,.10);
}
.doneleft { display: flex; flex-direction: column; gap: 2px; }
.doneleft strong { color: var(--success); font-size: var(--fs-base); }
.doneleft span { font-size: var(--fs-sm); color: var(--text-secondary); }
.table { border: 1px solid var(--line); border-radius: var(--r-sm); overflow: hidden; }
.tr { display: grid; grid-template-columns: 60px 1fr; gap: 12px; padding: 8px 12px; font-size: var(--fs-sm); border-bottom: 1px solid var(--line); }
.tr:last-child { border-bottom: 0; }
.tr.th { background: var(--surface-2); color: var(--text-tertiary); font-weight: 600; }
.tr span:first-child { color: var(--text-tertiary); }
.noplan { padding: 22px; text-align: center; color: var(--text-tertiary); font-size: var(--fs-sm); }
</style>
