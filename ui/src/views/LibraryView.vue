<script setup>
import { ref, computed, onMounted, onBeforeUnmount, watch } from 'vue'
import Icon from '../components/Icon.vue'
import PhotosPanel from '../components/PhotosPanel.vue'
import { api } from '../api'

const props = defineProps({
  search: { type: String, default: '' },
  tagFilter: { type: String, default: null },
})
const emit = defineEmits(['clear-tag', 'repair'])

const stats = ref([
  { label: '文件总数', value: '0', icon: 'photos', grad: 'blue' },
  { label: '已定年份', value: '0', icon: 'clock', grad: 'violet' },
  { label: '重复照片', value: '0', icon: 'duplicate', grad: 'pink' },
  { label: '已识别地点', value: '0', icon: 'mapPin', grad: 'amber' },
])

/* 常用筛选：任何时候都有意义 */
const basicFilters = [
  { id: 'all', label: '全部' },
  { id: 'video', label: '视频' },
  { id: 'photo', label: '仅照片' },
  { id: 'people', label: '有人物' },
  { id: 'dup', label: '仅重复' },
  { id: 'nodate', label: '缺日期' },
]

/* 质量类筛选：只有开了「质量待修」实验室功能才会产生这些标记，
   所以只在功能开启时显示，避免用户点了却永远是空结果 */
const qualityFilters = [
  { id: 'blur', label: '模糊' },
  { id: 'faded', label: '褪色' },
  { id: 'sepia', label: '泛黄' },
  { id: 'dark', label: '过暗' },
  { id: 'noisy', label: '噪点' },
]

/* 实验室开关状态：直接读设置，和 App.vue 用同一份数据源 */
const labQuality = ref(false)
async function syncLab() {
  const r = await api.settings()
  labQuality.value = !!(r && r.settings && r.settings.lab_quality)
}
onMounted(() => {
  syncLab()
  window.addEventListener('opo:lab-changed', syncLab)
})
onBeforeUnmount(() => window.removeEventListener('opo:lab-changed', syncLab))

const isQualityActive = computed(() => qualityFilters.some(f => f.id === activeFilter.value))
const activeFilterLabel = computed(() =>
  [...basicFilters, ...qualityFilters].find(f => f.id === activeFilter.value)?.label || ''
)
const activeFilter = ref('all')
const sort = ref('时间 ↑')
const SORTS = ['时间 ↑', '时间 ↓', '质量 ↓', '质量 ↑', '年代感 ↓']
const photosTotal = ref(0)
const panelKey = ref(0)

/* ---------------- 年份区间 ---------------- */
const yearFrom = ref('')
const yearTo = ref('')
const yearRangeOpen = ref(false)
const sortOpen = ref(false)
/* 质量类筛选项默认折叠：只有开了「质量待修」实验室功能才有意义 */
const qualityOpen = ref(false)

/* 输入脏数据（非数字、负值）时忽略，不让它进查询 */
function validYear(v) {
  const n = parseInt(String(v).trim(), 10)
  if (Number.isNaN(n) || n < 1826 || n > 2200) return null
  return n
}

/* 起止写反了自动纠正，避免用户以为筛不出东西 */
const rangeApplied = computed(() => {
  const a = validYear(yearFrom.value)
  const b = validYear(yearTo.value)
  if (a === null && b === null) return null
  if (a !== null && b !== null && a > b) return { from: b, to: a, swapped: true }
  return { from: a, to: b, swapped: false }
})

const rangeLabel = computed(() => {
  const r = rangeApplied.value
  if (!r) return '年份区间'
  if (r.from != null && r.to != null) return `${r.from} – ${r.to}`
  if (r.from != null) return `${r.from} 年起`
  return `至 ${r.to} 年`
})

function clearRange() {
  yearFrom.value = ''
  yearTo.value = ''
}

