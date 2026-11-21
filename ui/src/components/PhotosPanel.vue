<script setup>
/* PhotosPanel：统一的照片墙。
   - 卡片高度固定，图片用 contain 完整显示（不裁切）
   - 不嵌套滚动容器，跟随页面滚动
   - 滚动到底部自动继续加载，直到全部照片都出来
   - 点击打开大图，右键/双击调用系统看片器 */
import { ref, watch, onMounted, onBeforeUnmount, computed, nextTick } from 'vue'
import Icon from './Icon.vue'
import { api, native } from '../api'
import { thumbUrl } from '../thumb'
import { openViewer } from '../viewer'

const props = defineProps({
  query: { type: Object, default: () => ({}) },
  pageSize: { type: Number, default: 200 },
  thumbHeight: { type: Number, default: 168 },
  emptyTitle: { type: String, default: '没有符合条件的照片' },
  emptyHint: { type: String, default: '换个筛选条件，或先添加照片目录' },
  showHint: { type: Boolean, default: true },
  showRepair: { type: Boolean, default: false },
})

const emit = defineEmits(['count', 'repair', 'favorite'])

const photos = ref([])
const total = ref(0)
const loading = ref(false)
const failed = ref({})
const sentinel = ref(null)
const autoLoad = ref(true)

let seq = 0
let observer = null

async function load(reset = true) {
  const my = ++seq
  loading.value = true
  if (reset) { photos.value = []; total.value = 0 }
  const q = { ...props.query, limit: props.pageSize, offset: reset ? 0 : photos.value.length }
  Object.keys(q).forEach(k => {
    if (q[k] === '' || q[k] === null || q[k] === undefined) delete q[k]
  })
  const r = await api.photos(q)
  if (my !== seq) return
  loading.value = false
  if (r && r.items) {
    photos.value = reset ? r.items : photos.value.concat(r.items)
    total.value = r.total ?? photos.value.length
    emit('count', total.value)
    await nextTick()
    // 如果一页填不满屏幕，继续拉，直到填满或取完
    if (autoLoad.value && photos.value.length < total.value) maybeLoadMore()
  }
}

function maybeLoadMore() {
  if (loading.value) return
  if (photos.value.length >= total.value) return
  load(false)
}

function setupObserver() {
  if (typeof IntersectionObserver === 'undefined' || !sentinel.value) return
  if (observer) observer.disconnect()
  observer = new IntersectionObserver(entries => {
    if (entries.some(e => e.isIntersecting) && autoLoad.value) maybeLoadMore()
  }, { rootMargin: '300px' })
  observer.observe(sentinel.value)
}

function onImgError(id) {
  failed.value = { ...failed.value, [id]: true }
}

/* 收藏 / 取消收藏：先就地更新，再落库，失败时回滚 */
async function toggleFav(p) {
  const next = !p.favorite
  const idx = photos.value.findIndex(x => x.id === p.id)
  if (idx >= 0) photos.value[idx] = { ...photos.value[idx], favorite: next ? 1 : 0 }
  const r = await api.favorite(p.id, next)
  if (!r || !r.ok) {
    if (idx >= 0) photos.value[idx] = { ...photos.value[idx], favorite: next ? 0 : 1 }
    return
  }
  emit('favorite', { id: p.id, favorite: next })
  // 广播给收藏页等其它视图，保持数量同步
  window.dispatchEvent(new CustomEvent('opo:favorite-changed',
    { detail: { id: p.id, favorite: next } }))
}

watch(() => JSON.stringify(props.query || {}), () => load(true))

onMounted(async () => {
  await load(true)
  setupObserver()
})

onBeforeUnmount(() => {
  if (observer) observer.disconnect()
  observer = null
})

defineExpose({
  reload: () => load(true),
  loadAll: () => { autoLoad.value = true; maybeLoadMore() },
  photos, total,
})

const hasMore = computed(() => photos.value.length < total.value)
</script>

