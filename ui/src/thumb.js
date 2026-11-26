/* 后端返回的 path 是本地文件路径，统一转成图片地址。
 *
 * 引擎已改为进程内 Rust，图片走自定义协议，不再是本机 HTTP 服务。
 *
 * 关键：URL 必须带内容指纹（mtime+size）。
 * 只写 /photo?id=3 的话，重置索引重新扫描后 id=3 可能变成另一张照片，
 * 而浏览器缓存了旧 URL 的图片，就会出现"缩略图对、点开的大图不对"。
 * 把 mtime/size 拼进查询串，换了内容 URL 就变，缓存自然失效。
 *
 * 协议 URL 格式（Tauri 2）：
 *   Windows / Android: http://<scheme>.localhost/<path>
 *   macOS / Linux:     <scheme>://localhost/<path>
 * 原先写成 po://local/... 在 Windows 上不会被 WebView2 解析，图片全黑。
 */

function base() {
  const isWindows = typeof navigator !== 'undefined' &&
    /Windows/i.test(navigator.userAgent || '')
  return isWindows ? 'http://po.localhost' : 'po://localhost'
}

function ver(row) {
  const m = row && (row.mtime || row.mtime_ms)
  const s = row && row.size
  if (m === undefined && s === undefined) return ''
  return '&v=' + encodeURIComponent(String(m || 0)) + '_' + encodeURIComponent(String(s || 0))
}

export function thumbUrl(row, size = 480) {
  if (!row) return ''
  const B = base()
  if (row.id) return B + '/thumb?id=' + encodeURIComponent(row.id) + '&size=' + size + ver(row)
  if (row.thumb) return B + '/thumb?path=' + encodeURIComponent(row.thumb) + '&size=' + size + ver(row)
  if (row.path) return B + '/thumb?path=' + encodeURIComponent(row.path) + '&size=' + size + ver(row)
  return ''
}

export function photoUrl(row) {
  if (!row) return ''
  const B = base()
  if (row.id) {
    const v = encodeURIComponent(String(row.mtime || 0) + '_' + String(row.size || 0))
    return B + '/photo?id=' + encodeURIComponent(row.id) + '&v=' + v
  }
  if (row.path) return B + '/photo?path=' + encodeURIComponent(row.path) + ver(row)
  return ''
}

/* 视频播放地址：走 /video 端点，支持 Range 分段 */
export function videoUrl(row) {
  if (!row) return ''
  const B = base()
  if (row.id) {
    const v = encodeURIComponent(String(row.mtime || 0) + '_' + String(row.size || 0))
    return B + '/video?id=' + encodeURIComponent(row.id) + '&v=' + v
  }
  if (row.path) return B + '/video?path=' + encodeURIComponent(row.path) + ver(row)
  return ''
}
