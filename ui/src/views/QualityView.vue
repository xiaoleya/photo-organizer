<script setup>
/* 质量待修：每个问题类别都能正确列出照片（分页），并可直接跳去修复。 */
import { ref, computed, onMounted, watch } from 'vue'
import Icon from '../components/Icon.vue'
import PhotosPanel from '../components/PhotosPanel.vue'
import { api } from '../api'

const props = defineProps({
  search: { type: String, default: '' },
})
const emit = defineEmits(['repair'])

const FLAGS = [
  { id: '模糊', icon: 'quality', grad: 'violet', desc: '清晰度低于阈值' },
  { id: '过暗', icon: 'photos', grad: 'blue', desc: '整体亮度偏低' },
  { id: '过亮', icon: 'sparkle', grad: 'amber', desc: '过曝或大面积高光' },
  { id: '褪色平淡', icon: 'restore', grad: 'amber', desc: '对比度、饱和度低' },
  { id: '泛黄/棕褐', icon: 'sparkle', grad: 'pink', desc: '老照片泛黄' },
  { id: '噪点重', icon: 'layers', grad: 'green', desc: '颗粒与扫描噪声多' },
  { id: '低分辨率', icon: 'image', grad: 'blue', desc: '像素过小' },
]
const active = ref('模糊')
const counts = ref({})
const rescanning = ref(false)
const panelKey = ref(0)
const showStandard = ref(false)
const cfgVals = ref({})
const savingCfg = ref(false)
const cfgMsg = ref('')

/* 识别标准：大白话描述 + 后端真实键 */
const FIELDS = [
  { key: 'blur_threshold', title: '多糊才算「模糊」', desc: '数值越大越容易判成模糊', unit: '', step: 1, min: 10, max: 400 },
  { key: 'dark_threshold', title: '多暗才算「过暗」', desc: '0~1，数值越大越容易判成过暗', unit: '', step: 0.01, min: 0.02, max: 0.6 },
  { key: 'bright_threshold', title: '多亮才算「过亮」', desc: '0~1，数值越小越容易判成过亮', unit: '', step: 0.01, min: 0.5, max: 0.99 },
  { key: 'lowres_mp', title: '多少像素算「低分辨率」', desc: '小于它就提示分辨率低', unit: '百万像素', step: 0.1, min: 0.1, max: 12 },
  { key: 'faded_contrast', title: '多灰才算「褪色」', desc: '数值越大越容易判成褪色平淡', unit: '', step: 0.005, min: 0.02, max: 0.4 },
  { key: 'sepia_threshold', title: '多黄才算「泛黄」', desc: '0~1，数值越小越容易判成泛黄', unit: '', step: 0.05, min: 0.1, max: 0.95 },
  { key: 'phash_distance', title: '重复判定有多宽松', desc: '数值越大越容易算成重复', unit: '', step: 1, min: 2, max: 32 },
]

async function reloadStandard() {
  const r = await api.settings()
  const s = (r && r.settings) || {}
  const next = {}
  for (const f of FIELDS) {
    next[f.key] = s[f.key] !== undefined && s[f.key] !== null ? s[f.key] : ''
  }
  cfgVals.value = next
  cfgMsg.value = ''
}

async function saveStandard() {
  savingCfg.value = true
  const payload = {}
  for (const f of FIELDS) {
    const v = cfgVals.value[f.key]
    if (v === '' || v === null || v === undefined) continue
    const n = Number(v)
    if (!Number.isFinite(n)) {
      cfgMsg.value = '「' + f.title + '」要填数字'
      savingCfg.value = false
      return
    }
    payload[f.key] = n
  }
  const r = await api.saveSettings(payload)
  savingCfg.value = false
  cfgMsg.value = (r && r.ok)
    ? '已保存。点「按新标准重新判定」可立即生效'
    : '保存失败：' + (r?.error || '后端无响应')
}

async function refreshCounts() {
  for (const f of FLAGS) {
    const r = await api.photos({ flag: f.id, limit: 1 })
    counts.value = { ...counts.value, [f.id]: r ? r.total : 0 }
  }
}

onMounted(async () => {
  await refreshCounts()
  await reloadStandard()
})

/* 老库里的标记是按旧标准算的，这里可以一键按新标准重判（不重新解码图片） */
async function reclassify() {
  rescanning.value = true
  const r = await api.reclassify()
  rescanning.value = false
  if (r && r.ok) {
    await refreshCounts()
    panelKey.value++
  }
}

const query = computed(() => {
  const q = { flag: active.value, sort: 'quality_score ASC' }
  if (props.search) q.q = props.search
  return q
})
const activeInfo = computed(() => FLAGS.find(f => f.id === active.value) || FLAGS[0])
watch(active, () => {})
</script>

