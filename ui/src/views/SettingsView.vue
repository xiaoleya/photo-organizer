<script setup>
/* 设置：照片目录（含清除）、实验室功能、维护。
   识别标准已移到「质量待修」页面里，这里不再重复。 */
import { ref, onMounted } from 'vue'
import Icon from '../components/Icon.vue'
import { api, native, waitForBackend } from '../api'
import { theme, accent, THEMES, ACCENTS, setTheme, setAccent } from '../theme'

const roots = ref([])
const msg = ref('')
const msgType = ref('info')
const busy = ref(false)
const ready = ref(false)
const stats = ref(null)
const saving = ref(false)
const resetting = ref(false)
const cfg = ref({})

/* 实验室功能：只有「质量待修」「照片修复」两项需要开关，
   其余功能（重复检测、地点、收藏、整理归档）默认常驻，不再提供开关 */
const LAB_ITEMS = [
  {
    key: 'lab_quality', icon: 'quality', grad: 'violet',
    title: '质量待修',
    desc: '自动判定模糊、过暗、过亮、褪色、泛黄、噪点等问题照片',
    need: '关闭时扫描照片不会做质量判定，「质量待修」页面也不会出现',
  },
  {
    key: 'lab_restore', icon: 'restore', grad: 'pink',
    title: '照片修复',
    desc: '去黄、去划痕、自动色阶、降噪、锐化等增强处理',
    need: '关闭时「去修复」按钮会提示未启用',
  },
]

onMounted(async () => {
  ready.value = await waitForBackend(30000)
  if (!ready.value) {
    setMsg('处理引擎未响应。确认 backend 文件夹与主程序在同一目录', 'err')
    return
  }
  await loadAll()
  window.addEventListener('opo:index-reset', loadStats)
  window.addEventListener('opo:roots-changed', loadRoots)
})

async function loadAll() {
  await Promise.all([loadRoots(), loadStats(), loadSettings()])
}

async function loadRoots() {
  const r = await api.roots()
  if (r && r.items) roots.value = r.items
}
async function loadStats() {
  stats.value = await api.stats()
}
async function loadSettings() {
  const r = await api.settings()
  const s = (r && r.settings) || {}
  const next = { ...cfg.value }
  for (const item of LAB_ITEMS) next[item.key] = !!s[item.key]
  cfg.value = next
}

function setMsg(text, type = 'info') { msg.value = text; msgType.value = type }

async function toggleLab(item) {
  const next = { ...cfg.value, [item.key]: !cfg.value[item.key] }
  cfg.value = next
  saving.value = true
  const r = await api.saveSettings({ [item.key]: next[item.key] })
  saving.value = false
  if (!r || !r.ok) {
    cfg.value = { ...cfg.value, [item.key]: !next[item.key] }
    setMsg('保存失败：' + (r?.error || '后端无响应'), 'err')
    return
  }
  setMsg(
    next[item.key]
      ? `已开启「${item.title}」，请点「开始扫描」重新扫描照片后生效`
      : `已关闭「${item.title}」`,
    next[item.key] ? 'warn' : 'ok')
  window.dispatchEvent(new CustomEvent('opo:lab-changed'))
}

/* ---------------- 照片目录（含清除扫描记录） ---------------- */
async function pickAndAdd() {
  const dir = await native.pickFolder()
  if (!dir) return
  const r = await api.addRoot(dir)
  if (r && r.ok) {
    roots.value = r.roots || roots.value
    setMsg('已添加：' + dir + '，点「开始扫描」建立索引', 'ok')
    // 通知顶部路径徽标同步
    window.dispatchEvent(new CustomEvent('opo:roots-changed'))
  } else {
    setMsg('添加失败：' + (r?.error || '后端无响应'), 'err')
  }
}

