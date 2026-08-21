import assert from 'node:assert/strict'
import test from 'node:test'

import { resolveMinimumLauncherVersion } from './build-runtime.mjs'

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