<template>
  <div class="wrap">
    <div class="flags">
      <button
        v-for="f in FLAGS" :key="f.id" class="flag"
        :class="{ on: active === f.id }"
        @click="active = f.id"
      >
        <span class="fico" :class="'grad-' + f.grad"><Icon :name="f.icon" :size="17" /></span>
        <span class="ftext">
          <span class="fname">{{ f.id }}</span>
          <span class="fdesc">{{ f.desc }}</span>
        </span>
        <span class="fnum">{{ counts[f.id] ?? '·' }}</span>
      </button>
    </div>

    <div class="bar">
      <span>{{ active }} · {{ activeInfo.desc }}</span>
      <button class="rescan" :disabled="rescanning" @click="reclassify">
        {{ rescanning ? '重新判定中…' : '按新标准重新判定' }}
      </button>
      <button class="rescan" @click="showStandard = !showStandard">
        {{ showStandard ? '收起识别标准' : '识别标准' }}
      </button>
    </div>

    <!-- 识别标准（原在设置页，挪到这里更贴近使用场景） -->
    <section v-if="showStandard" class="standard card">
      <div class="std-head">
        <strong>识别标准</strong>
        <span>决定什么照片会被标成模糊 / 过暗 / 泛黄，一般保持默认即可</span>
      </div>
      <div class="std-fields">
        <label v-for="f in FIELDS" :key="f.key" class="std-field">
          <span class="ftitle">{{ f.title }}</span>
          <span class="fdesc">{{ f.desc }}</span>
          <span class="finput">
            <input type="number" :step="f.step" :min="f.min" :max="f.max"
                   v-model="cfgVals[f.key]" placeholder="默认" />
            <i v-if="f.unit">{{ f.unit }}</i>
          </span>
        </label>
      </div>
      <div class="std-actions">
        <button class="btn btn-primary" :disabled="savingCfg" @click="saveStandard">
          {{ savingCfg ? '保存中…' : '保存识别标准' }}
        </button>
        <button class="btn btn-ghost" @click="reloadStandard">还原</button>
        <span class="std-hint">{{ cfgMsg || '改完需要重新「开始扫描」才会作用到已有照片' }}</span>
      </div>
    </section>

    <PhotosPanel
      :key="panelKey"
      :query="query"
      :page-size="120"
      show-repair
      :empty-title="`这个类别下没有照片`"
      empty-hint="换个类别看看，或在设置里调整识别标准"
      @repair="emit('repair', $event)"
    />
  </div>
</template>

<style scoped>
.wrap { height: 100%; overflow-y: auto; display: flex; flex-direction: column; gap: 12px; padding-bottom: 24px; }
.flags { display: flex; gap: 10px; flex-wrap: wrap; flex: none; }
.flag {
  display: flex; align-items: center; gap: 10px;
  padding: 9px 14px 9px 9px; border-radius: var(--r-md);
  background: var(--surface); box-shadow: var(--shadow-sm);
  transition: box-shadow var(--dur) var(--ease), transform var(--dur) var(--ease);
}
.flag:hover { transform: translateY(-2px); }
.flag.on { box-shadow: var(--shadow-md), 0 0 0 2px var(--tint-strong); }
.fico { display: grid; place-items: center; width: 30px; height: 30px; border-radius: var(--r-xs); color: #fff; }
.grad-blue { background: var(--grad-blue) } .grad-violet { background: var(--grad-violet) }
.grad-pink { background: var(--grad-pink) } .grad-amber { background: var(--grad-amber) }
.grad-green { background: var(--grad-green) }
.ftext { display: flex; flex-direction: column; gap: 1px; }
.fname { font-size: var(--fs-sm); font-weight: 600; }
.fdesc { font-size: 10px; color: var(--text-tertiary); }
.fnum { font-size: var(--fs-sm); color: var(--text-tertiary); margin-left: 4px; }
.bar { flex: none; display: flex; align-items: center; gap: 12px; font-size: var(--fs-sm); color: var(--text-tertiary); }
.rescan {
  margin-left: auto; padding: 6px 14px; border-radius: var(--r-pill);
  background: var(--surface-2); color: var(--text-secondary); font-size: var(--fs-xs);
}
.rescan:hover { background: var(--surface-3); color: var(--text-primary); }
.rescan:disabled { opacity: .6; }

/* 识别标准面板 */
.standard { flex: none; padding: 15px 18px; margin-bottom: 2px; }
.std-head { display: flex; flex-direction: column; gap: 2px; margin-bottom: 12px; }
.std-head strong { font-size: var(--fs-base); font-weight: 600; }
.std-head span { font-size: var(--fs-xs); color: var(--text-tertiary); }
.std-fields { display: grid; grid-template-columns: repeat(auto-fit, minmax(300px, 1fr)); gap: 11px 28px; }
.std-field { display: grid; grid-template-columns: 1fr auto; gap: 1px 12px; align-items: center; }
.ftitle { font-size: var(--fs-sm); font-weight: 500; color: var(--text-primary); }
.fdesc { grid-column: 1; font-size: var(--fs-xs); color: var(--text-tertiary); }
.finput { grid-column: 2; grid-row: 1 / span 2; display: flex; align-items: center; gap: 5px; }
.finput input {
  width: 88px; height: 32px; padding: 0 9px; border-radius: var(--r-xs);
  background: var(--surface-2); font-size: var(--fs-sm);
  color: var(--text-primary); text-align: right;
}
.finput input:focus { box-shadow: 0 0 0 2px var(--tint-strong); }
.finput i { font-style: normal; font-size: 10px; color: var(--text-tertiary); }
.std-actions { display: flex; flex-wrap: wrap; align-items: center; gap: 10px; margin-top: 13px; }
.std-hint { font-size: var(--fs-xs); color: var(--text-tertiary); }
</style>
