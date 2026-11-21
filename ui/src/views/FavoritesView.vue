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
  <div class="wrap">
    <div class="head card">
      <div class="head-left">
        <div class="head-ico"><Icon name="starFilled" :size="20" /></div>
        <div>
          <strong>{{ total }} 张收藏</strong>
          <span>在照片上点 ★ 即可加入或取消收藏</span>
        </div>
      </div>
    </div>

    <div v-if="!total && !loading" class="empty card">
      <Icon name="star" :size="30" />
      <h3>还没有收藏的照片</h3>
      <p>浏览照片时点卡片上的星标，就会收藏到这里</p>
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
.wrap { height: 100%; overflow-y: auto; display: flex; flex-direction: column; gap: 14px; padding-bottom: 24px; }
.head { display: flex; align-items: center; justify-content: space-between; gap: 16px; padding: 14px 18px; flex: none; }
.head-left { display: flex; align-items: center; gap: 12px; }
.head-ico {
  display: grid; place-items: center; width: 38px; height: 38px;
  border-radius: var(--r-sm); color: #fff;
  background: linear-gradient(135deg, #f5a623 0%, #f97316 100%);
}
.head-left strong { display: block; font-size: var(--fs-md); }
.head-left span { font-size: var(--fs-xs); color: var(--text-tertiary); }
.empty { display: flex; flex-direction: column; align-items: center; justify-content: center; padding: 60px 0; gap: 10px; color: var(--text-tertiary); }
.empty h3 { margin: 0; color: var(--text-secondary); }
.empty p { margin: 0; font-size: var(--fs-sm); }
</style>
