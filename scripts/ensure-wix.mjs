import { createHash } from 'node:crypto'
import { existsSync } from 'node:fs'
import { cp, mkdir, readFile, writeFile } from 'node:fs/promises'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { spawnSync } from 'node:child_process'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const toolsRoot = join(root, '.tools', 'wix314')
const wixBin = join(toolsRoot, 'tools', 'bin')
const candle = join(wixBin, process.platform === 'win32' ? 'candle.exe' : 'candle')
const light = join(wixBin, process.platform === 'win32' ? 'light.exe' : 'light')
const packagePath = join(root, '.tools', 'wixsharp.wix.bin.3.14.1.nupkg')
const packageUrl = 'https://api.nuget.org/v3-flatcontainer/wixsharp.wix.bin/3.14.1/wixsharp.wix.bin.3.14.1.nupkg'
const expectedSha256 = 'E1864FCA96756322EAA1D8AC64BB9EC2CA83ADCD25B2DA08126CAB8AD9C681F2'

if (existsSync(candle) && existsSync(light)) process.exit(0)

await mkdir(toolsRoot, { recursive: true })
if (!existsSync(packagePath)) {
  console.log('下载固定 WiX 3.14 构建工具...')
  const response = await fetch(packageUrl)
  if (!response.ok) throw new Error(`WiX 下载失败：HTTP ${response.status}`)
  await writeFile(packagePath, Buffer.from(await response.arrayBuffer()))
}

const bytes = await readFile(packagePath)
const actualSha256 = createHash('sha256').update(bytes).digest('hex').toUpperCase()
if (actualSha256 !== expectedSha256) {
  throw new Error(`WiX 包 SHA-256 不匹配：${actualSha256}`)
}

const extractor = process.platform === 'win32' ? 'tar.exe' : 'tar'
const result = spawnSync(extractor, ['-xf', packagePath, '-C', toolsRoot], {
  cwd: root,
  stdio: 'inherit',
})
if (result.status !== 0) process.exit(result.status ?? 1)
if (!existsSync(candle) || !existsSync(light)) {
  throw new Error(`WiX 工具缺失：${candle} / ${light}`)
}

const tauriCache = join(process.env.LOCALAPPDATA ?? join(root, '.tools'), 'tauri', 'WixTools314')
await mkdir(tauriCache, { recursive: true })
await cp(wixBin, tauriCache, { recursive: true, force: true })
