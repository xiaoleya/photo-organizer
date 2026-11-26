<script setup>
/* 地点：按地点分组的时间轴。支持全局搜索过滤。 */
import { ref, onMounted, computed } from 'vue'
import Icon from '../components/Icon.vue'
import PhotosPanel from '../components/PhotosPanel.vue'
import { api } from '../api'

const props = defineProps({
  search: { type: String, default: '' },
})

const places = ref([])
const loading = ref(true)
const keyword = ref('')
const openPlace = ref({})
const openYear = ref({})
const backfilling = ref(false)
const knownCount = ref(0)

async function load() {
  loading.value = true
  const r = await api.places()
  loading.value = false
  if (r && r.items) {
    places.value = r.items
    knownCount.value = r.known_count ?? r.items.filter(x => x.place !== '未知地点').length
    if (!Object.keys(openPlace.value).length && r.items.length) {
      const firstKnown = r.items.find(x => x.place !== '未知地点') || r.items[0]
      openPlace.value = { [firstKnown.place]: true }
    }
  }
}

onMounted(load)

async function backfill() {
  backfilling.value = true
  await api.backfillPlaces()
  backfilling.value = false
  await load()
}

const shown = computed(() => {
  const kw = keyword.value.trim()
  if (!kw) return places.value
  return places.value.filter(p => p.place.includes(kw))
})

/* 按上级地区（省/直辖市）分组，组内再列县市，便于区分同名/同省的多地 */
const groupedPlaces = computed(() => {
  const map = new Map()
  for (const p of shown.value) {
    const parent = p.parent || ''
    if (!map.has(parent)) map.set(parent, { parent, items: [], count: 0 })
    const g = map.get(parent)
    g.items.push(p)
    g.count += p.count
  }
  return [...map.values()].sort((a, b) => {
    // 未知地点永远排在最后
    if (a.parent === '' && a.items[0]?.place === '未知地点') return 1
    if (b.parent === '' && b.items[0]?.place === '未知地点') return -1
    return b.count - a.count
  })
})

const totalPhotos = computed(() => places.value.reduce((s, p) => s + p.count, 0))

/* 全局搜索：把标题栏的搜索词带到每张照片的查询里 */
const queryFor = (extra) => {
  const q = { ...extra }
  if (props.search) q.q = props.search
  return q
}

function togglePlace(place) {
  openPlace.value = { ...openPlace.value, [place]: !openPlace.value[place] }
}
function toggleYear(place, year) {
  const k = place + '|' + year
  openYear.value = { ...openYear.value, [k]: !openYear.value[k] }
}
function isYearOpen(place, year) {
  return openYear.value[place + '|' + year] !== false
}
</script>

<template>
  <div class="page">
    <div class="page-head card">
      <div class="page-head-main">
        <div class="page-head-ico grad-blue"><Icon name="mapPin" :size="20" /></div>
        <div class="page-head-text">
          <strong>{{ knownCount }} 个已识别地点</strong>
          <span>共 {{ totalPhotos }} 张照片<template v-if="places.length > knownCount">，另有 {{ places.length - knownCount }} 组未识别</template></span>
        </div>
      </div>
      <div class="page-head-acts">
        <label class="head-search">
          <Icon name="search" :size="16" />
          <input v-model="keyword" placeholder="按地点名筛选" />
        </label>
        <button class="btn btn-ghost" :disabled="backfilling" @click="backfill">
          {{ backfilling ? '识别中…' : '重新识别地点' }}
        </button>
      </div>
    </div>

    <!-- 地点识别说明：很多用户不知道哪些照片才有地点 -->
    <details class="notice">
      <summary>
        <Icon name="helpCircle" :size="15" />
        为什么大部分照片显示「未知地点」？点这里看说明
      </summary>
      <div class="notice-body">
        <p><b>只有下面这些照片才带地点信息：</b></p>
        <ul>
          <li>手机<b>原相机</b>拍摄、且拍摄时<b>开启了定位</b>的照片</li>
          <li>照片里保留了 <b>GPS 信息（EXIF）</b>，且之后没有被任何软件处理过</li>
          <li>照片所在<b>文件夹或文件名里含城市名</b>（例如 <code>D:\照片\北京\</code>），软件也会据此识别</li>
        </ul>
        <p><b>这些情况地点会丢失（所以显示未知地点）：</b></p>
        <ul>
          <li><b>微信 / QQ 传输</b>：为保护隐私，转发时会自动清空 GPS</li>
          <li><b>截图</b>：截图本身不带任何拍摄信息</li>
          <li>经过美图、压缩、二次编辑后重新保存的图片</li>
          <li>相机拍摄的胶片扫描件、老照片翻拍</li>
        </ul>
        <p class="notice-tip">
          想给这些照片补上地点，可以按城市把照片放进对应文件夹（如「上高」），
          再点「重新识别地点」，软件就会按文件夹名归类。
        </p>
      </div>
    </details>

    <div v-if="!places.length && !loading" class="empty-state fill">
      <div class="es-ico"><Icon name="mapPin" :size="26" /></div>
      <h3 class="es-title">还没有地点信息</h3>
      <p class="es-desc">带 GPS 的照片会自动识别到城市；目录名里带城市名也能识别</p>
    </div>

    <div v-else class="places">
      <section v-for="g in groupedPlaces" :key="g.parent || 'none'" class="region">
        <div v-if="g.parent" class="region-head">
          <Icon name="map" :size="14" />
          <strong>{{ g.parent }}</strong>
          <span>{{ g.items.length }} 个地方 · {{ g.count }} 张</span>
        </div>

        <section v-for="p in g.items" :key="p.place" class="place">
          <button class="place-head" @click="togglePlace(p.place)">
            <span class="place-pin" :class="p.place === '未知地点' ? 'unknown' : 'known'">
              <Icon :name="p.place === '未知地点' ? 'helpCircle' : 'pinFilled'" :size="17" />
            </span>
            <span class="place-info">
              <strong>{{ p.leaf || p.place }}</strong>
              <span>
                <template v-if="p.parent && p.parent !== p.place">{{ p.parent }} · </template>
                {{ p.years.length }} 年 · {{ p.count }} 张
              </span>
            </span>
            <Icon :name="openPlace[p.place] ? 'chevronDown' : 'chevronRight'" :size="18" />
          </button>

          <div v-if="openPlace[p.place]" class="place-body">
            <div v-if="!p.years.length" class="no-year">
              <PhotosPanel :query="queryFor({ place: p.place })" :page-size="60" :show-hint="false" />
            </div>
            <div v-for="y in p.years" :key="y.year" class="year-block">
              <button class="year-head" @click="toggleYear(p.place, y.year)">
                <strong>{{ y.year }} 年</strong>
                <span>{{ y.count }} 张</span>
                <Icon :name="isYearOpen(p.place, y.year) ? 'chevronDown' : 'chevronRight'" :size="15" />
              </button>
              <div v-if="isYearOpen(p.place, y.year)" class="year-body">
                <PhotosPanel
                  :query="queryFor({ place: p.place, year: y.year, sort: 'dt ASC' })"
                  :page-size="60"
                  :show-hint="false"
                />
              </div>
            </div>
          </div>
        </section>
      </section>
    </div>
  </div>
