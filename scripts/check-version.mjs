import { readFile } from 'node:fs/promises'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = resolve(fileURLToPath(new URL('..', import.meta.url)))
export const RELEASE_VERSION = /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/

export function parseReleaseVersion(argv) {
  if (argv.length === 0) return null
  if (argv.length !== 2 || argv[0] !== '--release-version' || !RELEASE_VERSION.test(argv[1])) {
    throw new Error('usage: check-version.mjs [--release-version x.y.z]')
  }
  return argv[1]
}

const requestedRelease = parseReleaseVersion(process.argv.slice(2))
const canonical = JSON.parse(await readFile(resolve(root, 'version.json'), 'utf8')).launcherVersion
const cargo = await readFile(resolve(root, 'src-tauri/Cargo.toml'), 'utf8')
const tauri = JSON.parse(await readFile(resolve(root, 'src-tauri/tauri.conf.json'), 'utf8'))
const cargoVersion = cargo.match(/^version\s*=\s*"([^"]+)"/m)?.[1]
if (!RELEASE_VERSION.test(canonical) || cargoVersion !== canonical || tauri.version !== canonical) {
  throw new Error(`version drift: version.json=${canonical}, Cargo.toml=${cargoVersion}, tauri.conf.json=${tauri.version}`)
}
process.stdout.write(`launcher version: ${requestedRelease ?? canonical}\n`)