async function removeRoot(path) {
  if (!confirm('从列表移除这个目录？只移除记录，不会删除任何文件。')) return
  const r = await api.removeRoot(path)
  if (r && r.ok) {
    roots.value = (r.roots || []).filter(x => x !== path)
    setMsg('已移除：' + path, 'ok')
    // 通知顶部路径徽标同步
    window.dispatchEvent(new CustomEvent('opo:roots-changed'))
  } else {
    setMsg('移除失败：' + (r?.error || '后端无响应'), 'err')
  }
}

async function openFolder(p) { await native.openPath(p) }

/* 清除扫描记录：不动原始照片文件 */
async function resetIndex(alsoRoots) {
  const text = alsoRoots
    ? '将清除所有扫描记录，并连照片目录一起移除（原始照片文件不受影响）。继续？'
    : '将清除所有扫描记录、缩略图缓存和标签（原始照片文件不受影响），清空后需要重新「开始扫描」。继续？'
  if (!confirm(text)) return
  resetting.value = true
  setMsg('正在清理…')
  const r = await api.resetIndex(!alsoRoots)
  resetting.value = false
  if (r && r.ok) {
    stats.value = null
    await loadAll()
    window.dispatchEvent(new CustomEvent('opo:index-reset'))
    setMsg('已清除 ' + r.removed + ' 条扫描记录' +
      (alsoRoots ? '，目录也一并移除' : '，目录已保留，点「开始扫描」可重建'), 'ok')
  } else {
    setMsg('清理失败：' + (r?.error || '后端无响应'), 'err')
  }
}

/* ---------------- 开始扫描 ---------------- */
async function runScan() {
  if (!roots.value.length) { setMsg('请先添加照片目录', 'warn'); return }
  busy.value = true
  setMsg('正在扫描，请稍候…')
  const r = await api.scanStart(roots.value)
  if (!r || !r.ok) {
    busy.value = false
    setMsg('启动失败：' + (r?.error || '后端无响应'), 'err')
    return
  }
  const timer = setInterval(async () => {
    const p = await api.progress()
    if (!p) return
    setMsg(`${p.phase} ${p.total ? p.done + '/' + p.total : ''} · ${p.detail}`, 'info')
    if (!p.running) {
      clearInterval(timer)
      busy.value = false
      await loadStats()
      if (p.error) setMsg('扫描失败：' + p.error, 'err')
      else setMsg('扫描完成：共 ' + (stats.value?.total || 0) + ' 张照片', 'ok')
      window.dispatchEvent(new CustomEvent('opo:scan-done'))
    }
  }, 700)
}

function fmt(n) {
  if (!n) return '0 B'
  const u = ['B', 'KB', 'MB', 'GB']; let i = 0; n = Number(n)
  while (n >= 1024 && i < 3) { n /= 1024; i++ }
  return n.toFixed(1) + ' ' + u[i]
}
</script>