</template>

<style scoped>
/* 根容器/页头/空态已统一到 base.css，这里只保留本页特有的地点树样式 */
.head-search { display: flex; align-items: center; gap: 8px; height: 34px; padding: 0 12px; border-radius: var(--r-pill); background: var(--surface-2); color: var(--text-tertiary); }
.head-search input { width: 180px; font-size: var(--fs-sm); color: var(--text-primary); }

/* 地点识别说明 */
.notice {
  flex: none; border-radius: var(--r-md); background: var(--surface);
  border: 1px solid var(--line); overflow: hidden;
}
.notice summary {
  display: flex; align-items: center; gap: 8px;
  padding: 11px 15px; cursor: pointer; list-style: none;
  font-size: var(--fs-sm); color: var(--brand-300);
}
.notice summary::-webkit-details-marker { display: none; }
.notice summary:hover { background: var(--surface-hover); }
.notice-body { padding: 4px 18px 14px; font-size: var(--fs-sm); color: var(--text-secondary); line-height: 1.7; }
.notice-body p { margin: 8px 0 4px; }
.notice-body b { color: var(--text-primary); }
.notice-body ul { margin: 0; padding-left: 20px; }
.notice-body li { margin: 2px 0; }
.notice-body code {
  padding: 1px 5px; border-radius: 4px;
  background: var(--surface-2); font-size: var(--fs-xs);
}
.notice-tip { color: var(--text-tertiary); font-size: var(--fs-xs); margin-top: 10px !important; }
.places { display: flex; flex-direction: column; gap: 14px; }
.region { display: flex; flex-direction: column; gap: 10px; }
.region-head {
  display: flex; align-items: center; gap: 8px;
  padding: 2px 4px; font-size: var(--fs-sm); color: var(--text-tertiary);
}
.region-head strong { font-size: var(--fs-base); color: var(--text-secondary); }
.region-head span { font-size: var(--fs-xs); }
.place { background: var(--surface); border-radius: var(--r-xl); box-shadow: var(--shadow-md); overflow: hidden; }
.place-head { display: flex; width: 100%; align-items: center; gap: 14px; padding: 15px 20px; text-align: left; }
.place-head:hover { background: var(--surface-hover); }
.place-pin { display: grid; place-items: center; width: 40px; height: 40px; border-radius: var(--r-md); color: #fff; }
.place-pin.known { background: var(--grad-blue); }
.place-pin.unknown { background: var(--surface-3); color: var(--text-tertiary); }
.place-info { flex: 1; display: flex; flex-direction: column; gap: 1px; }
.place-info strong { font-size: var(--fs-md); }
.place-info span { font-size: var(--fs-xs); color: var(--text-tertiary); }
.place-body { padding: 0 16px 16px; display: flex; flex-direction: column; gap: 10px; }
.year-block { border-top: 1px solid var(--line); padding-top: 8px; }
.year-head { display: flex; width: 100%; align-items: center; gap: 10px; padding: 6px 2px; text-align: left; }
.year-head strong { font-size: var(--fs-base); }
.year-head span { font-size: var(--fs-xs); color: var(--text-tertiary); }
.year-head svg { margin-left: auto; color: var(--text-tertiary); }
.year-body { padding-bottom: 6px; }
.no-year { padding-bottom: 6px; }
</style>
