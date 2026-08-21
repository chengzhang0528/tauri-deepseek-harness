import { createHash } from 'node:crypto'
import { spawnSync } from 'node:child_process'
import { createReadStream, createWriteStream, existsSync } from 'node:fs'
import { access, copyFile, cp, mkdir, mkdtemp, readFile, readdir, rename, rm, stat, writeFile } from 'node:fs/promises'
import { dirname, join, resolve } from 'node:path'
import { Readable, Transform } from 'node:stream'
import { pipeline } from 'node:stream/promises'
import { fileURLToPath, pathToFileURL } from 'node:url'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const MAX_DOWNLOAD_BYTES = 768 * 1024 * 1024
const metadataPath = join(root, 'src-tauri', 'resources', 'runtime-versions.windows-x64.json')
const runtimePackagePath = join(root, 'runtime', 'package.json')
const runtimeLockPath = join(root, 'runtime', 'package-lock.json')
const bridgePatchPath = join(root, 'src-tauri', 'resources', 'desktop-bridge.patch.yml')
const bridgeModulePlaceholder = '__DSH_DESKTOP_BRIDGE_MODULE__'

function parseArgs(argv) {
  const values = new Map()
  for (let index = 0; index < argv.length; index += 1) {
    const key = argv[index]
    if (!key.startsWith('--') || !argv[index + 1] || argv[index + 1].startsWith('--')) {
      throw new Error(`expected --name value, got ${key}`)
    }
    values.set(key.slice(2), argv[index + 1])
    index += 1
  }
  return values
}

function required(values, name) {
  const value = values.get(name)
  if (!value) throw new Error(`missing --${name}`)
  return resolve(value)
}

async function readJson(path, label) {
  try {
    return JSON.parse(await readFile(path, 'utf8'))
  } catch (error) {
    throw new Error(`cannot read ${label}: ${error instanceof Error ? error.message : String(error)}`)
  }
}

function validateMetadata(metadata) {
  if (metadata.schema !== 1 || metadata.platform !== 'windows' || metadata.arch !== 'x64') {
    throw new Error('runtime metadata has an unsupported schema or target')
  }
  for (const component of ['node', 'dsh', 'pnpm', 'ripgrep']) {
    if (!/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(metadata[component]?.version ?? '')) {
      throw new Error(`runtime metadata version is invalid for ${component}`)
    }
  }
  for (const component of ['node', 'ripgrep']) {
    if (!/^[0-9a-f]{64}$/i.test(metadata[component]?.sha256 ?? '')) {
      throw new Error(`runtime metadata SHA-256 is invalid for ${component}`)
    }
  }
}

async function ensureNewDirectory(path, label) {
  if (existsSync(path)) {
    const entries = await readdir(path)
    if (entries.length > 0) throw new Error(`${label} must not already contain files: ${path}`)
  }
  await mkdir(path, { recursive: true })
}

async function downloadVerified(url, destination, expectedSha256) {
  if (existsSync(destination)) {
    const existing = await digest(destination)
    if (existing.sha256 === expectedSha256.toLowerCase()) return existing
    await rm(destination, { force: true })
  }
  let response
  try {
    response = await fetch(url)
  } catch (error) {
    throw new Error(`download request failed for ${url}: ${error instanceof Error ? error.message : String(error)}`)
  }
  if (!response.ok || !response.body) throw new Error(`download failed for ${url}: HTTP ${response.status}`)
  const declared = Number(response.headers.get('content-length') ?? 0)
  if (declared > MAX_DOWNLOAD_BYTES) throw new Error(`download exceeds ${MAX_DOWNLOAD_BYTES} bytes: ${url}`)

  const temporary = `${destination}.part`
  await rm(temporary, { force: true })
  let bytes = 0
  const hash = createHash('sha256')
  const meter = new Transform({
    transform(chunk, _encoding, callback) {
      bytes += chunk.length
      if (bytes > MAX_DOWNLOAD_BYTES) {
        callback(new Error(`download exceeds ${MAX_DOWNLOAD_BYTES} bytes: ${url}`))
        return
      }
      hash.update(chunk)
      callback(null, chunk)
    },
  })
  try {
    await pipeline(Readable.fromWeb(response.body), meter, createWriteStream(temporary, { flags: 'wx' }))
    const sha256 = hash.digest('hex')
    if (sha256 !== expectedSha256.toLowerCase()) {
      throw new Error(`SHA-256 mismatch for ${url}`)
    }
    await rename(temporary, destination)
    return { bytes, sha256 }
  } catch (error) {
    await rm(temporary, { force: true })
    throw error
  }
}

