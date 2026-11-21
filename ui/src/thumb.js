/* 后端返回的 path 是本地文件路径，统一转成后端图片地址。

   关键：URL 必须带内容指纹（mtime+size）。
   只写 /photo?id=3 的话，重置索引重新扫描后 id=3 可能变成另一张照片，
   而浏览器缓存了旧 URL 的图片，就会出现"缩略图对、点开的大图不对"。
   把 mtime/size 拼进查询串，换了内容 URL 就变，缓存自然失效。 */

const BASE = 'http://127.0.0.1:18921'

function ver(row) {
  const m = row && (row.mtime || row.mtime_ms)
  const s = row && row.size
  if (m === undefined && s === undefined) return ''
  return '&v=' + encodeURIComponent(String(m || 0)) + '_' + encodeURIComponent(String(s || 0))
}

export function thumbUrl(row, size = 480) {
  if (!row) return ''
  if (row.id) return BASE + '/thumb?id=' + encodeURIComponent(row.id) + '&size=' + size + ver(row)
  if (row.thumb) return BASE + '/thumb?path=' + encodeURIComponent(row.thumb) + '&size=' + size + ver(row)
  if (row.path) return BASE + '/thumb?path=' + encodeURIComponent(row.path) + '&size=' + size + ver(row)
  return ''
}

export function photoUrl(row) {
  if (!row) return ''
  if (row.id) {
    const v = encodeURIComponent(String(row.mtime || 0) + '_' + String(row.size || 0))
    return BASE + '/photo?id=' + encodeURIComponent(row.id) + '&v=' + v
  }
  if (row.path) return BASE + '/photo?path=' + encodeURIComponent(row.path) + ver(row)
  return ''
}
