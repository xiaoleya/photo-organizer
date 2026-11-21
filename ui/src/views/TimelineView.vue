<script setup>
/* 时间轴：整页滚动。默认按月展开；月份可按 1→12 或 12→1 排序。 */
import { ref, onMounted } from 'vue'
import Icon from '../components/Icon.vue'
import PhotosPanel from '../components/PhotosPanel.vue'
import { api, waitForBackend } from '../api'

const props = defineProps({
  search: { type: String, default: '' },
})

const years = ref([])
const open = ref({})
const openMonth = ref({})
const loading = ref(true)
const failed = ref(false)
/* 默认按月显示 */
const showMonths = ref(true)
/* 默认 12→1：最近一个月在最上面 */
const monthOrder = ref('desc')

async function load() {
  loading.value = true
  failed.value = false
  const ok = await waitForBackend(20000)
  if (!ok) { loading.value = false; failed.value = true; return }
  const r = await api.timeline()
  loading.value = false
  if (r && r.items) {
    years.value = r.items
    if (r.items.length && !Object.keys(open.value).length) {
      open.value = { [r.items[0].year]: true }
    }
  } else {
    failed.value = true
  }
}

onMounted(load)

function toggle(y) {
  open.value = { ...open.value, [y]: !open.value[y] }
}
function isMonthOpen(y, m) {
  return openMonth.value[y + '-' + m] !== false
}
function toggleMonth(y, m) {
  openMonth.value = { ...openMonth.value, [y + '-' + m]: !isMonthOpen(y, m) }
}
function toggleMonths() { showMonths.value = !showMonths.value }
function toggleOrder() { monthOrder.value = monthOrder.value === 'desc' ? 'asc' : 'desc' }

function monthsOf(y) {
  const list = (y.months || []).slice()
  list.sort((a, b) => monthOrder.value === 'desc' ? b.month - a.month : a.month - b.month)
  return list
}

const queryFor = (extra) => {
  const q = { ...extra, sort: monthOrder.value === 'desc' ? 'dt DESC' : 'dt ASC' }
  if (props.search) q.q = props.search
  return q
}
</script>

<template>
  <div class="wrap">
    <div class="head">
      <span class="summary">
        <template v-if="years.length">
          {{ years.length }} 年 · {{ years.reduce((s, y) => s + y.count, 0) }} 张有日期的照片
        </template>
        <template v-else-if="loading">正在读取时间轴…</template>
        <template v-else-if="failed">读取失败，点右侧重试</template>
        <template v-else>还没有带日期的照片</template>
      </span>
      <button class="toggle" :class="{ on: showMonths }" @click="toggleMonths">
        <span class="dot"></span>{{ showMonths ? '隐藏月份' : '显示月份' }}
      </button>
      <button class="toggle" :class="{ on: true }" @click="toggleOrder" :title="monthOrder === 'desc' ? '当前：最近的月份在前' : '当前：最早的月份在前'">
        <Icon name="sortDown" :size="14" />
        {{ monthOrder === 'desc' ? '月份 12 → 1' : '月份 1 → 12' }}
      </button>
      <button class="toggle" @click="load">刷新</button>
    </div>

    <div v-if="!years.length && !loading" class="empty">
      <Icon name="clock" :size="30" />
      <h3>{{ failed ? '时间轴读取失败' : '还没有带日期的照片' }}</h3>
      <p>{{ failed ? '点上方「刷新」重试，或先运行一次「开始识别」' : '运行一次识别后，照片会按年份排成时间轴' }}</p>
    </div>

    <div v-else class="timeline">
      <section v-for="y in years" :key="y.year" class="year">
        <button class="year-head" @click="toggle(y.year)">
          <span class="year-badge">{{ y.year }}</span>
          <span class="year-info">
            <strong>{{ y.year }} 年</strong>
            <span>{{ y.count }} 张照片 · {{ y.months.length }} 个月</span>
          </span>
          <Icon :name="open[y.year] ? 'chevronDown' : 'chevronRight'" :size="18" />
        </button>

        <div v-if="open[y.year]" class="year-body">
          <template v-if="showMonths">
            <div v-for="m in monthsOf(y)" :key="m.month" class="month-block">
              <button class="month-head" @click="toggleMonth(y.year, m.month)">
                <strong>{{ y.year }} 年 {{ m.month }} 月</strong>
                <span>{{ m.count }} 张</span>
                <Icon :name="isMonthOpen(y.year, m.month) ? 'chevronDown' : 'chevronRight'" :size="14" />
              </button>
              <div v-if="isMonthOpen(y.year, m.month)" class="month-body">
                <PhotosPanel
                  :query="queryFor({ year: y.year, month: m.month })"
                  :page-size="60"
                  :show-hint="false"
                />
              </div>
            </div>
          </template>
          <PhotosPanel
            v-else
            :query="queryFor({ year: y.year })"
            :page-size="60"
            :show-hint="false"
          />
        </div>
      </section>
    </div>
  </div>
</template>

<style scoped>
.wrap { height: 100%; overflow-y: auto; padding-bottom: 24px; }
.head {
  display: flex; flex-wrap: wrap; align-items: center; gap: 10px;
  margin-bottom: 12px; font-size: var(--fs-sm); color: var(--text-tertiary);
}
.summary { flex: 1; min-width: 160px; }
.toggle {
  display: inline-flex; align-items: center; gap: 7px;
  padding: 7px 14px; border-radius: var(--r-pill);
  background: var(--surface); box-shadow: var(--shadow-sm);
  font-size: var(--fs-sm); color: var(--text-secondary);
}
.toggle:hover { background: var(--surface-3); color: var(--text-primary); }
.toggle.on { background: var(--tint-strong); color: var(--brand-300); }
.toggle .dot { width: 7px; height: 7px; border-radius: 50%; background: currentColor; opacity: .7; }
.empty {
  display: flex; flex-direction: column; align-items: center;
  justify-content: center; padding: 80px 0; gap: 10px; color: var(--text-tertiary);
}
.empty h3 { margin: 0; color: var(--text-secondary); }
.empty p { margin: 0; font-size: var(--fs-sm); }
.timeline { display: flex; flex-direction: column; gap: 12px; }
.year { background: var(--surface); border-radius: var(--r-xl); box-shadow: var(--shadow-md); overflow: hidden; }
.year-head { display: flex; width: 100%; align-items: center; gap: 14px; padding: 15px 20px; text-align: left; }
.year-head:hover { background: var(--surface-hover); }
.year-badge {
  display: grid; place-items: center; min-width: 68px; height: 44px;
  padding: 0 10px; border-radius: var(--r-md);
  background: var(--grad-blue); color: #fff;
  font-family: var(--font-display); font-size: var(--fs-md); font-weight: 700;
}
.year-info { flex: 1; display: flex; flex-direction: column; }
.year-info strong { font-size: var(--fs-md); }
.year-info span { font-size: var(--fs-xs); color: var(--text-tertiary); }
.year-body { padding: 0 16px 16px; }
.month-block { border-top: 1px solid var(--line); padding-top: 8px; margin-bottom: 8px; }
.month-head { display: flex; width: 100%; align-items: center; gap: 10px; padding: 6px 2px; text-align: left; }
.month-head strong { font-size: var(--fs-base); }
.month-head span { font-size: var(--fs-xs); color: var(--text-tertiary); }
.month-head svg { margin-left: auto; color: var(--text-tertiary); }
.month-body { padding-bottom: 6px; }
</style>
