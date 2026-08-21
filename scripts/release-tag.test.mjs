import assert from 'node:assert/strict'
import test from 'node:test'

import { parseReleaseTagArgs, releaseVersionFromTag } from './release-tag.mjs'

test('uses the v-prefixed tag as the release version', () => {
  assert.equal(releaseVersionFromTag('v0.1.2'), '0.1.2')
  assert.equal(parseReleaseTagArgs(['--tag', 'v1.2.3-rc.1']), '1.2.3-rc.1')
})

test('rejects malformed release tags', () => {
  assert.throws(() => releaseVersionFromTag('0.1.2'), /vX.Y.Z/)
  assert.throws(() => parseReleaseTagArgs(['--tag', 'v1.2']), /vX.Y.Z/)
  assert.throws(() => parseReleaseTagArgs([]), /usage/)
})