const query = computed(() => {
  const q = {}
  if (activeFilter.value === 'dup') q.dup = 'only'
  if (activeFilter.value === 'video') q.kind = 'video'
  if (activeFilter.value === 'photo') q.kind = 'photo'
  if (activeFilter.value === 'people') q.has_people = '1'
  if (activeFilter.value === 'nodate') q.nodate = '1'
  if (activeFilter.value === 'blur') q.flag = '模糊'
  if (activeFilter.value === 'faded') q.flag = '褪色平淡'
  if (activeFilter.value === 'sepia') q.flag = '泛黄/棕褐'
  if (activeFilter.value === 'dark') q.flag = '过暗'
  if (activeFilter.value === 'noisy') q.flag = '噪点重'
  q.sort = {
    '时间 ↑': 'dt ASC', '时间 ↓': 'dt DESC',
    '质量 ↓': 'quality_score DESC', '质量 ↑': 'quality_score ASC',
    '年代感 ↓': 'age_score DESC',
  }[sort.value] || 'dt ASC'
  const r = rangeApplied.value
  if (r && r.from != null) q.year_from = r.from
  if (r && r.to != null) q.year_to = r.to
  if (props.search) q.q = props.search
  if (props.tagFilter) q.tag = props.tagFilter
  return q
})

async function loadStats() {
  const st = await api.stats()
  if (!st) return
  stats.value[0].value = (st.total || 0).toLocaleString()
  stats.value[1].value = (st.with_dt || 0).toLocaleString()
  stats.value[2].value = String(st.dups || 0)
  stats.value[3].value = String(st.places || 0)
}

onMounted(() => {
  loadStats()
  // 重置索引 / 扫描完成后，照片库要立即清空或刷新，不能等切页
  window.addEventListener('opo:index-reset', onIndexReset)
  window.addEventListener('opo:scan-done', onIndexReset)
})
onBeforeUnmount(() => {
  window.removeEventListener('opo:index-reset', onIndexReset)
  window.removeEventListener('opo:scan-done', onIndexReset)
})

/* 强制让 PhotosPanel 重新拉取（改 key 会重建组件） */
function onIndexReset() {
  panelKey.value++
  loadStats()
}

watch(() => props.tagFilter, loadStats)
defineExpose({ reload: loadStats })
</script>

<template>
  <div class="page">
    <section class="stats">
      <article v-for="s in stats" :key="s.label" class="stat" :class="'stat-' + s.grad">
        <span class="stat-glow"></span>
        <div class="stat-ico"><Icon :name="s.icon" :size="17" /></div>
        <div class="stat-body">
          <div class="stat-value">{{ s.value }}</div>
          <div class="stat-label">{{ s.label }}</div>
        </div>
      </article>
    </section>

    <section class="toolbar">
      <div class="chips">
        <button
          v-for="f in basicFilters" :key="f.id" class="pill"
          :class="{ 'is-active': activeFilter === f.id }"
          @click="activeFilter = f.id"
        >{{ f.label }}</button>

        <!-- 质量类筛选：只有开启「质量待修」实验室功能后才出现，
             否则这些标记根本不会生成，摆着只会让人困惑 -->
        <div v-if="labQuality" class="quality-wrap" :class="{ open: qualityOpen }">
          <button
            class="pill" :class="{ 'is-active': isQualityActive }"
            @click="qualityOpen = !qualityOpen"
          >
            <Icon name="quality" :size="14" />
            {{ isQualityActive ? activeFilterLabel : '质量筛选' }}
            <Icon :name="qualityOpen ? 'chevronDown' : 'chevronRight'" :size="12" />
          </button>
          <div v-if="qualityOpen" class="quality-pop card">
            <button
              v-for="f in qualityFilters" :key="f.id"
              class="quality-item" :class="{ on: activeFilter === f.id }"
              @click="activeFilter = f.id; qualityOpen = false"
            >
              <span class="q-dot"></span>
              {{ f.label }}
              <Icon v-if="activeFilter === f.id" name="check" :size="13" />
            </button>
          </div>
        </div>
      </div>
      <div class="toolbar-right">
        <!-- 年份区间：点开才展开，不占常驻空间 -->
        <div class="range-wrap">
          <button
            class="pill" :class="{ 'is-active': rangeApplied }"
            @click="yearRangeOpen = !yearRangeOpen"
          >
            <Icon name="calendar" :size="14" />
            {{ rangeLabel }}
            <Icon :name="yearRangeOpen ? 'chevronDown' : 'chevronRight'" :size="12" />
          </button>
          <div v-if="yearRangeOpen" class="range-pop card">
            <div class="range-head">按拍摄年份筛选</div>
            <div class="range-row">
              <input v-model="yearFrom" class="range-input" placeholder="起 (如 1985)" maxlength="4" inputmode="numeric" />
              <span class="range-sep">–</span>
              <input v-model="yearTo" class="range-input" placeholder="止 (如 1990)" maxlength="4" inputmode="numeric" />
            </div>
            <div v-if="rangeApplied && rangeApplied.swapped" class="range-hint">
              起止写反了，已自动对调
            </div>
            <div class="range-acts">
              <button class="btn btn-ghost" @click="clearRange">清空</button>
              <button class="btn btn-primary" @click="yearRangeOpen = false">完成</button>
            </div>
          </div>
        </div>

        <!-- 排序：自定义下拉，避免原生 select 在各平台样式不一 -->
        <div class="sort" :class="{ open: sortOpen }">
          <button class="sort-btn" @click="sortOpen = !sortOpen">
            <Icon name="sortDown" :size="14" />
            <span class="sort-cur">{{ sort }}</span>
            <Icon name="chevronDown" :size="13" class="sort-caret" />
          </button>
          <div v-if="sortOpen" class="sort-pop">
            <button
              v-for="s in SORTS" :key="s"
              class="sort-item" :class="{ on: s === sort }"
              @click="sort = s; sortOpen = false"
            >
              <Icon v-if="s === sort" name="check" :size="13" />
              <span v-else class="sort-dot"></span>
              {{ s }}
            </button>
          </div>
        </div>
      </div>
    </section>



    <div v-if="props.tagFilter" class="tagline">
      <button class="tag-chip" @click="emit('clear-tag')">
        标签：{{ props.tagFilter }} <Icon name="close" :size="12" />
      </button>
    </div>

    <PhotosPanel
      :key="panelKey"
      :query="query"
      :page-size="120"
      selectable
      empty-title="没有符合条件的照片"
      empty-hint="换个筛选条件，或先点右上角「选择文件夹」再「开始扫描」"
      @count="photosTotal = $event"
    />
  </div>
