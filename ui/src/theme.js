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
  },
  {
    id: 'cyan', label: '青色',
    colors: {
      50: '#ecfeff', 100: '#cffafe', 200: '#a5f3fc', 300: '#67e8f9',
      400: '#22d3ee', 500: '#06b6d4', 600: '#0891b2', 700: '#0e7490',
      rgb: '6, 182, 212',
    },
  },
  {
    id: 'violet', label: '紫色',
    colors: {
      50: '#f5f3ff', 100: '#ede9fe', 200: '#ddd6fe', 300: '#c4b5fd',
      400: '#a78bfa', 500: '#8b5cf6', 600: '#7c3aed', 700: '#6d28d9',
      rgb: '139, 92, 246',
    },
  },
  {
    id: 'orange', label: '橙色',
    colors: {
      50: '#fff7ed', 100: '#ffedd5', 200: '#fed7aa', 300: '#fdba74',
      400: '#fb923c', 500: '#f97316', 600: '#ea580c', 700: '#c2410c',
      rgb: '249, 115, 22',
    },
  },
  {
    id: 'green', label: '绿色',
    colors: {
      50: '#ecfdf5', 100: '#d1fae5', 200: '#a7f3d0', 300: '#6ee7b7',
      400: '#34d399', 500: '#10b981', 600: '#059669', 700: '#047857',
      rgb: '16, 185, 129',
    },
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