<template>
  <div class="panel">
    <div v-if="showHint" class="panel-head">
      <span>共 {{ total }} 张照片<template v-if="hasMore">（已显示 {{ photos.length }}）</template></span>
    </div>

    <div v-if="!photos.length && !loading" class="panel-empty">
      <Icon name="photos" :size="28" />
      <strong>{{ emptyTitle }}</strong>
      <span>{{ emptyHint }}</span>
    </div>

    <div v-else class="panel-grid">
      <article
        v-for="p in photos"
        :key="p.id"
        class="pcard"
        @click="openViewer(p, photos)"
        @dblclick="native.openPath(p.path)"
      >
        <div class="pthumb" :style="{ height: thumbHeight + 'px' }">
          <img
            v-if="!failed[p.id]"
            :src="thumbUrl(p, 640)"
            :alt="p.name"
            loading="lazy"
            decoding="async"
            @error="onImgError(p.id)"
          />
          <div v-else class="pthumb-fail"><Icon name="image" :size="20" /></div>
        </div>
        <div class="pmeta">
          <span class="pname" :title="p.name">{{ p.name }}</span>
          <span class="psub">
            {{ p.dt ? p.dt.slice(0, 10) : '无日期' }}<template v-if="p.place && p.place !== '未知地点'"> · {{ p.place }}</template>
          </span>
        </div>

        <!-- 底部操作条：收藏（星标）在左，去修复在右，都常驻显示 -->
        <div class="pactions">
          <button
            class="pact fav" :class="{ on: p.favorite }"
            :title="p.favorite ? '取消收藏' : '加入收藏'"
            @click.stop="toggleFav(p)"
          >
            <Icon :name="p.favorite ? 'starFilled' : 'star'" :size="14" />
          </button>
          <button
            v-if="showRepair" class="pact repair" title="去修复这张照片"
            @click.stop="emit('repair', p)"
          >
            <Icon name="wrench" :size="14" />
            <span>修复</span>
          </button>
        </div>
      </article>
    </div>

    <div ref="sentinel" class="panel-foot">
      <button v-if="hasMore" class="more" :disabled="loading" @click="maybeLoadMore">
        {{ loading ? '加载中…' : `加载更多（剩余 ${total - photos.length} 张）` }}
      </button>
      <span v-else-if="photos.length" class="done">已显示全部 {{ photos.length }} 张</span>
      <span v-else-if="loading" class="done">加载中…</span>
    </div>
  </div>
</template>

<style scoped>
.panel { display: flex; flex-direction: column; }
.panel-head { font-size: var(--fs-sm); color: var(--text-tertiary); margin-bottom: 10px; }
.panel-grid {
  display: grid; gap: 14px; align-content: start;
  grid-auto-rows: min-content;
  grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
}
.pcard {
  position: relative; display: flex; flex-direction: column;
  border-radius: var(--r-lg); background: var(--surface);
  box-shadow: var(--shadow-sm); cursor: pointer; overflow: hidden;
  transition: transform var(--dur) var(--ease), box-shadow var(--dur) var(--ease);
}
.pcard:hover { transform: translateY(-3px); box-shadow: var(--shadow-md); }
.pthumb {
  width: 100%; flex: none; display: flex; align-items: center; justify-content: center;
  background: #0c0e12; overflow: hidden;
}
/* 小图/长条图按原比例放大到能看清，但不裁切、不拉伸变形 */
.pthumb img {
  max-width: 100%; max-height: 100%; width: auto; height: auto;
  object-fit: contain; display: block; image-rendering: auto;
}
.pthumb-fail {
  display: grid; place-items: center; width: 100%; height: 100%;
  color: var(--text-tertiary); background: var(--surface-3);
}
.pmeta { padding: 9px 11px 4px; display: flex; flex-direction: column; gap: 2px; min-width: 0; }
.pname {
  font-size: var(--fs-base); font-weight: 600; color: var(--text-primary);
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
}
.psub {
  font-size: var(--fs-xs); color: var(--text-tertiary);
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
}

/* 常驻操作条：收藏在左、修复在右，不再浮在图片右上角遮挡内容 */
.pactions {
  display: flex; align-items: center; gap: 6px;
  padding: 0 9px 9px;
}
.pact {
  display: inline-flex; align-items: center; gap: 4px;
  height: 26px; padding: 0 9px; border-radius: var(--r-xs);
  background: var(--surface-2); color: var(--text-tertiary);
  font-size: var(--fs-xs); transition: all var(--dur) var(--ease);
}
.pact:hover { background: var(--surface-3); color: var(--text-primary); }
.pact.fav.on { background: rgba(245,158,11,.16); color: #f5a623; }
.pact.repair { margin-left: auto; }
.pact.repair:hover { background: var(--tint-strong); color: var(--brand-300); }
.panel-empty {
  display: flex; flex-direction: column; align-items: center;
  justify-content: center; gap: 8px; padding: 70px 0; color: var(--text-tertiary);
}
.panel-empty strong { color: var(--text-secondary); font-size: var(--fs-md); }
.panel-empty span { font-size: var(--fs-sm); }
.panel-foot { display: flex; justify-content: center; padding: 16px 0 6px; }
.more {
  padding: 9px 22px; border-radius: var(--r-pill);
  background: var(--surface-2); color: var(--text-secondary); font-size: var(--fs-sm);
}
.more:hover { background: var(--surface-3); color: var(--text-primary); }
.more:disabled { opacity: .6; }
.done { font-size: var(--fs-xs); color: var(--text-tertiary); }
</style>