</template>

<style scoped>
/* 根容器/页头/空态已统一到 base.css，这里只保留本页特有的统计卡与工具栏 */
/* ==========================================================================
   统计卡
   --------------------------------------------------------------------------
   原来是 4 块高饱和渐变色块，颜色跳、信息密度低。
   改成：深色表面 + 左侧一道细的品牌色渐变条 + 淡淡的角部光晕。
   数据是主角，颜色只作点缀。
   ========================================================================== */
.stats { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); gap: 12px; flex: none; }

.stat {
  position: relative; overflow: hidden;
  display: flex; align-items: center; gap: 13px;
  border-radius: var(--r-lg);
  padding: 15px 16px;
  background: var(--surface);
  box-shadow: var(--surface-shadow);
  border: 1px solid var(--surface-stroke);
  transition: box-shadow var(--dur) var(--ease), border-color var(--dur) var(--ease),
              transform var(--dur) var(--ease);
  /* 左侧渐变条：每张卡颜色不同，作为唯一的高饱和点缀 */
  --accent: var(--brand-400);
}
.stat::before {
  content: ''; position: absolute; left: 0; top: 0; bottom: 0; width: 3px;
  background: linear-gradient(180deg, var(--accent), transparent);
  opacity: .85;
}
.stat:hover {
  box-shadow: var(--surface-shadow-hover);
  border-color: var(--surface-stroke-hover);
  transform: translateY(-1px);
}

/* 角落光晕：极淡，让卡片有一点温度 */
.stat-glow {
  position: absolute; right: -32px; top: -32px;
  width: 96px; height: 96px; border-radius: 50%;
  background: radial-gradient(circle, var(--accent), transparent 68%);
  opacity: .12; pointer-events: none;
}

.stat-ico {
  position: relative; z-index: 1;
  display: grid; place-items: center;
  width: 34px; height: 34px; flex: none;
  border-radius: 10px;
  background: color-mix(in oklab, var(--accent) 16%, transparent);
  color: var(--accent);
}

.stat-body { position: relative; z-index: 1; min-width: 0; }
.stat-value {
  font-family: var(--font-display);
  font-size: var(--fs-2xl);
  font-weight: 700; line-height: 1.05;
  color: var(--text-primary);
  font-variant-numeric: tabular-nums;
}
.stat-label {
  margin-top: 2px;
  font-size: var(--fs-xs);
  color: var(--text-tertiary);
  white-space: nowrap;
}

