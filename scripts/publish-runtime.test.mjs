import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { mkdtemp, mkdir, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import test from 'node:test'

import { BOOTSTRAP_KEY, loadPublication, safeObjectKey } from './publish-runtime.mjs'

function digest(value) {
  return createHash('sha256').update(value).digest('hex')
}

async function fixture({ release = '0.1.3', minimumLauncher = '0.1.1' } = {}) {
  const output = await mkdtemp(join(tmpdir(), 'dsh-runtime-publish-'))
  const archive = Buffer.from('verified runtime archive')
  const archiveKey = `releases/${release}/windows-x64/runtime.zip`
  const archivePath = join(output, ...archiveKey.split('/'))
  await mkdir(join(output, 'releases', release, 'windows-x64'), { recursive: true })
  await writeFile(archivePath, archive)
  const manifest = {
    schema: 1, product: 'atlas-dsh-desktop', release, platform: 'windows', arch: 'x64', minimumLauncher,
    components: [{
      id: 'runtime', version: release,
      asset: { objectKey: archiveKey, bytes: archive.length, sha256: digest(archive) },
      archive: 'zip', installRoot: '', doctor: { program: 'node/node.exe', args: ['--version'], timeoutSeconds: 30 }, licenses: ['THIRD_PARTY_NOTICES.md'],
    }],
  }
  const manifestBytes = Buffer.from(`${JSON.stringify(manifest, null, 2)}\n`)
  const manifestKey = `releases/${release}/windows-x64/manifest.json`
  await writeFile(join(output, ...manifestKey.split('/')), manifestBytes)
  const bootstrap = {
    schema: 1, product: 'atlas-dsh-desktop', platform: 'windows', arch: 'x64', release, minimumLauncher,
    manifest: { objectKey: manifestKey, bytes: manifestBytes.length, sha256: digest(manifestBytes) },
  }
  await mkdir(join(output, 'bootstrap'), { recursive: true })
  await writeFile(join(output, ...BOOTSTRAP_KEY.split('/')), `${JSON.stringify(bootstrap, null, 2)}\n`)
  return { output, manifestPath: join(output, ...manifestKey.split('/')), bootstrapPath: join(output, ...BOOTSTRAP_KEY.split('/')) }
}

test('accepts a self-consistent runtime publication closure', async () => {
  const { output } = await fixture()
  const publication = await loadPublication(output)
  assert.equal(publication.archive.key, 'releases/0.1.3/windows-x64/runtime.zip')
  assert.equal(publication.manifest.identity.bytes > 0, true)
})

test('accepts a runtime release above the minimum compatible Launcher', async () => {
  const { output } = await fixture({ release: '0.1.3', minimumLauncher: '0.1.1' })
  const publication = await loadPublication(output)
  assert.equal(publication.archive.key, 'releases/0.1.3/windows-x64/runtime.zip')
})

test('rejects a manifest whose runtime digest does not match its archive', async () => {
  const { output, manifestPath, bootstrapPath } = await fixture()
  const manifest = JSON.parse(await (await import('node:fs/promises')).readFile(manifestPath, 'utf8'))
  manifest.components[0].asset.sha256 = '0'.repeat(64)
  const manifestBytes = Buffer.from(`${JSON.stringify(manifest, null, 2)}\n`)
  await writeFile(manifestPath, manifestBytes)
  const bootstrap = JSON.parse(await (await import('node:fs/promises')).readFile(bootstrapPath, 'utf8'))
  bootstrap.manifest.bytes = manifestBytes.length
  bootstrap.manifest.sha256 = digest(manifestBytes)
  await writeFile(bootstrapPath, `${JSON.stringify(bootstrap, null, 2)}\n`)
  await assert.rejects(loadPublication(output), /archive bytes or SHA-256/)
})

test('accepts only safe OSS object keys', () => {
  assert.equal(safeObjectKey('releases/0.1.0/windows-x64/runtime.zip'), true)
  assert.equal(safeObjectKey('../bootstrap/windows-x64.json'), false)
  assert.equal(safeObjectKey('/bootstrap/windows-x64.json'), false)
  assert.equal(safeObjectKey('bootstrap\\windows-x64.json'), false)
})
