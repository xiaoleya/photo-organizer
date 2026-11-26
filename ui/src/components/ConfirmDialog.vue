<script setup>
/* 统一确认弹窗。替代原生 confirm()，因为需要：
   - 危险操作的分级警告（普通删除 vs 永久删除）
   - 与界面一致的视觉，而不是系统灰框
   - 能显示附带的详情行（文件数、大小等）
*/
import { computed } from 'vue'
import Icon from './Icon.vue'

const props = defineProps({
  show: { type: Boolean, default: false },
  title: { type: String, default: '确认操作' },
  /* 主要说明 */
  message: { type: String, default: '' },
  /* 附带的详情行，如「共 12 张 · 86.4 MB」 */
  detail: { type: String, default: '' },
  /* 危险提示（红底），用于永久删除这类不可恢复的操作 */
  warn: { type: String, default: '' },
  confirmText: { type: String, default: '确定' },
  cancelText: { type: String, default: '取消' },
  /* danger：红色主按钮；normal：蓝色 */
  level: { type: String, default: 'danger' },
  /* 左上角图标名。不传时按语义自动选 */
  icon: { type: String, default: '' },
  busy: { type: Boolean, default: false },
})

/* 图标：显式指定优先；否则按操作语义猜 */
const iconShown = computed(() => {
  if (props.icon) return props.icon
  return props.level === 'danger' ? 'trash' : 'helpCircle'
})


const emit = defineEmits(['confirm', 'cancel'])
</script>

<template>
  <teleport to="body">
    <div v-if="show" class="cfm-mask" @click.self="!busy && emit('cancel')">
      <div class="cfm-box" :class="level">
        <div class="cfm-head">
          <span class="cfm-ico" :class="level">
            <Icon :name="iconShown" :size="19" />
          </span>
          <strong>{{ title }}</strong>
        </div>

        <p v-if="message" class="cfm-msg">{{ message }}</p>
        <p v-if="detail" class="cfm-detail">{{ detail }}</p>

        <div v-if="warn" class="cfm-warn">
          <Icon name="helpCircle" :size="15" />
          <span>{{ warn }}</span>
        </div>

        <div class="cfm-acts">
          <button class="cfm-btn" :disabled="busy" @click="emit('cancel')">
            {{ cancelText }}
          </button>
          <button
            class="cfm-btn primary" :class="level"
            :disabled="busy" @click="emit('confirm')"
          >
            {{ busy ? '处理中…' : confirmText }}
          </button>
        </div>
      </div>
    </div>
  </teleport>
</template>

<style scoped>
.cfm-mask {
  position: fixed; inset: 0; z-index: 4000;
  display: grid; place-items: center;
  background: rgba(0, 0, 0, .55);
  backdrop-filter: blur(2px);
  animation: cfm-fade .14s ease;
}
@keyframes cfm-fade { from { opacity: 0 } to { opacity: 1 } }

.cfm-box {
  width: min(420px, 90vw);
  padding: 22px 24px 18px;
  border-radius: var(--r-xl);
  background: var(--surface);
  box-shadow: var(--shadow-lg);
  animation: cfm-pop .16s var(--ease, ease);
}
@keyframes cfm-pop {
  from { opacity: 0; transform: scale(.96) translateY(6px) }
  to { opacity: 1; transform: none }
}

.cfm-head { display: flex; align-items: center; gap: 11px; margin-bottom: 12px; }
.cfm-head strong { font-size: var(--fs-lg); font-weight: 600; }
.cfm-ico {
  display: grid; place-items: center;
  width: 34px; height: 34px; flex: none;
  border-radius: var(--r-sm);
  color: #fff;
}
.cfm-ico.danger { background: rgba(239,68,68,.9); }
.cfm-ico.normal { background: var(--brand-500); }

.cfm-msg {
  margin: 0 0 6px;
  font-size: var(--fs-base); line-height: 1.65;
  color: var(--text-secondary);
}
.cfm-detail {
  margin: 0 0 12px;
  font-size: var(--fs-sm); color: var(--text-tertiary);
}

/* 永久删除时的高亮警告块 */
.cfm-warn {
  display: flex; align-items: flex-start; gap: 8px;
  margin-bottom: 16px; padding: 10px 12px;
  border-radius: var(--r-sm);
  background: rgba(239,68,68,.10);
  border-left: 2px solid var(--danger);
  font-size: var(--fs-sm); line-height: 1.6;
  color: var(--danger);
}
.cfm-warn svg { flex: none; margin-top: 2px; }

.cfm-acts { display: flex; justify-content: flex-end; gap: 10px; }
.cfm-btn {
  height: 36px; padding: 0 18px;
  border-radius: var(--r-md);
  font-size: var(--fs-md); font-weight: 600;
  background: var(--surface-2); color: var(--text-secondary);
  transition: background var(--dur) var(--ease), opacity var(--dur) var(--ease);
}
.cfm-btn:hover { background: var(--surface-3); color: var(--text-primary); }
.cfm-btn:disabled { opacity: .55; cursor: default; }
.cfm-btn.primary { color: #fff; }
.cfm-btn.primary.danger { background: var(--danger); }
.cfm-btn.primary.danger:hover { background: #dc2626; }
.cfm-btn.primary.normal { background: var(--brand-500); }
.cfm-btn.primary.normal:hover { background: var(--brand-400); }
</style>