/* 每张卡的强调色 */
.stat-blue   { --accent: #4EA9FF; }
.stat-violet { --accent: #A78BFA; }
.stat-pink   { --accent: #F472B6; }
.stat-amber  { --accent: #FBBF24; }
/* ---------- 排序下拉 ---------- */
.sort { position: relative; }
.sort-btn {
  display: flex; align-items: center; gap: 7px;
  height: 30px; padding: 0 11px; border-radius: var(--r-pill);
  background: var(--surface-2);
  border: 1px solid transparent;
  color: var(--text-secondary);
  font-size: var(--fs-sm);
  white-space: nowrap;
  transition: background var(--dur) var(--ease), border-color var(--dur) var(--ease);
}
.sort-btn:hover { background: var(--surface-3); border-color: var(--surface-stroke-hover); }
.sort.open .sort-btn {
  background: var(--surface-3);
  border-color: var(--surface-stroke-hover);
}
.sort-cur { color: var(--text-primary); font-weight: 500; }
.sort-caret { opacity: .55; transition: transform var(--dur) var(--ease); }
.sort.open .sort-caret { transform: rotate(180deg); }

.sort-pop {
  position: absolute; right: 0; top: calc(100% + 7px); z-index: 70;
  min-width: 156px; padding: 5px;
  border-radius: var(--r-md);
  background: var(--surface);
  border: 1px solid var(--line-strong);
  box-shadow: var(--shadow-lg);
  display: flex; flex-direction: column; gap: 1px;
}
.sort-item {
  display: flex; align-items: center; gap: 8px;
  padding: 8px 10px; border-radius: var(--r-xs);
  font-size: var(--fs-sm); color: var(--text-secondary);
  text-align: left; white-space: nowrap;
  transition: background var(--dur) var(--ease), color var(--dur) var(--ease);
}
.sort-item:hover { background: var(--surface-hover); color: var(--text-primary); }
.sort-item.on { color: var(--brand-300); background: var(--tint); }
.sort-dot { width: 13px; height: 13px; flex: none; }

/* ---------- 质量筛选下拉 ---------- */
.quality-wrap { position: relative; }
.quality-pop {
  position: absolute; left: 0; top: calc(100% + 7px); z-index: 70;
  min-width: 132px; padding: 5px;
  border-radius: var(--r-md);
  background: var(--surface);
  border: 1px solid var(--line-strong);
  box-shadow: var(--shadow-lg);
  display: flex; flex-direction: column; gap: 1px;
}
.quality-item {
  display: flex; align-items: center; gap: 8px;
  padding: 8px 10px; border-radius: var(--r-xs);
  font-size: var(--fs-sm); color: var(--text-secondary);
  text-align: left; white-space: nowrap;
  transition: background var(--dur) var(--ease), color var(--dur) var(--ease);
}
.quality-item:hover { background: var(--surface-hover); color: var(--text-primary); }
.quality-item.on { color: var(--brand-300); background: var(--tint); }
.quality-item svg { margin-left: auto; }
.q-dot {
  width: 6px; height: 6px; flex: none; border-radius: 50%;
  background: var(--warning); opacity: .7;
}

.toolbar { display: flex; flex: none; align-items: center; justify-content: space-between; gap: 16px; }.chips { display: flex; gap: 8px; flex-wrap: wrap; }
.toolbar-right { display: flex; align-items: center; gap: 10px; }

/* 年份区间 */
.range-wrap { position: relative; }
.range-pop {
  position: absolute; right: 0; top: calc(100% + 8px); z-index: 60;
  width: 268px; padding: 14px 16px;
  box-shadow: var(--shadow-lg);
}
.range-head { font-size: var(--fs-xs); color: var(--text-tertiary); margin-bottom: 10px; }
.range-row { display: flex; align-items: center; gap: 8px; }
.range-input {
  flex: 1; min-width: 0; height: 34px; padding: 0 10px;
  border-radius: var(--r-sm); background: var(--surface-2);
  border: 1px solid var(--line);
  font-size: var(--fs-sm); color: var(--text-primary);
}
.range-input::placeholder { color: var(--text-tertiary); }
.range-sep { color: var(--text-tertiary); flex: none; }
.range-hint {
  margin-top: 8px; font-size: var(--fs-xs); color: var(--warning);
}
.range-acts { display: flex; justify-content: flex-end; gap: 8px; margin-top: 12px; }
.range-acts .btn { height: 32px; padding: 0 14px; font-size: var(--fs-sm); }

.tagline { flex: none; }
.tag-chip {
  display: inline-flex; align-items: center; gap: 6px;
  padding: 5px 12px; border-radius: var(--r-pill);
  background: var(--tint-strong); color: var(--brand-300); font-size: var(--fs-xs);
}
</style>
