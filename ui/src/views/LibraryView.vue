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
  { label: '照片总数', value: '0', icon: 'photos', grad: 'blue' },
  { label: '已定年份', value: '0', icon: 'clock', grad: 'violet' },
  { label: '重复照片', value: '0', icon: 'duplicate', grad: 'pink' },
  { label: '已识别地点', value: '0', icon: 'mapPin', grad: 'amber' },
])

const filters = [
  { id: 'all', label: '全部' },
  { id: 'dup', label: '仅重复' },
  { id: 'nodate', label: '缺日期' },
  { id: 'blur', label: '模糊' },
  { id: 'faded', label: '褪色' },
  { id: 'sepia', label: '泛黄' },
  { id: 'dark', label: '过暗' },
  { id: 'noisy', label: '噪点' },
]
const activeFilter = ref('all')
const sort = ref('时间 ↑')
const SORTS = ['时间 ↑', '时间 ↓', '质量 ↓', '质量 ↑', '年代感 ↓']
const photosTotal = ref(0)
const panelKey = ref(0)

const query = computed(() => {
  const q = {}
  if (activeFilter.value === 'dup') q.dup = 'only'
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
  <div class="wrap">
    <section class="stats">
      <article v-for="s in stats" :key="s.label" class="stat" :class="'stat-' + s.grad">
        <div class="stat-ico"><Icon :name="s.icon" :size="19" /></div>
        <div class="stat-label">{{ s.label }}</div>
        <div class="stat-value">{{ s.value }}</div>
      </article>
    </section>

    <section class="toolbar">
      <div class="chips">
        <button
          v-for="f in filters" :key="f.id" class="pill"
          :class="{ 'is-active': activeFilter === f.id }"
          @click="activeFilter = f.id"
        >{{ f.label }}</button>
      </div>
      <div class="toolbar-right">
        <div class="sort">
          <span>排序</span>
          <select v-model="sort">
            <option v-for="s in SORTS" :key="s">{{ s }}</option>
          </select>
          <Icon name="chevronDown" :size="15" />
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
      empty-title="没有符合条件的照片"
      empty-hint="换个筛选条件，或先点右上角「选择文件夹」再「开始扫描」"
      @count="photosTotal = $event"
    />
  </div>
</template>

<style scoped>
.wrap { height: 100%; overflow-y: auto; display: flex; flex-direction: column; gap: 12px; padding-bottom: 24px; }
.stats { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); gap: 14px; flex: none; }
.stat {
  position: relative; overflow: hidden; border-radius: var(--r-lg);
  padding: 16px 18px 15px; color: #fff; box-shadow: var(--shadow-md);
}
.stat::after {
  content: ''; position: absolute; inset: 0; pointer-events: none;
  background: radial-gradient(120% 90% at 15% 0%, rgba(255,255,255,.32), transparent 62%);
}
.stat-blue { background: var(--grad-blue) } .stat-violet { background: var(--grad-violet) }
.stat-pink { background: var(--grad-pink) } .stat-amber { background: var(--grad-amber) }
.stat-ico {
  position: relative; z-index: 1; display: grid; place-items: center;
  width: 34px; height: 34px; margin-bottom: 10px;
  border-radius: var(--r-sm); background: rgba(255,255,255,.22);
}
.stat-label { position: relative; z-index: 1; font-size: var(--fs-sm); color: rgba(255,255,255,.82); }
.stat-value {
  position: relative; z-index: 1; margin-top: 2px;
  font-family: var(--font-display); font-size: var(--fs-2xl);
  font-weight: 700; line-height: 1.1;
}
.toolbar { display: flex; flex: none; align-items: center; justify-content: space-between; gap: 16px; }
.chips { display: flex; gap: 8px; flex-wrap: wrap; }
.toolbar-right { display: flex; align-items: center; gap: 10px; }
.sort {
  position: relative; display: flex; align-items: center; gap: 7px;
  height: 32px; padding: 0 12px; border-radius: var(--r-pill);
  background: var(--surface); box-shadow: var(--shadow-sm);
  font-size: var(--fs-sm); color: var(--text-secondary);
}
.sort select {
  appearance: none; border: 0; background: none; font-size: var(--fs-sm);
  font-weight: 600; color: var(--text-primary); cursor: pointer; outline: none; padding-right: 4px;
}
.sort select option { background: var(--surface-2); color: var(--text-primary); }
.tagline { flex: none; }
.tag-chip {
  display: inline-flex; align-items: center; gap: 6px;
  padding: 5px 12px; border-radius: var(--r-pill);
  background: var(--tint-strong); color: var(--brand-300); font-size: var(--fs-xs);
}
</style>