<template>
  <div class="wrap">
    <section v-if="stats" class="card block summary">
      <div class="sum-item"><span>照片总数</span><strong>{{ stats.total }}</strong></div>
      <div class="sum-item"><span>已扫描</span><strong>{{ stats.analyzed }}</strong></div>
      <div class="sum-item"><span>有拍摄时间</span><strong>{{ stats.with_dt }}</strong></div>
      <div class="sum-item"><span>重复照片</span><strong>{{ stats.dups }}</strong></div>
      <div class="sum-item"><span>占用空间</span><strong>{{ fmt(stats.bytes) }}</strong></div>
    </section>

    <!-- 照片目录 + 清除扫描记录（合并到一块） -->
    <section class="card block">
      <header>
        <span class="bico grad-blue"><Icon name="folder" :size="18" /></span>
        <div class="ht">
          <strong>照片目录</strong>
          <p>选择照片所在文件夹，再点「开始扫描」建立索引</p>
        </div>
      </header>

      <div class="roots">
        <div v-for="r in roots" :key="r" class="root">
          <Icon name="folder" :size="16" />
          <span class="ellipsis">{{ r }}</span>
          <button class="mini" title="在资源管理器中打开" @click="openFolder(r)">
            <Icon name="organize" :size="15" />
          </button>
          <button class="mini danger" title="从列表移除（不删除文件）" @click="removeRoot(r)">
            <Icon name="trash" :size="15" />
          </button>
        </div>
        <div v-if="!roots.length" class="noroot">还没有添加任何目录</div>
      </div>

      <div class="actions">
        <button class="btn btn-ghost" @click="pickAndAdd">
          <Icon name="folder" :size="16" /> 选择文件夹
        </button>
        <button class="btn btn-primary" :disabled="busy" @click="runScan">
          <Icon name="sparkle" :size="16" /> {{ busy ? '扫描中…' : '开始扫描' }}
        </button>
      </div>

      <div class="divider-line"></div>

      <div class="sub-head">
        <strong>清除扫描记录</strong>
        <span>只清索引、缩略图缓存和标签，<b>不会删除原始照片</b></span>
      </div>
      <div class="actions">
        <button class="btn btn-danger" :disabled="busy || resetting" @click="resetIndex(false)">
          {{ resetting ? '清理中…' : '清除扫描记录' }}
        </button>
        <button class="btn btn-ghost" :disabled="busy || resetting" @click="resetIndex(true)">
          连目录一起清除
        </button>
      </div>

      <div v-if="msg" class="msg" :class="msgType">{{ msg }}</div>
    </section>

    <!-- 外观主题 -->
    <section class="card block">
      <header>
        <span class="bico grad-accent"><Icon name="palette" :size="18" /></span>
        <div class="ht">
          <strong>外观主题</strong>
          <p>选择深浅模式和强调色，立即生效并自动记住</p>
        </div>
      </header>

      <div class="sub-head">
        <strong>深浅模式</strong>
        <span>切换整体明暗</span>
      </div>
      <div class="theme-list">
        <button
          v-for="t in THEMES" :key="t.id"
          class="theme-card" :class="{ on: theme === t.id }"
          @click="setTheme(t.id)"
        >
          <span class="theme-preview" :class="'preview-' + t.id">
            <span class="pv-bar"></span>
            <span class="pv-body"><i></i><i></i><i></i></span>
          </span>
          <span class="theme-name">{{ t.label }}</span>
          <Icon v-if="theme === t.id" name="check" :size="15" />
        </button>
      </div>

      <div class="sub-head">
        <strong>强调色</strong>
        <span>影响按钮、选中项与高亮</span>
      </div>
      <div class="accent-list">
        <button
          v-for="a in ACCENTS" :key="a.id"
          class="accent-item" :class="{ on: accent === a.id }"
          @click="setAccent(a.id)"
        >
          <span class="accent-dot" :style="{ background: a.colors[500] }"></span>
          <span>{{ a.label }}</span>
          <Icon v-if="accent === a.id" name="check" :size="14" />
        </button>
      </div>
    </section>

    <!-- 实验室功能 -->
    <section class="card block">
      <header>
        <span class="bico grad-violet"><Icon name="wand" :size="18" /></span>
        <div class="ht">
          <strong>实验室功能</strong>
          <p>这些功能还在打磨，可能不稳定、出现各种 bug，仅供尝鲜体验</p>
        </div>
      </header>

      <div class="lab-warning">
        这两项还在打磨，可能不稳定、出现各种 bug，仅供尝鲜体验。<br />
        <b>打开或关闭后，请重新点「开始扫描」，否则已有的照片不会带上新结果。</b>
      </div>

      <div class="lab-list">
        <div v-for="item in LAB_ITEMS" :key="item.key" class="lab-item" :class="{ on: cfg[item.key] }">
          <span class="lab-ico" :class="'grad-' + item.grad">
            <Icon :name="item.icon" :size="17" />
          </span>
          <div class="lab-text">
            <div class="lab-title">
              {{ item.title }}
              <span class="lab-state" :class="{ on: cfg[item.key] }">
                {{ cfg[item.key] ? '已启用' : '已关闭' }}
              </span>
            </div>
            <p class="lab-desc">{{ item.desc }}</p>
            <p class="lab-need">{{ item.need }}</p>
          </div>
          <button class="switch" :class="{ on: cfg[item.key] }" :disabled="saving" @click="toggleLab(item)" :title="cfg[item.key] ? '点击关闭' : '点击启用'">
            <span class="knob"></span>
          </button>
        </div>
      </div>
    </section>
  </div>
