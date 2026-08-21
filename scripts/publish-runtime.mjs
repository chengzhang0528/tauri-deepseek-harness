import { createHmac, createHash } from 'node:crypto'
import { createReadStream } from 'node:fs'
import { readFile, stat } from 'node:fs/promises'
import { join, resolve } from 'node:path'
import { Readable } from 'node:stream'
import { fileURLToPath } from 'node:url'

export const OSS_BUCKET = 'shared-public-assets'
export const OSS_ORIGIN = new URL('https://shared-public-assets.oss-cn-beijing.aliyuncs.com/atlas-dsh-desktop/')
export const BOOTSTRAP_KEY = 'bootstrap/windows-x64.json'
const PRODUCT = 'atlas-dsh-desktop'
const PLATFORM = 'windows'
const ARCH = 'x64'
const SHA256 = /^[0-9a-f]{64}$/i
const VERSION = /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/

function fail(message) {
  throw new Error(message)
}

function required(value, label) {
  if (!value) fail(`${label} is required`)
  return value
}

export function safeObjectKey(key) {
  return typeof key === 'string'
    && key.length > 0
    && !key.startsWith('/')
    && !key.includes('\\')
    && !key.split('/').some((part) => !part || part === '.' || part === '..' || /[:?*"<>|]/.test(part))
}

function objectUrl(key) {
  if (!safeObjectKey(key)) fail(`unsafe OSS object key: ${key}`)
  return new URL(key, OSS_ORIGIN)
}

function parseArgs(argv) {
  const values = new Map()
  for (let index = 0; index < argv.length; index += 1) {
    const key = argv[index]
    const value = argv[index + 1]
    if (key !== '--output-dir' || !value || value.startsWith('--') || values.has(key)) {
      fail(`usage: publish-runtime.mjs --output-dir <directory>`)
    }
    values.set(key, value)
    index += 1
  }
  return { outputDir: resolve(required(values.get('--output-dir'), '--output-dir')) }
}

async function digestFile(path) {
  const hash = createHash('sha256')
  let bytes = 0
  for await (const chunk of createReadStream(path)) {
    hash.update(chunk)
    bytes += chunk.length
  }
  return { bytes, sha256: hash.digest('hex') }
}

async function readJson(path, label) {
  try {
    return JSON.parse(await readFile(path, 'utf8'))
  } catch (error) {
    fail(`cannot read ${label}: ${error instanceof Error ? error.message : String(error)}`)
  }
}

function assertTarget(value, label) {
  if (value !== PRODUCT && label === 'product') fail(`unexpected ${label}: ${value}`)
  if (value !== PLATFORM && label === 'platform') fail(`unexpected ${label}: ${value}`)
  if (value !== ARCH && label === 'arch') fail(`unexpected ${label}: ${value}`)
}

function assertAsset(asset, label) {
  if (!asset || !safeObjectKey(asset.objectKey) || !Number.isSafeInteger(asset.bytes) || asset.bytes <= 0 || !SHA256.test(asset.sha256 ?? '')) {
    fail(`${label} is invalid`)
  }
}

function assertRelease(value, label) {
  if (!VERSION.test(value ?? '')) fail(`${label} is invalid`)
}

function resolveOutputFile(outputDir, objectKey) {
  const path = resolve(outputDir, ...objectKey.split('/'))
  if (!path.startsWith(`${outputDir}\\`) && path !== outputDir) fail(`object path escapes output directory: ${objectKey}`)
  return path
}

export async function loadPublication(outputDir) {
  const bootstrapPath = join(outputDir, ...BOOTSTRAP_KEY.split('/'))
  const bootstrap = await readJson(bootstrapPath, 'runtime bootstrap')
  if (bootstrap?.schema !== 1) fail('runtime bootstrap schema is unsupported')
  for (const [label, value] of Object.entries({ product: bootstrap.product, platform: bootstrap.platform, arch: bootstrap.arch })) {
    assertTarget(value, label)
  }
  assertRelease(bootstrap.release, 'runtime bootstrap release')
  assertRelease(bootstrap.minimumLauncher, 'runtime bootstrap minimum launcher')
  assertAsset(bootstrap.manifest, 'runtime bootstrap manifest')

  const manifestPath = resolveOutputFile(outputDir, bootstrap.manifest.objectKey)
  const manifest = await readJson(manifestPath, 'runtime manifest')
  if (manifest?.schema !== 1) fail('runtime manifest schema is unsupported')
  for (const [label, value] of Object.entries({ product: manifest.product, platform: manifest.platform, arch: manifest.arch })) {
    assertTarget(value, label)
  }
  if (manifest.release !== bootstrap.release || manifest.minimumLauncher !== bootstrap.minimumLauncher) {
    fail('runtime manifest does not match bootstrap release compatibility')
  }
  if (!Array.isArray(manifest.components) || manifest.components.length !== 1) fail('runtime manifest must contain one runtime component')
  const component = manifest.components[0]
  if (component.id !== 'runtime' || component.version !== manifest.release || component.archive !== 'zip' || component.installRoot !== '') {
    fail('runtime manifest component is invalid')
  }
  assertAsset(component.asset, 'runtime archive asset')
  if (component.doctor?.program !== 'node/node.exe' || component.doctor?.timeoutSeconds !== 30) {
    fail('runtime manifest doctor is invalid')
  }

  const [bootstrapIdentity, manifestIdentity, archiveIdentity] = await Promise.all([
    digestFile(bootstrapPath),
    digestFile(manifestPath),
    digestFile(resolveOutputFile(outputDir, component.asset.objectKey)),
  ])
  if (manifestIdentity.bytes !== bootstrap.manifest.bytes || manifestIdentity.sha256 !== bootstrap.manifest.sha256.toLowerCase()) {
    fail('runtime manifest bytes or SHA-256 do not match bootstrap')
  }
  if (archiveIdentity.bytes !== component.asset.bytes || archiveIdentity.sha256 !== component.asset.sha256.toLowerCase()) {
    fail('runtime archive bytes or SHA-256 do not match manifest')
  }

  return {
    bootstrap: { key: BOOTSTRAP_KEY, path: bootstrapPath, identity: bootstrapIdentity, contentType: 'application/json' },
    manifest: { key: bootstrap.manifest.objectKey, path: manifestPath, identity: manifestIdentity, contentType: 'application/json' },
    archive: { key: component.asset.objectKey, path: resolveOutputFile(outputDir, component.asset.objectKey), identity: archiveIdentity, contentType: 'application/zip' },
  }
}

function credentials(environment) {
  return {
    accessKeyId: required(environment.ALIYUN_OSS_ACCESS_KEY_ID, 'ALIYUN_OSS_ACCESS_KEY_ID'),
    accessKeySecret: required(environment.ALIYUN_OSS_ACCESS_KEY_SECRET, 'ALIYUN_OSS_ACCESS_KEY_SECRET'),
  }
}

function signedHeaders(method, key, contentType, credentialsValue, immutable = false) {
  const date = new Date().toUTCString()
  const canonicalHeaders = immutable ? 'x-oss-forbid-overwrite:true\n' : ''
  const canonicalResource = `/${OSS_BUCKET}${OSS_ORIGIN.pathname.replace(/\/$/, '')}/${key}`
  const payload = `${method}\n\n${contentType}\n${date}\n${canonicalHeaders}${canonicalResource}`
  const signature = createHmac('sha1', credentialsValue.accessKeySecret).update(payload).digest('base64')
  const headers = {
    Authorization: `OSS ${credentialsValue.accessKeyId}:${signature}`,
    Date: date,
    'Content-Type': contentType,
    'User-Agent': 'DSH-Desktop-Release',
  }
  if (immutable) headers['x-oss-forbid-overwrite'] = 'true'
  return headers
}

async function responseError(response, action) {
  const detail = (await response.text()).slice(0, 1024).replace(/\s+/g, ' ').trim()
  fail(`${action} returned HTTP ${response.status}${detail ? `: ${detail}` : ''}`)
}

async function readPublicBytes(key, allowMissing = false) {
  const response = await fetch(objectUrl(key), { headers: { 'User-Agent': 'DSH-Desktop-Release' } })
  if (response.status === 404 && allowMissing) return null
  if (!response.ok) await responseError(response, `public read of ${key}`)
  return Buffer.from(await response.arrayBuffer())
}

async function readPublicIdentity(key) {
  const response = await fetch(objectUrl(key), { headers: { 'User-Agent': 'DSH-Desktop-Release' } })
  if (!response.ok) await responseError(response, `public read of ${key}`)
  if (!response.body) fail(`public read of ${key} has no response body`)
  const hash = createHash('sha256')
  let bytes = 0
  const reader = response.body.getReader()
  while (true) {
    const item = await reader.read()
    if (item.done) break
    hash.update(item.value)
    bytes += item.value.byteLength
  }
  return { bytes, sha256: hash.digest('hex') }
}

async function verifyPublicFile(asset) {
  const actual = await readPublicIdentity(asset.key)
  if (actual.bytes !== asset.identity.bytes || actual.sha256 !== asset.identity.sha256) {
    fail(`public ${asset.key} differs from the verified local artifact`)
  }
}

async function putFile(asset, credentialsValue, immutable) {
  const headers = signedHeaders('PUT', asset.key, asset.contentType, credentialsValue, immutable)
  headers['Content-Length'] = String((await stat(asset.path)).size)
  const response = await fetch(objectUrl(asset.key), {
    method: 'PUT',
    headers,
    body: Readable.toWeb(createReadStream(asset.path)),
    duplex: 'half',
  })
  if (!response.ok && !(immutable && response.status === 409)) {
    await responseError(response, `upload of ${asset.key}`)
  }
}

async function putBytes(key, bytes, contentType, credentialsValue) {
  const response = await fetch(objectUrl(key), {
    method: 'PUT',
    headers: signedHeaders('PUT', key, contentType, credentialsValue),
    body: bytes,
  })
  if (!response.ok) await responseError(response, `upload of ${key}`)
}

async function deleteObject(key, credentialsValue) {
  const response = await fetch(objectUrl(key), {
    method: 'DELETE',
    headers: signedHeaders('DELETE', key, '', credentialsValue),
  })
  if (!response.ok && response.status !== 404) await responseError(response, `deletion of ${key}`)
}

async function rollbackBootstrap(previous, credentialsValue) {
  if (previous) {
    await putBytes(BOOTSTRAP_KEY, previous, 'application/json', credentialsValue)
    const restored = await readPublicBytes(BOOTSTRAP_KEY)
    if (!restored.equals(previous)) fail('bootstrap rollback confirmation differs from the previous pointer')
    return 'restored-previous-bootstrap'
  }
  await deleteObject(BOOTSTRAP_KEY, credentialsValue)
  if (await readPublicBytes(BOOTSTRAP_KEY, true)) fail('bootstrap removal was not confirmed')
  return 'removed-new-bootstrap'
}

export async function publish(outputDir, environment = process.env) {
  const publication = await loadPublication(outputDir)
  const credentialsValue = credentials(environment)
  const previousBootstrap = await readPublicBytes(BOOTSTRAP_KEY, true)

  for (const asset of [publication.archive, publication.manifest]) {
    await putFile(asset, credentialsValue, true)
    await verifyPublicFile(asset)
  }

  try {
    await putFile(publication.bootstrap, credentialsValue, false)
    const committed = await readPublicBytes(BOOTSTRAP_KEY)
    if (!committed.equals(await readFile(publication.bootstrap.path))) {
      fail('public bootstrap differs from the verified local bootstrap')
    }
  } catch (error) {
    const rollback = await rollbackBootstrap(previousBootstrap, credentialsValue)
    fail(`bootstrap publication failed and rollback ${rollback}: ${error instanceof Error ? error.message : String(error)}`)
  }

  return {
    archive: { key: publication.archive.key, ...publication.archive.identity },
    manifest: { key: publication.manifest.key, ...publication.manifest.identity },
    bootstrap: { key: publication.bootstrap.key, ...publication.bootstrap.identity },
  }
}

async function main() {
  const { outputDir } = parseArgs(process.argv.slice(2))
  process.stdout.write(`${JSON.stringify(await publish(outputDir))}\n`)
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    process.stderr.write(`runtime publication failed: ${error instanceof Error ? error.message : String(error)}\n`)
    process.exitCode = 1
  })
}
