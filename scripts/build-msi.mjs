import { spawnSync } from 'node:child_process'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const args = ['build', '--bundles', 'msi', '--target', 'x86_64-pc-windows-gnu']
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
