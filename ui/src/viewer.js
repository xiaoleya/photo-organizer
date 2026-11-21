/* 大图查看器：全局单例，任何页面点照片都能打开。
   支持滚轮/按钮/键盘缩放、拖拽平移、左右切换、Esc 关闭、跳转修复。 */
import { reactive } from 'vue'
import { api } from './api'
import { photoUrl } from './thumb'

const MIN_ZOOM = 1
const MAX_ZOOM = 6

const state = reactive({
  visible: false,
  row: null,
  list: [],
  index: 0,
  tags: [],
  loading: false,
  showOriginal: false,
  zoom: 1,
  offsetX: 0,
  offsetY: 0,
  dragX: 0,
  dragY: 0,
  dragging: false,
  gotoRestore: null,
})

function resetZoom() {
  state.zoom = 1
  state.offsetX = 0
  state.offsetY = 0
}

export function zoomBy(delta) {
  const next = Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, state.zoom * delta))
  state.zoom = Math.round(next * 100) / 100
  if (state.zoom <= 1) { state.offsetX = 0; state.offsetY = 0 }
}

export function setZoom(v) {
  state.zoom = Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, v))
  if (state.zoom <= 1) { state.offsetX = 0; state.offsetY = 0 }
}

export function startDrag(e) {
  if (state.zoom <= 1) return
  state.dragging = true
  state.dragX = e.clientX
  state.dragY = e.clientY
}

export function onDrag(e) {
  if (!state.dragging || state.zoom <= 1) return
  const dx = e.clientX - state.dragX
  const dy = e.clientY - state.dragY
  state.dragX = e.clientX
  state.dragY = e.clientY
  state.offsetX += dx
  state.offsetY += dy
}

export function endDrag() {
  state.dragging = false
}

function loadDetail() {
  const row = state.list[state.index]
  if (!row) { close(); return }
  state.row = row
  state.tags = []
  state.loading = true
  resetZoom()
  api.photo(row.id).then(r => {
    if (!r || state.row?.id !== row.id) return
    state.row = { ...row, ...r }
    state.tags = r.tags || []
  }).finally(() => {
    if (state.row?.id === row.id) state.loading = false
  })
}

export function openViewer(row, list) {
  if (!row) return
  /* 关键：必须复制一份快照。
     之前直接存了响应式数组的引用，列表随后"加载更多"或筛选会改写它，
     index 就指到了别的照片——这就是"点的和放大的不是同一张"的原因。 */
  const rows = (Array.isArray(list) && list.length ? list : [row])
    .map(p => ({ ...p }))
  let i = rows.findIndex(p => p.id === row.id)
  if (i < 0) { rows.unshift({ ...row }); i = 0 }
  state.list = rows
  state.index = i
  state.visible = true
  state.showOriginal = false
  loadDetail()
}

export function close() {
  state.visible = false
  state.row = null
  state.list = []
  state.tags = []
  state.loading = false
}

export function step(delta) {
  if (!state.list.length) return
  const n = state.list.length
  state.index = (state.index + delta + n) % n
  state.showOriginal = false
  loadDetail()
}

export function toggleOriginal() {
  state.showOriginal = !state.showOriginal
}

/* 把“去修复”交给 App 处理（切换页面 + 选中照片）。
   handler 必须能拿到照片对象；调用后大图查看器会立即关闭。 */
export function requestRestore(handler) {
  state.gotoRestore = handler
}

export function gotoRestore() {
  const row = state.row
  const handler = state.gotoRestore
  close()
  if (row && handler) handler(row)
}

/* 通用图片预览：修复前后对比等场景，同样支持缩放 */
const preview = reactive({
  visible: false,
  src: '',
  title: '',
  zoom: 1,
  offsetX: 0,
  offsetY: 0,
  dragX: 0,
  dragY: 0,
  dragging: false,
})

export function openImage(src, title = '') {
  if (!src) return
  preview.src = src
  preview.title = title
  preview.zoom = 1
  preview.offsetX = 0
  preview.offsetY = 0
  preview.visible = true
}

export function closeImage() {
  preview.visible = false
  preview.src = ''
  preview.title = ''
}

export function previewZoomBy(delta) {
  const next = Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, preview.zoom * delta))
  preview.zoom = Math.round(next * 100) / 100
  if (preview.zoom <= 1) { preview.offsetX = 0; preview.offsetY = 0 }
}

export function previewReset() {
  preview.zoom = 1
  preview.offsetX = 0
  preview.offsetY = 0
}

export function previewStartDrag(e) {
  if (preview.zoom <= 1) return
  preview.dragging = true
  preview.dragX = e.clientX
  preview.dragY = e.clientY
}

export function previewOnDrag(e) {
  if (!preview.dragging || preview.zoom <= 1) return
  preview.offsetX += e.clientX - preview.dragX
  preview.offsetY += e.clientY - preview.dragY
  preview.dragX = e.clientX
  preview.dragY = e.clientY
}

export function previewEndDrag() {
  preview.dragging = false
}

export const previewState = preview

export function markFavorite(fav) {
  if (state.row) state.row = { ...state.row, favorite: fav ? 1 : 0 }
  const row = state.list[state.index]
  if (row) state.list[state.index] = { ...row, favorite: fav ? 1 : 0 }
}

export const viewerState = state
export const viewerImage = () => photoUrl(state.row)
