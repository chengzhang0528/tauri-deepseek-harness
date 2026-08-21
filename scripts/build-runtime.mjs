import { createHash } from 'node:crypto'
import { createReadStream } from 'node:fs'
import { mkdir, readFile, readdir, rename, rm, writeFile } from 'node:fs/promises'
import { existsSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { spawnSync } from 'node:child_process'

const root = resolve(fileURLToPath(new URL('..', import.meta.url)))
const RELEASE_VERSION = /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/

export function parseArgs(argv) {
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

export function resolveMinimumLauncherVersion(values, fallback) {
  const explicit = values.get('minimum-launcher-version')
  const legacy = values.get('launcher-version')
  if (explicit && legacy && explicit !== legacy) {
    throw new Error('--minimum-launcher-version and --launcher-version must match when both are provided')
  }
  const value = explicit ?? legacy ?? fallback
  if (!RELEASE_VERSION.test(value)) {
    throw new Error('minimum launcher version must be a semantic x.y.z version')
  }
  return value
}

function required(values, name) {
  const value = values.get(name)
  if (!value) throw new Error(`missing --${name}`)
  return resolve(value)
}

function compareReleases(left, right) {
  const parse = (value) => {
    const separator = value.indexOf('-')
    const core = separator === -1 ? value : value.slice(0, separator)
    const prerelease = separator === -1 ? null : value.slice(separator + 1)
    const numbers = core.split('.').map((part) => Number(part))
    return { numbers, prerelease: prerelease ? prerelease.split('.') : [] }
  }
  const a = parse(left)
  const b = parse(right)
  for (let index = 0; index < 3; index += 1) {
    if (a.numbers[index] !== b.numbers[index]) return a.numbers[index] - b.numbers[index]
  }
  if (a.prerelease.length === 0 && b.prerelease.length > 0) return 1
  if (a.prerelease.length > 0 && b.prerelease.length === 0) return -1
  for (let index = 0; index < Math.max(a.prerelease.length, b.prerelease.length); index += 1) {
    if (index >= a.prerelease.length) return -1
    if (index >= b.prerelease.length) return 1
    const leftPart = a.prerelease[index]
    const rightPart = b.prerelease[index]
    const leftNumeric = /^\d+$/.test(leftPart)
    const rightNumeric = /^\d+$/.test(rightPart)
    if (leftNumeric && rightNumeric && Number(leftPart) !== Number(rightPart)) return Number(leftPart) - Number(rightPart)
    if (leftNumeric !== rightNumeric) return leftNumeric ? -1 : 1
    if (leftPart !== rightPart) return leftPart < rightPart ? -1 : 1
  }
  return 0
}

function validateCatalogEntry(entry, label) {
  if (!entry || !RELEASE_VERSION.test(entry.release ?? '') || !RELEASE_VERSION.test(entry.minimumLauncher ?? '')) {
    throw new Error(`${label} release compatibility is invalid`)
  }
  if (!entry.manifest || typeof entry.manifest.objectKey !== 'string' || !Number.isSafeInteger(entry.manifest.bytes) || entry.manifest.bytes <= 0 || !/^[0-9a-f]{64}$/i.test(entry.manifest.sha256 ?? '')) {
    throw new Error(`${label} manifest asset is invalid`)
  }
}

export function mergeCatalogEntries(existingEntries, currentEntry) {
  const entries = existingEntries.map((entry, index) => {
    validateCatalogEntry(entry, `catalog entry ${index}`)
    return {
      release: entry.release,
      minimumLauncher: entry.minimumLauncher,
      manifest: { ...entry.manifest },
    }
  })
  const seen = new Set()
  for (const entry of entries) {
    if (!seen.add(entry.release)) throw new Error(`catalog input contains duplicate release ${entry.release}`)
  }
  validateCatalogEntry(currentEntry, 'current catalog entry')
  if (seen.has(currentEntry.release)) throw new Error(`catalog already contains release ${currentEntry.release}`)
  entries.push({
    release: currentEntry.release,
    minimumLauncher: currentEntry.minimumLauncher,
    manifest: { ...currentEntry.manifest },
  })
  return entries.sort((left, right) => compareReleases(left.release, right.release))
}

async function readCatalogInput(values) {
  const input = values.get('catalog-input')
  if (!input) return []
  const parsed = JSON.parse(await readFile(resolve(input), 'utf8'))
  if (parsed?.schema !== 1 || parsed.product !== 'atlas-dsh-desktop' || parsed.platform !== 'windows' || parsed.arch !== 'x64' || !Array.isArray(parsed.releases)) {
    throw new Error('catalog input must be a release catalog for atlas-dsh-desktop windows-x64')
  }
  return parsed.releases
}

async function digest(path) {
  const hash = createHash('sha256')
  let bytes = 0
  for await (const chunk of createReadStream(path)) {
    hash.update(chunk)
    bytes += chunk.length
  }
  return { bytes, sha256: hash.digest('hex') }
}

function quotePowerShell(value) {
  return `'${value.replaceAll("'", "''")}'`
}

async function relativeEntries(source, current = '') {
  const directory = join(source, current)
  const entries = []
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const relative = current ? join(current, entry.name) : entry.name
    if (entry.isSymbolicLink()) throw new Error(`runtime closure contains a symbolic link: ${relative}`)
    entries.push(relative.replaceAll('\\', '/'))
    if (entry.isDirectory()) entries.push(...await relativeEntries(source, relative))
  }
  return entries
}

async function archiveDirectory(source, destination) {
  const entries = await relativeEntries(source)
  if (entries.length === 0) throw new Error('runtime closure is empty')
  // Walk every entry above to reject links, but pass only the top-level entries
  // to the archiver so large node_modules trees do not exceed Windows argv limits.
  const topLevelEntries = entries.filter((entry) => !entry.includes('/'))
  const tar = spawnSync('tar.exe', ['-a', '-c', '-f', destination, '-C', source, ...topLevelEntries], {
    cwd: root,
    windowsHide: true,
    stdio: 'inherit',
  })
  if (tar.status === 0) return
  const command = `Compress-Archive -Path ${topLevelEntries.map((entry) => quotePowerShell(join(source, entry))).join(',')} -DestinationPath ${quotePowerShell(destination)} -Force`
  const powershell = spawnSync('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command', command], {
    cwd: root,
    windowsHide: true,
    stdio: 'inherit',
  })
  if (powershell.status !== 0) throw new Error('neither tar.exe nor Compress-Archive could create the runtime archive')
}

