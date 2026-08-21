import { readFile } from 'node:fs/promises'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = resolve(fileURLToPath(new URL('..', import.meta.url)))
const canonical = JSON.parse(await readFile(resolve(root, 'version.json'), 'utf8')).launcherVersion
const cargo = await readFile(resolve(root, 'src-tauri/Cargo.toml'), 'utf8')
const tauri = JSON.parse(await readFile(resolve(root, 'src-tauri/tauri.conf.json'), 'utf8'))
const cargoVersion = cargo.match(/^version\s*=\s*"([^"]+)"/m)?.[1]
if (!canonical || cargoVersion !== canonical || tauri.version !== canonical) {
  throw new Error(`version drift: version.json=${canonical}, Cargo.toml=${cargoVersion}, tauri.conf.json=${tauri.version}`)
}
process.stdout.write(`launcher version: ${canonical}\n`)
