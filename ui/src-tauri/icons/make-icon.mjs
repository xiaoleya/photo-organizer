/* 把 icon.svg 渲染成多尺寸 PNG，再打包成 Windows ICO。
   用法：node make-icon.mjs

   为什么不能「一个尺寸缩到底」：
   镜头白环在 1024 画布里只有 40px 宽，直接缩到 16px 时
   只剩 0.6 像素，抗锯齿后几乎透明 —— 任务栏图标就变成一块蓝方块。
   所以小尺寸单独处理：放大描边、简化细节，保证 16px 下仍能认出是相机。
*/
import sharp from 'sharp'
import { readFileSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const svgPath = join(here, 'icon.svg')
const icoPath = join(here, 'icon.ico')

/* 小尺寸专用 SVG：白环更粗、镜头更大、机身撑满画布，省掉高光点。
   小尺寸最怕「看起来小」——四周留白会被任务栏背景吃掉，
   所以机身和镜头都往外扩，尽量占满。 */
function smallSizeSvg(strokeWidth, lensR) {
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024" width="1024" height="1024">
  <defs>
    <linearGradient id="b" x1="0.5" y1="0" x2="0.5" y2="1">
      <stop offset="0" stop-color="#5EB2FF"/><stop offset="1" stop-color="#2E7FDC"/>
    </linearGradient>
  </defs>
  <g opacity="0.92">
    <rect x="330" y="140" width="364" height="190" rx="66" fill="url(#b)"/>
    <rect x="40" y="250" width="944" height="624" rx="150" fill="url(#b)"/>
    <circle cx="512" cy="562" r="${lensR}" fill="none" stroke="#ffffff" stroke-width="${strokeWidth}"/>
  </g>
</svg>`
}

/* 大尺寸细节多；小尺寸换成简化版 */
const SPEC = [
  { size: 16, svg: smallSizeSvg(104, 226) },
  { size: 24, svg: smallSizeSvg(86, 214) },
  { size: 32, svg: smallSizeSvg(70, 200) },
  { size: 48, svg: null },   // 48 起用原始 SVG 就够清晰
  { size: 64, svg: null },
  { size: 128, svg: null },
  { size: 256, svg: null },
]

const baseSvg = readFileSync(svgPath)
const images = []
for (const { size, svg } of SPEC) {
  const src = svg ? Buffer.from(svg) : baseSvg
  // 小尺寸先按较大的画布渲染再缩，边缘更干净；
  // 但不能太高，sharp 对总像素数有上限
  const density = size <= 32 ? 384 : 192
  const png = await sharp(src, { density })
    .resize(size, size, { fit: 'contain', background: { r: 0, g: 0, b: 0, alpha: 0 } })
    .png({ compressionLevel: 9 })
    .toBuffer()
  images.push({ size, png })
  console.log(`渲染 ${size}x${size} -> ${png.length} bytes${svg ? ' (简化版)' : ''}`)
}

// ---- 组装 ICO ----
const headerSize = 6
const dirEntrySize = 16
let offset = headerSize + dirEntrySize * images.length

const header = Buffer.alloc(headerSize)
header.writeUInt16LE(0, 0)
header.writeUInt16LE(1, 2)
header.writeUInt16LE(images.length, 4)

const dirEntries = []
for (const img of images) {
  const e = Buffer.alloc(dirEntrySize)
  e.writeUInt8(img.size >= 256 ? 0 : img.size, 0)
  e.writeUInt8(img.size >= 256 ? 0 : img.size, 1)
  e.writeUInt8(0, 2)
  e.writeUInt8(0, 3)
  e.writeUInt16LE(1, 4)
  e.writeUInt16LE(32, 6)
  e.writeUInt32LE(img.png.length, 8)
  e.writeUInt32LE(offset, 12)
  dirEntries.push(e)
  offset += img.png.length
}

const ico = Buffer.concat([header, ...dirEntries, ...images.map((i) => i.png)])
writeFileSync(icoPath, ico)
console.log(`\n写入 ${icoPath}  ${ico.length} bytes  (${images.length} 个尺寸)`)
