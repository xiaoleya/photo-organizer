/* 主题管理：深浅模式 + 品牌色，用 CSS 变量覆盖实现，本地持久化 */
import { ref } from 'vue'

export const THEMES = [
  { id: 'dark', label: '深色', icon: 'moon' },
  { id: 'light', label: '浅色', icon: 'sun' },
]

export const ACCENTS = [
  {
    id: 'blue', label: '蓝色',
    colors: {
      50: '#eef6ff', 100: '#d9ecff', 200: '#b7dbff', 300: '#8ac6ff',
      400: '#5aaefe', 500: '#3b95f7', 600: '#2478e0', 700: '#1b5fbb',
      rgb: '59, 149, 247',
    },
    /* 每种强调色配套的页面氛围：底色色相、环境光颜色。
       切换强调色时整个界面的"气质"跟着变，不只是按钮换色。 */
    mood: { dark: '#0f1115', light: '#f4f6fa', glow: 'rgba(59,149,247,.07)' },
  },
  {
    id: 'cyan', label: '青色',
    colors: {
      50: '#ecfeff', 100: '#cffafe', 200: '#a5f3fc', 300: '#67e8f9',
      400: '#22d3ee', 500: '#06b6d4', 600: '#0891b2', 700: '#0e7490',
      rgb: '6, 182, 212',
    },
    mood: { dark: '#0c1215', light: '#f2f9fa', glow: 'rgba(6,182,212,.07)' },
  },
  {
    id: 'violet', label: '紫色',
    colors: {
      50: '#f5f3ff', 100: '#ede9fe', 200: '#ddd6fe', 300: '#c4b5fd',
      400: '#a78bfa', 500: '#8b5cf6', 600: '#7c3aed', 700: '#6d28d9',
      rgb: '139, 92, 246',
    },
    mood: { dark: '#121017', light: '#f7f5fc', glow: 'rgba(139,92,246,.07)' },
  },
  {
    id: 'orange', label: '橙色',
    colors: {
      50: '#fff7ed', 100: '#ffedd5', 200: '#fed7aa', 300: '#fdba74',
      400: '#fb923c', 500: '#f97316', 600: '#ea580c', 700: '#c2410c',
      rgb: '249, 115, 22',
    },
    mood: { dark: '#141210', light: '#fbf7f2', glow: 'rgba(249,115,22,.07)' },
  },
  {
    id: 'green', label: '绿色',
    colors: {
      50: '#ecfdf5', 100: '#d1fae5', 200: '#a7f3d0', 300: '#6ee7b7',
      400: '#34d399', 500: '#10b981', 600: '#059669', 700: '#047857',
      rgb: '16, 185, 129',
    },
    mood: { dark: '#0d1311', light: '#f2faf7', glow: 'rgba(16,185,129,.07)' },
  },
]

const KEY_THEME = 'opo-theme'
const KEY_ACCENT = 'opo-accent'

export const theme = ref(localStorage.getItem(KEY_THEME) || 'dark')
export const accent = ref(localStorage.getItem(KEY_ACCENT) || 'blue')

function apply() {
  const root = document.documentElement
  root.setAttribute('data-theme', theme.value)
  root.setAttribute('data-accent', accent.value)

  const a = ACCENTS.find(x => x.id === accent.value) || ACCENTS[0]
  const c = a.colors
  const s = `--brand`
  root.style.setProperty(`${s}-50`, c[50])
  root.style.setProperty(`${s}-100`, c[100])
  root.style.setProperty(`${s}-200`, c[200])
  root.style.setProperty(`${s}-300`, c[300])
  root.style.setProperty(`${s}-400`, c[400])
  root.style.setProperty(`${s}-500`, c[500])
  root.style.setProperty(`${s}-600`, c[600])
  root.style.setProperty(`${s}-700`, c[700])
  root.style.setProperty('--grad-accent', `linear-gradient(135deg, ${c[400]} 0%, ${c[600]} 100%)`)
  root.style.setProperty('--shadow-glow', `0 8px 32px rgba(${c.rgb}, .28)`)

  // 深浅主题的 tint 也跟随品牌色
  const isLight = theme.value === 'light'
  root.style.setProperty('--tint', `rgba(${c.rgb}, ${isLight ? .10 : .14})`)
  root.style.setProperty('--tint-strong', `rgba(${c.rgb}, ${isLight ? .18 : .22})`)

  // ---------- 氛围：随强调色改变页面底色与环境光 ----------
  // 切换强调色不只是按钮换色，整个界面的底色调、侧栏色都跟着变，
  // 视觉上的"换主题"感才明显。
  const mood = a.mood || {}
  const darkBase = mood.dark || '#0f1115'
  const lightBase = mood.light || '#f4f6fa'
  const darkRail = shade(darkBase, 6)   // 侧栏比页面更深一点
  const lightRail = tint(lightBase, 4)
  root.style.setProperty('--page-bg-dark', darkBase)
  root.style.setProperty('--page-bg-light', lightBase)
  root.style.setProperty('--rail-bg-dark', darkRail)
  root.style.setProperty('--rail-bg-light', lightRail)
  root.style.setProperty('--ambient-glow', mood.glow || 'transparent')
}

/* 颜色工具：把 hex 变暗 percent% */
function shade(hex, percent) {
  const n = parseInt(hex.slice(1), 16)
  const f = (v) => Math.max(0, Math.min(255, Math.round(v * (100 - percent) / 100)))
  const r = f((n >> 16) & 255), g = f((n >> 8) & 255), b = f(n & 255)
  return `#${((r << 16) | (g << 8) | b).toString(16).padStart(6, '0')}`
}

/* 颜色工具：往白色靠 percent%（模拟提亮） */
function tint(hex, percent) {
  const n = parseInt(hex.slice(1), 16)
  const f = (v) => Math.max(0, Math.min(255, Math.round(v + (255 - v) * percent / 100)))
  const r = f((n >> 16) & 255), g = f((n >> 8) & 255), b = f(n & 255)
  return `#${((r << 16) | (g << 8) | b).toString(16).padStart(6, '0')}`
}

export function setTheme(v) {
  if (!THEMES.some(t => t.id === v)) return
  theme.value = v
  localStorage.setItem(KEY_THEME, v)
  apply()
}

export function setAccent(v) {
  if (!ACCENTS.some(a => a.id === v)) return
  accent.value = v
  localStorage.setItem(KEY_ACCENT, v)
  apply()
}

export function initTheme() { apply() }