</template>

<style scoped>
.wrap { height: 100%; overflow-y: auto; display: flex; flex-direction: column; gap: 14px; padding-bottom: 24px; }
.block { padding: 18px 22px; }
.summary { display: grid; grid-template-columns: repeat(auto-fit, minmax(110px, 1fr)); gap: 14px; }
.sum-item { display: flex; flex-direction: column; gap: 3px; }
.sum-item span { font-size: var(--fs-xs); color: var(--text-tertiary); }
.sum-item strong { font-family: var(--font-display); font-size: var(--fs-xl); }
.block header { display: flex; align-items: flex-start; gap: 13px; margin-bottom: 15px; }
.ht { display: flex; flex-direction: column; gap: 3px; }
.ht strong { font-size: var(--fs-lg); font-weight: 600; }
.ht p { margin: 0; font-size: var(--fs-sm); color: var(--text-tertiary); }
.bico { display: grid; place-items: center; width: 36px; height: 36px; flex: none; border-radius: var(--r-sm); color: #fff; }
.grad-blue { background: var(--grad-blue) } .grad-violet { background: var(--grad-violet) }
.grad-pink { background: var(--grad-pink) } .grad-green { background: var(--grad-green) }
.grad-amber { background: var(--grad-amber) }

.roots { display: flex; flex-direction: column; gap: 7px; margin-bottom: 14px; }
.root {
  display: flex; align-items: center; gap: 8px; padding: 10px 13px;
  border-radius: var(--r-sm); background: var(--surface-2);
  font-size: var(--fs-base); color: var(--text-secondary);
}
.root > svg { color: var(--brand-400); flex: none; }
.root .ellipsis { flex: 1; min-width: 0; }
.mini { display: grid; place-items: center; width: 26px; height: 26px; flex: none; border-radius: var(--r-xs); color: var(--text-tertiary); }
.mini:hover { background: var(--surface-3); color: var(--text-primary); }
.mini.danger:hover { background: rgba(239,68,68,.16); color: var(--danger); }
.noroot { padding: 14px 2px; font-size: var(--fs-sm); color: var(--text-tertiary); }
.actions { display: flex; flex-wrap: wrap; align-items: center; gap: 10px; }
.msg { margin-top: 12px; font-size: var(--fs-sm); color: var(--text-tertiary); }
.msg.ok { color: var(--success) } .msg.err { color: var(--danger) } .msg.warn { color: var(--warning) }

.divider-line { height: 1px; background: var(--line); margin: 18px 0 15px; }
.sub-head { display: flex; flex-direction: column; gap: 2px; margin-bottom: 11px; }
.sub-head strong { font-size: var(--fs-base); font-weight: 600; }
.sub-head span { font-size: var(--fs-xs); color: var(--text-tertiary); }
.sub-head b { color: var(--warning); }
.btn-danger { background: rgba(239,68,68,.14); color: var(--danger); }
.btn-danger:hover { background: rgba(239,68,68,.22); }
.btn-danger:disabled { opacity: .6; }

/* 外观主题 */
.grad-accent { background: var(--grad-accent); }
.theme-list { display: flex; gap: 10px; margin-bottom: 15px; }
.theme-card {
  display: flex; align-items: center; gap: 10px;
  min-width: 130px; padding: 10px 12px;
  border-radius: var(--r-md); background: var(--surface-2);
  font-size: var(--fs-sm); color: var(--text-secondary);
  transition: box-shadow var(--dur) var(--ease), background var(--dur) var(--ease);
}
.theme-card:hover { background: var(--surface-3); }
.theme-card.on { box-shadow: inset 0 0 0 2px var(--brand-500); color: var(--brand-300); }
.theme-preview {
  position: relative; display: flex; flex-direction: column; gap: 3px;
  width: 34px; height: 26px; padding: 3px; border-radius: 5px; flex: none;
  border: 1px solid var(--line);
}
.preview-dark { background: #0f1115; }
.preview-light { background: #f4f6fa; }
.preview-dark .pv-bar, .preview-dark .pv-body i { background: #3b95f7; }
.preview-light .pv-bar, .preview-light .pv-body i { background: #3b95f7; }
.theme-preview .pv-bar { height: 4px; border-radius: 3px; }
.theme-preview .pv-body { display: flex; flex-direction: column; gap: 2px; }
.theme-preview .pv-body i { height: 3px; border-radius: 2px; opacity: .65; }
.theme-preview .pv-body i:nth-child(2) { width: 70%; }
.theme-preview .pv-body i:nth-child(3) { width: 45%; }

.accent-list { display: flex; flex-wrap: wrap; gap: 9px; }
.accent-item {
  display: flex; align-items: center; gap: 7px;
  padding: 7px 12px; border-radius: var(--r-pill);
  background: var(--surface-2); font-size: var(--fs-sm);
  color: var(--text-secondary);
}
.accent-item:hover { background: var(--surface-3); }
.accent-item.on { box-shadow: inset 0 0 0 1.5px var(--brand-500); color: var(--brand-300); }
.accent-dot { width: 14px; height: 14px; border-radius: 50%; flex: none; }

/* 实验室功能 */
.lab-warning {
  padding: 11px 14px; margin-bottom: 14px; border-radius: var(--r-sm);
  background: rgba(245,158,11,.10); border: 1px solid rgba(245,158,11,.22);
  font-size: var(--fs-sm); color: var(--text-secondary); line-height: 1.7;
}
.lab-warning b { color: var(--warning); }
.lab-list { display: flex; flex-direction: column; gap: 9px; }
.lab-item {
  display: flex; align-items: flex-start; gap: 12px;
  padding: 13px 15px; border-radius: var(--r-md);
  background: var(--surface-2);
  transition: box-shadow var(--dur) var(--ease), background var(--dur) var(--ease);
}
.lab-item.on { background: var(--tint); box-shadow: inset 0 0 0 1.5px var(--brand-500); }
.lab-ico { display: grid; place-items: center; width: 32px; height: 32px; flex: none; border-radius: var(--r-xs); color: #fff; }
.lab-text { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
.lab-title { display: flex; align-items: center; gap: 8px; font-size: var(--fs-base); font-weight: 600; }
.lab-state { padding: 1px 7px; border-radius: var(--r-pill); background: var(--surface-3); color: var(--text-tertiary); font-size: 10px; font-weight: 500; }
.lab-state.on { background: var(--tint-strong); color: var(--brand-300); }
.lab-desc { margin: 0; font-size: var(--fs-sm); color: var(--text-secondary); }
.lab-need { margin: 0; font-size: var(--fs-xs); color: var(--text-tertiary); }

.switch {
  position: relative; flex: none; width: 42px; height: 24px;
  border-radius: var(--r-pill); background: var(--surface-3);
  transition: background var(--dur) var(--ease);
}
.switch .knob {
  position: absolute; left: 3px; top: 3px; width: 18px; height: 18px;
  border-radius: 50%; background: #fff; transition: transform var(--dur) var(--ease);
}
.switch.on { background: var(--brand-500); }
.switch.on .knob { transform: translateX(18px); }
.switch:disabled { opacity: .6; }
</style>
