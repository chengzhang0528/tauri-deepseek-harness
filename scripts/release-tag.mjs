import { spawnSync } from 'node:child_process'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const TAG = /^v(\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?)$/

export function releaseVersionFromTag(tag) {
  const match = typeof tag === 'string' ? TAG.exec(tag) : null
  if (!match) throw new Error('tag must be vX.Y.Z')
  return match[1]
}

export function parseReleaseTagArgs(argv) {
  if (argv.length !== 2 || argv[0] !== '--tag') {
    throw new Error('usage: release-tag.mjs --tag vX.Y.Z')
  }
  return releaseVersionFromTag(argv[1])
}

function main() {
  const releaseVersion = parseReleaseTagArgs(process.argv.slice(2))
  const result = spawnSync(process.execPath, [
    join(root, 'scripts', 'build-msi.mjs'),
    '--release-version',
    releaseVersion,
  ], {
    cwd: root,
    env: { ...process.env, DSH_RELEASE_VERSION: releaseVersion },
    stdio: 'inherit',
  })
  if (result.error) throw result.error
  process.exit(result.status ?? 1)
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    main()
  } catch (error) {
    process.stderr.write(`release tag build failed: ${error instanceof Error ? error.message : String(error)}\n`)
    process.exitCode = 1
  }
}