async function main() {
  const values = parseArgs(process.argv.slice(2))
  const runtimeRoot = required(values, 'runtime-root')
  const release = values.get('release')
  if (!release || !RELEASE_VERSION.test(release)) {
    throw new Error('release must be a semantic x.y.z version')
  }
  const outputDir = required(values, 'output-dir')
  const minimumLauncherVersion = resolveMinimumLauncherVersion(
    values,
    JSON.parse(await readFile(join(root, 'version.json'), 'utf8')).launcherVersion,
  )
  const metadataPath = join(root, 'src-tauri', 'resources', 'runtime-versions.windows-x64.json')
  const bridgePath = join(root, 'src-tauri', 'resources', 'desktop-bridge.mjs')
  const bridgePatchTemplatePath = join(root, 'src-tauri', 'resources', 'desktop-bridge.patch.yml')
  if (!existsSync(runtimeRoot)) throw new Error(`runtime root does not exist: ${runtimeRoot}`)
  const runtimeMetadata = await readFile(join(runtimeRoot, 'runtime-versions.windows-x64.json'), 'utf8')
  const expectedMetadata = await readFile(metadataPath, 'utf8')
  if (runtimeMetadata !== expectedMetadata) throw new Error('runtime root does not contain the pinned runtime metadata')
  const runtimeBridge = await readFile(join(runtimeRoot, 'desktop-bridge.mjs'), 'utf8')
  const expectedBridge = await readFile(bridgePath, 'utf8')
  if (runtimeBridge !== expectedBridge) throw new Error('runtime root does not contain the pinned desktop bridge')
  const patchTemplate = await readFile(bridgePatchTemplatePath, 'utf8')
  const placeholder = '__DSH_DESKTOP_BRIDGE_MODULE__'
  if (patchTemplate.split(placeholder).length !== 2) throw new Error('desktop bridge patch template must contain one module placeholder')
  const expectedPatch = patchTemplate.replace(placeholder, pathToFileURL(join(runtimeRoot, 'desktop-bridge.mjs')).href)
  const runtimePatch = await readFile(join(runtimeRoot, 'desktop-bridge.patch.yml'), 'utf8')
  if (runtimePatch !== expectedPatch) throw new Error('runtime root does not contain the rendered desktop bridge patch')

  const doctor = spawnSync(process.execPath, [join(root, 'scripts', 'doctor-runtime.mjs'), '--root', runtimeRoot, '--metadata', metadataPath], {
    cwd: root,
    windowsHide: true,
    stdio: 'inherit',
  })
  if (doctor.status !== 0) throw new Error('runtime doctor failed; archive was not created')

  const releaseDir = join(outputDir, 'releases', release, 'windows-x64')
  const archivePath = join(releaseDir, 'runtime.zip')
  const temporaryArchive = `${archivePath}.part.zip`
  await mkdir(releaseDir, { recursive: true })
  await rm(temporaryArchive, { force: true })
  await archiveDirectory(runtimeRoot, temporaryArchive)
  await rename(temporaryArchive, archivePath)
  const asset = await digest(archivePath)
  const manifest = {
    schema: 1,
    product: 'atlas-dsh-desktop',
    release,
    platform: 'windows',
    arch: 'x64',
    minimumLauncher: minimumLauncherVersion,
    components: [{
      id: 'runtime',
      version: release,
      asset: {
        objectKey: `releases/${release}/windows-x64/runtime.zip`,
        ...asset,
      },
      archive: 'zip',
      installRoot: '',
      doctor: { program: 'node/node.exe', args: ['--version'], timeoutSeconds: 30 },
      licenses: ['THIRD_PARTY_NOTICES.md'],
    }],
  }
  const manifestPath = join(releaseDir, 'manifest.json')
  await writeFile(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`, 'utf8')
  const manifestDigest = await digest(manifestPath)
  const catalogEntries = mergeCatalogEntries(await readCatalogInput(values), {
    release,
    minimumLauncher: minimumLauncherVersion,
    manifest: {
      objectKey: `releases/${release}/windows-x64/manifest.json`,
      ...manifestDigest,
    },
  })
  const catalog = {
    schema: 1,
    product: 'atlas-dsh-desktop',
    platform: 'windows',
    arch: 'x64',
    releases: catalogEntries,
  }
  const catalogPath = join(outputDir, 'catalog', release, 'windows-x64', 'catalog.json')
  await mkdir(join(outputDir, 'catalog', release, 'windows-x64'), { recursive: true })
  await writeFile(catalogPath, `${JSON.stringify(catalog, null, 2)}\n`, 'utf8')
  const catalogDigest = await digest(catalogPath)
  const bootstrap = {
    schema: 1,
    product: 'atlas-dsh-desktop',
    platform: 'windows',
    arch: 'x64',
    release,
    minimumLauncher: minimumLauncherVersion,
    manifest: {
      objectKey: `releases/${release}/windows-x64/manifest.json`,
      ...manifestDigest,
    },
    catalog: {
      objectKey: `catalog/${release}/windows-x64/catalog.json`,
      ...catalogDigest,
    },
  }
  const bootstrapPath = join(outputDir, 'bootstrap', 'windows-x64.json')
  await mkdir(join(outputDir, 'bootstrap'), { recursive: true })
  await writeFile(bootstrapPath, `${JSON.stringify(bootstrap, null, 2)}\n`, 'utf8')
  if (values.has('seed-out')) {
    await writeFile(resolve(values.get('seed-out')), `${JSON.stringify(bootstrap, null, 2)}\n`, 'utf8')
  }
  process.stdout.write(`runtime: ${release}\narchive: ${archivePath}\narchive bytes: ${asset.bytes}\narchive sha256: ${asset.sha256}\nmanifest: ${manifestPath}\nbootstrap: ${bootstrapPath}\n`)
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    await main()
  } catch (error) {
    process.stderr.write(`runtime build failed: ${error instanceof Error ? error.message : String(error)}\n`)
    process.exitCode = 1
  }
}
