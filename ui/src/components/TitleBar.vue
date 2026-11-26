<script setup>
/* 自绘标题栏：无边框窗口的拖动、最小化/最大化/关闭，以及状态徽标。
   参考设计：左侧应用图标 + 标题，右侧信息徽标 + 三个控制按钮。 */
import { ref, computed, onMounted, onBeforeUnmount } from 'vue'
import Icon from './Icon.vue'
import { api, inTauri } from '../api'

const props = defineProps({
  folderCount: { type: Number, default: 0 },
  photoCount: { type: Number, default: 0 },
  analyzed: { type: Number, default: 0 },
})

const maximizing = ref(false)
const hover = ref('')

async function win() {
  const t = window.__TAURI_INTERNALS__ || window.__TAURI__
  if (!t) return null
  return t
}

async function invoke(cmd, args = {}) {
  const g = window.__TAURI__
  const internals = window.__TAURI_INTERNALS__
  try {
    if (g && g.core && typeof g.core.invoke === 'function') return await g.core.invoke(cmd, args)
    if (internals && typeof internals.invoke === 'function') return await internals.invoke(cmd, args)
  } catch (e) {
    console.warn('window command failed:', cmd, e)
  }
  return null
}

async function startDrag(e) {
  if (e.button !== 0) return
  // 双击要交给 dblclick 处理，否则两次 mousedown 都会调 start_dragging，
  // 而 Tauri 在最大化状态下拖动会自动还原窗口 ——
  // 表现就是"双击只会变小，不能变大"。
  if (e.detail > 1) return
  await invoke('plugin:window|start_dragging')
}

async function minimize() { await invoke('plugin:window|minimize') }

async function toggleMax() {
  await invoke('plugin:window|toggle_maximize')
  await refreshMax()
}

/* 双击标题栏切换最大化/还原 —— 两个方向都要生效 */
async function onDblClick() {
  await invoke('plugin:window|toggle_maximize')
  await refreshMax()
}

async function close() { await invoke('plugin:window|close') }

async function refreshMax() {
  const v = await invoke('plugin:window|is_maximized')
  if (typeof v === 'boolean') {
    if (v !== maximizing.value) {
      maximizing.value = v
      // 通知外层去掉圆角与边框
      document.documentElement.classList.toggle('app-maximized', v)
    }
  }
}

let timer = null
onMounted(() => {
  refreshMax()
  timer = setInterval(refreshMax, 1200)
})
onBeforeUnmount(() => { if (timer) clearInterval(timer) })

const analyzedPct = computed(() => {
  if (!props.photoCount) return 0
  return Math.round(100 * props.analyzed / props.photoCount)
})
</script>

<template>
  <header class="titlebar" :class="{ maximized: maximizing }"
          @mousedown="startDrag" @dblclick="onDblClick">
    <div class="tb-left">
      <span class="tb-title">照片整理-by小乐</span>
    </div>

    <div class="tb-right" @mousedown.stop>
      <!-- 信息徽标：照片数 + 已扫描进度（对应参考图的磁盘容量条） -->
      <div v-if="photoCount" class="tb-badge" :title="`共 ${photoCount} 张，已扫描 ${analyzed} 张`">
        <Icon name="photos" :size="14" />
        <span class="tb-badge-label">{{ photoCount }} 张</span>
        <span class="tb-badge-bar">
          <span class="tb-badge-fill" :style="{ width: analyzedPct + '%' }"></span>
        </span>
        <span class="tb-badge-sub">已扫描 {{ analyzed }}</span>
      </div>

      <div class="tb-ctrls">
        <button class="tb-btn" title="最小化" @click="minimize"
                @mouseenter="hover='min'" @mouseleave="hover=''">
          <svg width="12" height="12" viewBox="0 0 12 12"><path d="M2 6h8" stroke="currentColor" stroke-width="1.2" /></svg>
        </button>
        <button class="tb-btn" :title="maximizing ? '还原' : '最大化'" @click="toggleMax"
                @mouseenter="hover='max'" @mouseleave="hover=''">
          <svg v-if="!maximizing" width="12" height="12" viewBox="0 0 12 12">
            <rect x="2.2" y="2.2" width="7.6" height="7.6" rx="1.2" stroke="currentColor" stroke-width="1.2" fill="none" />
          </svg>
          <svg v-else width="12" height="12" viewBox="0 0 12 12">
            <rect x="3.2" y="3.2" width="6.2" height="6.2" rx="1" stroke="currentColor" stroke-width="1.1" fill="none" />
            <path d="M2 8.2V2.6a.8.8 0 0 1 .8-.8h5.4" stroke="currentColor" stroke-width="1.1" fill="none" />
          </svg>
        </button>
        <button class="tb-btn tb-close" title="关闭" @click="close"
                @mouseenter="hover='close'" @mouseleave="hover=''">
          <svg width="12" height="12" viewBox="0 0 12 12">
            <path d="M2.4 2.4l7.2 7.2M9.6 2.4L2.4 9.6" stroke="currentColor" stroke-width="1.2" />
          </svg>
        </button>
      </div>
    </div>
  </header>
</template>

<style scoped>
/* 无边框窗口的自绘标题栏：高度 48px，与下方内容同一底色 */
.titlebar {
  display: flex; align-items: center; justify-content: space-between;
  height: 48px; flex: none; padding: 0 0 0 18px;
  background: var(--page-bg);
  border-radius: 12px 12px 0 0;
  user-select: none;
}
.titlebar.maximized { border-radius: 0; }

.tb-left { display: flex; align-items: center; gap: 9px; min-width: 0; }
.tb-title {
  font-size: 14px; font-weight: 600; color: var(--text-secondary);
  white-space: nowrap; letter-spacing: .01em;
}

.tb-right { display: flex; align-items: center; gap: 12px; flex: none; }

/* 信息徽标：与参考图的磁盘容量条同结构 */
.tb-badge {
  display: flex; align-items: center; gap: 8px;
  height: 26px; padding: 0 12px; border-radius: 8px;
  background: var(--surface-2); border: 1px solid var(--line);
  font-size: 11px; color: var(--text-tertiary);
}
.tb-badge svg { flex: none; }
.tb-badge-label { color: var(--text-secondary); font-weight: 600; white-space: nowrap; }
.tb-badge-bar {
  width: 90px; height: 3px; flex: none; border-radius: 999px;
  background: var(--surface-3); overflow: hidden;
}
.tb-badge-fill {
  display: block; height: 100%; border-radius: 999px;
  background: var(--brand-400); transition: width .3s var(--ease);
}
.tb-badge-sub { white-space: nowrap; }

.tb-ctrls { display: flex; align-items: center; height: 48px; }
.tb-btn {
  display: grid; place-items: center;
  width: 48px; height: 48px; flex: none;
  border-radius: 0; color: var(--text-tertiary);
  transition: background var(--dur) var(--ease), color var(--dur) var(--ease);
}
.tb-btn:hover { background: var(--surface-hover); color: var(--text-primary); }
.tb-close:hover { background: #e81123; color: #fff; }
.tb-btn:last-child { border-top-right-radius: 12px; }
.titlebar.maximized .tb-btn:last-child { border-top-right-radius: 0; }
</style>
