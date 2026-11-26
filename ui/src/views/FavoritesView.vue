<script setup>
/* 收藏：单独一个照片墙，只列被标星的照片。 */
import { ref, computed, onMounted, onBeforeUnmount } from 'vue'
import Icon from '../components/Icon.vue'
import PhotosPanel from '../components/PhotosPanel.vue'
import { api } from '../api'

const props = defineProps({
  search: { type: String, default: '' },
})

const total = ref(0)
const loading = ref(true)
const panelKey = ref(0)

const query = computed(() => {
  const q = { fav: '1', sort: 'dt DESC' }
  if (props.search) q.q = props.search
  return q
})

async function refresh() {
  loading.value = true
  const r = await api.photos({ fav: '1', limit: 1 })
  total.value = r ? r.total : 0
  loading.value = false
}

onMounted(() => {
  refresh()
  window.addEventListener('opo:favorite-changed', onAnyFavorite)
})
onBeforeUnmount(() => {
  window.removeEventListener('opo:favorite-changed', onAnyFavorite)
})

/* 任一处取消收藏后刷新，避免列表里还留着已取消的图 */
function onAnyFavorite(e) {
  if (e && e.detail && e.detail.favorite === false) {
    panelKey.value++
    refresh()
  }
}
</script>

<template>
  <div class="page">
    <div class="page-head card">
      <div class="page-head-main">
        <div class="page-head-ico grad-amber">
          <Icon name="starFilled" :size="20" />
        </div>
        <div class="page-head-text">
          <strong>{{ total }} 张收藏</strong>
          <span>在照片上点 ★ 即可加入或取消收藏</span>
        </div>
      </div>
    </div>

    <div v-if="!total && !loading" class="empty-state fill">
      <div class="es-ico"><Icon name="star" :size="26" /></div>
      <h3 class="es-title">还没有收藏的照片</h3>
      <p class="es-desc">浏览照片时点卡片上的星标，就会收藏到这里</p>
    </div>

    <PhotosPanel
      v-else
      :key="panelKey"
      :query="query"
      :page-size="120"
      empty-title="没有收藏的照片"
      empty-hint="点照片卡片上的星标即可收藏"
      @count="total = $event"
    />
  </div>
</template>

<style scoped>
/* 布局已统一到 base.css 的 .page / .page-head / .empty-state，此处无自定义样式 */
</style>