async function localAsset(path, expectedSha256, label) {
  if (!existsSync(path)) throw new Error(`${label} does not exist: ${path}`)
  const asset = await digest(path)
  if (asset.bytes > MAX_DOWNLOAD_BYTES) throw new Error(`${label} exceeds ${MAX_DOWNLOAD_BYTES} bytes`)
  if (asset.sha256 !== expectedSha256.toLowerCase()) throw new Error(`${label} SHA-256 mismatch`)
  return asset
}

async function digest(path) {
  const hash = createHash('sha256')
  let bytes = 0
  for await (const chunk of createReadStream(path)) {
    bytes += chunk.length
    hash.update(chunk)
  }
  return { bytes, sha256: hash.digest('hex') }
}

function extractZip(archive, destination) {
  const result = spawnSync('tar.exe', ['-xf', archive, '-C', destination], {
    cwd: root,
    windowsHide: true,
    encoding: 'utf8',
  })
  if (result.status !== 0) {
    throw new Error(`cannot extract ${archive}: ${result.stderr || result.stdout || 'tar.exe failed'}`)
  }
}

async function oneTopLevelDirectory(directory, label) {
  const entries = await readdir(directory, { withFileTypes: true })
  const directories = entries.filter((entry) => entry.isDirectory())
  if (entries.length !== 1 || directories.length !== 1) {
    throw new Error(`${label} archive has an unexpected top-level layout`)
  }
  return join(directory, directories[0].name)
}

function run(program, args, options = {}) {
  const result = spawnSync(program, args, {
    cwd: root,
    windowsHide: true,
    stdio: 'inherit',
    ...options,
  })
  if (result.error) throw result.error
  if (result.status !== 0) throw new Error(`${program} failed with exit code ${result.status}`)
}

async function writeNotices(runtimeRoot, metadata) {
  const content = `# Third-Party Notices\n\nThis runtime closure contains the following pinned upstream components. Their license texts are retained in the listed runtime paths.\n\n- Node.js ${metadata.node.version}: \`node/LICENSE\`\n- DeepSeek Harness ${metadata.dsh.version}: \`node_modules/@deepseek-ai/dsh\` and each dependency package's included license file\n- pnpm ${metadata.pnpm.version}: \`node_modules/pnpm\`\n- ripgrep ${metadata.ripgrep.version}: \`third-party/ripgrep/LICENSE-MIT\` and \`third-party/ripgrep/UNLICENSE\`\n\nBuild sources:\n\n- ${metadata.node.source}\n- ${metadata.dsh.source}\n- ${metadata.pnpm.source}\n- ${metadata.ripgrep.source}\n`
  await writeFile(join(runtimeRoot, 'THIRD_PARTY_NOTICES.md'), content, 'utf8')
}

async function validateLock(lock, metadata) {
  const dsh = lock.packages?.['node_modules/@deepseek-ai/dsh']?.version
  const pnpm = lock.packages?.['node_modules/pnpm']?.version
  if (dsh !== metadata.dsh.version) throw new Error(`package lock pins dsh ${dsh ?? 'unknown'}, expected ${metadata.dsh.version}`)
  if (pnpm !== metadata.pnpm.version) throw new Error(`package lock pins pnpm ${pnpm ?? 'unknown'}, expected ${metadata.pnpm.version}`)
}

async function renderBridgePatch(runtimeRoot) {
  const template = await readFile(bridgePatchPath, 'utf8')
  const occurrences = template.split(bridgeModulePlaceholder).length - 1
  if (occurrences !== 1) throw new Error('desktop bridge patch template must contain one module placeholder')
  return template.replace(bridgeModulePlaceholder, pathToFileURL(join(runtimeRoot, 'desktop-bridge.mjs')).href)
}

