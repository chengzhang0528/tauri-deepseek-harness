import { spawnSync } from 'node:child_process'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const releaseVersion = parseReleaseVersion(process.argv.slice(2))
const versionCheckArgs = [join(root, 'scripts', 'check-version.mjs')]
if (releaseVersion) versionCheckArgs.push('--release-version', releaseVersion)
const versionCheck = spawnSync(process.execPath, versionCheckArgs, {
  cwd: root,
  stdio: 'inherit',
})
if (versionCheck.status !== 0) process.exit(versionCheck.status ?? 1)
const args = ['build', '--bundles', 'msi', '--target', 'x86_64-pc-windows-gnu']
if (releaseVersion) args.push('--config', JSON.stringify({ version: releaseVersion }))
const wixBin = join(root, '.tools', 'wix314', 'tools', 'bin')
const cli = join(root, 'node_modules', '@tauri-apps', 'cli', 'tauri.js')
const prepare = spawnSync(process.execPath, [join(root, 'scripts', 'ensure-wix.mjs')], {
  cwd: root,
  stdio: 'inherit',
})
if (prepare.status !== 0) process.exit(prepare.status ?? 1)

const env = {
  ...process.env,
  PATH: `${wixBin}${process.platform === 'win32' ? ';' : ':'}${process.env.PATH ?? ''}`,
  RUSTUP_TOOLCHAIN: 'stable-x86_64-pc-windows-gnu',
}
if (releaseVersion) env.DSH_RELEASE_VERSION = releaseVersion
else delete env.DSH_RELEASE_VERSION
const result = spawnSync(process.execPath, [cli, ...args], {
  cwd: root,
  env,
  stdio: 'inherit',
})

if (result.error) {
  console.error(`无法启动 Tauri CLI：${result.error.message}`)
  process.exit(1)
}

process.exit(result.status ?? 1)

function parseReleaseVersion(argv) {
  if (argv.length === 0) return null
  if (argv.length !== 2 || argv[0] !== '--release-version' || !/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(argv[1])) {
    throw new Error('usage: build-msi.mjs [--release-version x.y.z]')
  }
  return argv[1]
}
