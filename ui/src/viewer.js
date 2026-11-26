/* 大图查看器：全局单例，任何页面点照片都能打开。
   支持滚轮/按钮/键盘缩放、拖拽平移、左右切换、Esc 关闭、跳转修复。 */
import { reactive } from 'vue'
import { api } from './api'
import { photoUrl, videoUrl } from './thumb'

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
  /* 旋转角度（度）。仅影响显示，不改原文件；切换照片时归零。
     注意：这里刻意【不】取模到 0-359，见 rotateBy 的说明 */
  rotate: 0,
  /* 为 true 时禁用 transform 过渡（切换照片/归位时用，避免长距离倒转） */
  noTransition: false,
})

function resetZoom() {
  state.zoom = 1
  state.offsetX = 0
  state.offsetY = 0
}

/* 每次点击累加 ±90°，不取模。
 *
 * 为什么不能取模：转到 270° 后再点一次，`(270+90) % 360 = 0`，
 * CSS 的 transition 会在 270deg → 0deg 之间**倒着**插值，
 * 屏幕上就是"一下倒转三圈回到原位"，非常突兀。
 * 让数值一直累加（360、450、540…），CSS 里 rotate(360deg) 与
 * rotate(0deg) 视觉完全等价，所以既正确又每次只转 90°。
 *
 * 累加到 ±3600° 时顺手归到等价角度，避免长时间反复点击后数值无限增大。
 */
export function rotateBy(delta) {
  state.rotate += delta
  if (Math.abs(state.rotate) >= 3600) {
    // 归到最近的等价角度，且不触发过渡（数值变化但视觉不变）
    state.noTransition = true
    state.rotate = state.rotate % 360
    requestAnimationFrame(() => {
      requestAnimationFrame(() => { state.noTransition = false })
    })
  }
}

/* 归位：把角度清零。
 * 调用方负责在切换照片时设 `noTransition`，否则从 270° 回 0 会倒着抡。 */
function resetRotate() {
  state.rotate = 0
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
  // 缩放和旋转都直接归位，不要带动画（否则会从上一张的状态"滑"过来）
  state.noTransition = true
  resetZoom()
  resetRotate()
  requestAnimationFrame(() => {
    requestAnimationFrame(() => { state.noTransition = false })
  })
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
/* 当前这张是不是视频 */
export const viewerIsVideo = () => state.row?.kind === 'video'
/* 视频播放地址 */
export const viewerVideo = () => videoUrl(state.row)

/* 视频不内嵌播放：软件不带解码器（那样安装包要 60 MB+），
   点开查看器后由界面引导用系统播放器打开。这里不再需要 blob 逻辑。 */