async function main() {
  const values = parseArgs(process.argv.slice(2))
  const runtimeRoot = required(values, 'runtime-root')
  const workDir = required(values, 'work-dir')
  const metadata = await readJson(metadataPath, 'runtime metadata')
  const lock = await readJson(runtimeLockPath, 'runtime package lock')
  validateMetadata(metadata)
  await validateLock(lock, metadata)
  await access(runtimePackagePath)
  await ensureNewDirectory(runtimeRoot, 'runtime root')
  await mkdir(workDir, { recursive: true })

  const downloads = join(workDir, 'downloads')
  await mkdir(downloads, { recursive: true })
  const nodeZip = values.has('node-zip')
    ? resolve(values.get('node-zip'))
    : join(downloads, `node-v${metadata.node.version}-win-x64.zip`)
  const ripgrepZip = values.has('ripgrep-zip')
    ? resolve(values.get('ripgrep-zip'))
    : join(downloads, `ripgrep-${metadata.ripgrep.version}-x86_64-pc-windows-msvc.zip`)
  const [nodeAsset, ripgrepAsset] = await Promise.all([
    values.has('node-zip')
      ? localAsset(nodeZip, metadata.node.sha256, 'Node ZIP')
      : downloadVerified(metadata.node.source, nodeZip, metadata.node.sha256),
    values.has('ripgrep-zip')
      ? localAsset(ripgrepZip, metadata.ripgrep.sha256, 'ripgrep ZIP')
      : downloadVerified(metadata.ripgrep.source, ripgrepZip, metadata.ripgrep.sha256),
  ])

  const extractionRoot = await mkdtemp(join(workDir, 'extract-'))
  try {
    const nodeExtract = join(extractionRoot, 'node')
    const ripgrepExtract = join(extractionRoot, 'ripgrep')
    await Promise.all([mkdir(nodeExtract), mkdir(ripgrepExtract)])
    extractZip(nodeZip, nodeExtract)
    extractZip(ripgrepZip, ripgrepExtract)
    await cp(await oneTopLevelDirectory(nodeExtract, 'Node'), join(runtimeRoot, 'node'), { recursive: true, force: false })
    const ripgrepRoot = await oneTopLevelDirectory(ripgrepExtract, 'ripgrep')
    await copyFile(join(ripgrepRoot, 'rg.exe'), join(runtimeRoot, 'rg.exe'), 0)
    await mkdir(join(runtimeRoot, 'third-party', 'ripgrep'), { recursive: true })
    for (const file of ['LICENSE-MIT', 'UNLICENSE']) {
      await copyFile(join(ripgrepRoot, file), join(runtimeRoot, 'third-party', 'ripgrep', file), 0)
    }
  } finally {
    await rm(extractionRoot, { recursive: true, force: true })
  }

  await Promise.all([
    copyFile(runtimePackagePath, join(runtimeRoot, 'package.json')),
    copyFile(runtimeLockPath, join(runtimeRoot, 'package-lock.json')),
    copyFile(metadataPath, join(runtimeRoot, 'runtime-versions.windows-x64.json')),
    copyFile(join(root, 'src-tauri', 'resources', 'desktop-bridge.mjs'), join(runtimeRoot, 'desktop-bridge.mjs')),
    copyFile(join(root, 'scripts', 'doctor-runtime.mjs'), join(runtimeRoot, 'doctor-runtime.mjs')),
  ])
  await writeFile(join(runtimeRoot, 'desktop-bridge.patch.yml'), await renderBridgePatch(runtimeRoot), 'utf8')

  const npmCli = join(runtimeRoot, 'node', 'node_modules', 'npm', 'bin', 'npm-cli.js')
  const npmCache = join(workDir, 'npm-cache')
  run(join(runtimeRoot, 'node', 'node.exe'), [npmCli, 'ci', '--omit=dev', '--no-audit', '--no-fund'], {
    cwd: runtimeRoot,
    env: {
      ...process.env,
      PATH: `${join(runtimeRoot, 'node')};${process.env.PATH ?? ''}`,
      npm_config_cache: npmCache,
    },
  })
  await writeNotices(runtimeRoot, metadata)
  const closure = await directorySize(runtimeRoot)
  process.stdout.write(`runtime root: ${runtimeRoot}\nnode download bytes: ${nodeAsset.bytes}\nripgrep download bytes: ${ripgrepAsset.bytes}\nclosure bytes: ${closure}\n`)
}

async function directorySize(directory) {
  let total = 0
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name)
    if (entry.isDirectory()) total += await directorySize(path)
    else if (entry.isFile()) total += (await stat(path)).size
    else throw new Error(`runtime closure contains a non-file entry: ${path}`)
  }
  return total
}

try {
  await main()
} catch (error) {
  process.stderr.write(`runtime preparation failed: ${error instanceof Error ? error.message : String(error)}\n`)
  process.exitCode = 1
}
