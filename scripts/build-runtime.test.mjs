import assert from 'node:assert/strict'
import test from 'node:test'

import { mergeCatalogEntries, resolveMinimumLauncherVersion } from './build-runtime.mjs'

const asset = (release) => ({
  objectKey: `releases/${release}/windows-x64/manifest.json`,
  bytes: 10,
  sha256: 'a'.repeat(64),
})

test('defaults runtime compatibility to the stable Launcher version', () => {
  assert.equal(resolveMinimumLauncherVersion(new Map(), '0.1.1'), '0.1.1')
})

test('allows a runtime release to use an older compatible Launcher', () => {
  assert.equal(
    resolveMinimumLauncherVersion(new Map([['minimum-launcher-version', '0.1.1']]), '0.1.1'),
    '0.1.1',
  )
})

test('keeps the legacy launcher-version option compatible', () => {
  assert.equal(resolveMinimumLauncherVersion(new Map([['launcher-version', '0.1.1']]), '0.1.3'), '0.1.1')
})

test('rejects conflicting compatibility options', () => {
  assert.throws(
    () => resolveMinimumLauncherVersion(
      new Map([
        ['minimum-launcher-version', '0.1.1'],
        ['launcher-version', '0.1.3'],
      ]),
      '0.1.1',
    ),
    /must match/,
  )
})

test('rejects invalid compatibility versions', () => {
  assert.throws(
    () => resolveMinimumLauncherVersion(new Map([['minimum-launcher-version', 'latest']]), '0.1.1'),
    /semantic x\.y\.z/,
  )
})

test('merges an immutable catalog and orders releases semantically', () => {
  const entries = mergeCatalogEntries([
    { release: '0.1.1', minimumLauncher: '0.1.0', manifest: asset('0.1.1') },
    { release: '0.1.0', minimumLauncher: '0.1.0', manifest: asset('0.1.0') },
  ], { release: '0.1.2-rc.1', minimumLauncher: '0.1.1', manifest: asset('0.1.2-rc.1') })
  assert.deepEqual(entries.map((entry) => entry.release), ['0.1.0', '0.1.1', '0.1.2-rc.1'])
})

test('rejects duplicate releases when extending an immutable catalog', () => {
  assert.throws(
    () => mergeCatalogEntries([{ release: '0.1.1', minimumLauncher: '0.1.0', manifest: asset('0.1.1') }], {
      release: '0.1.1', minimumLauncher: '0.1.1', manifest: asset('0.1.1'),
    }),
    /already contains release/,
  )
})
