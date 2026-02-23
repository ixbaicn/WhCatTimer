const { execFileSync, spawnSync } = require('node:child_process')
const fs = require('node:fs')
const path = require('node:path')

function stripWrapQuotes(v) {
  return String(v || '').trim().replace(/^['"]+|['"]+$/g, '')
}

function findLatestDir(root) {
  if (!root || !fs.existsSync(root)) return null
  const dirs = fs
    .readdirSync(root, { withFileTypes: true })
    .filter((d) => d.isDirectory())
    .map((d) => d.name)
    .sort((a, b) => a.localeCompare(b, undefined, { numeric: true, sensitivity: 'base' }))
  if (!dirs.length) return null
  return path.join(root, dirs[dirs.length - 1])
}

function getVsInstallPath() {
  if (process.platform !== 'win32') return ''
  const fromEnv = stripWrapQuotes(process.env.VSINSTALLDIR || '')
  if (fromEnv && fs.existsSync(fromEnv)) return fromEnv

  const candidates = [
    'C:\\Program Files\\Microsoft Visual Studio\\2022\\Community',
    'C:\\Program Files\\Microsoft Visual Studio\\2022\\Professional',
    'C:\\Program Files\\Microsoft Visual Studio\\2022\\Enterprise',
    'C:\\Program Files\\Microsoft Visual Studio\\2022\\BuildTools',
  ]
  for (const c of candidates) {
    if (fs.existsSync(c)) return c
  }

  const pf86 = process.env['ProgramFiles(x86)'] || 'C:\\Program Files (x86)'
  const vswhere = path.join(pf86, 'Microsoft Visual Studio', 'Installer', 'vswhere.exe')
  if (!fs.existsSync(vswhere)) return ''
  try {
    const out = execFileSync(
      vswhere,
      [
        '-latest',
        '-products',
        '*',
        '-requires',
        'Microsoft.VisualStudio.Component.VC.Tools.x86.x64',
        '-property',
        'installationPath',
      ],
      { encoding: 'utf8' },
    )
    return stripWrapQuotes(out)
  } catch {
    return ''
  }
}

function findLinker(installPath) {
  if (process.platform !== 'win32') return null

  try {
    const found = execFileSync('where', ['link.exe'], { encoding: 'utf8' })
      .split(/\r?\n/)
      .map((v) => stripWrapQuotes(v))
      .find((v) => v && fs.existsSync(v))
    if (found) return found
  } catch {}

  if (!installPath) return null
  const latestMsvc = findLatestDir(path.join(installPath, 'VC', 'Tools', 'MSVC'))
  if (!latestMsvc) return null
  const linker = path.join(latestMsvc, 'bin', 'Hostx64', 'x64', 'link.exe')
  return fs.existsSync(linker) ? linker : null
}

function importCmdEnvironment(env, batPath, args) {
  const cleanBat = stripWrapQuotes(batPath)
  const cmdExpr = `call ""${cleanBat}"" ${args || ''} >nul && set`
  const dump = execFileSync('cmd.exe', ['/d', '/c', cmdExpr], {
    encoding: 'utf8',
    maxBuffer: 16 * 1024 * 1024,
    windowsHide: true,
  })
  for (const line of dump.split(/\r?\n/)) {
    const idx = line.indexOf('=')
    if (idx <= 0) continue
    env[line.slice(0, idx)] = line.slice(idx + 1)
  }
}

function setupMsvcEnvironment(env, installPath) {
  if (process.platform !== 'win32' || !installPath) return false
  const scripts = [
    [path.join(installPath, 'Common7', 'Tools', 'VsDevCmd.bat'), '-no_logo -arch=x64 -host_arch=x64'],
    [path.join(installPath, 'VC', 'Auxiliary', 'Build', 'vcvars64.bat'), ''],
    [path.join(installPath, 'VC', 'Auxiliary', 'Build', 'vcvarsall.bat'), 'x64'],
  ]

  for (const [bat, args] of scripts) {
    if (!fs.existsSync(bat)) continue
    try {
      importCmdEnvironment(env, bat, args)
      console.log(`[whcat-timer] loaded MSVC env: ${bat}`)
      return true
    } catch {
      console.warn(`[whcat-timer] failed to load: ${bat}`)
    }
  }
  return false
}

function findKernel32LibParents() {
  const roots = [
    path.join(process.env['ProgramFiles(x86)'] || 'C:\\Program Files (x86)', 'Windows Kits', '10', 'Lib'),
    path.join(process.env.ProgramFiles || 'C:\\Program Files', 'Windows Kits', '10', 'Lib'),
  ]
  const out = []
  for (const root of roots) {
    if (!fs.existsSync(root)) continue
    const latest = findLatestDir(root)
    if (latest) {
      const um = path.join(latest, 'um', 'x64')
      if (fs.existsSync(path.join(um, 'kernel32.lib'))) out.push(um)
      const ucrt = path.join(latest, 'ucrt', 'x64')
      if (fs.existsSync(ucrt)) out.push(ucrt)
      const shared = path.join(latest, 'shared', 'x64')
      if (fs.existsSync(shared)) out.push(shared)
    }
  }
  return [...new Set(out)]
}

function ensureWindowsLibEnv(env, installPath, linker) {
  if (process.platform !== 'win32') return false
  const libs = []

  if (linker) {
    const msvcRoot = path.resolve(linker, '..', '..', '..', '..')
    const msvcLib = path.join(msvcRoot, 'lib', 'x64')
    if (fs.existsSync(msvcLib)) libs.push(msvcLib)
  } else if (installPath) {
    const latestMsvc = findLatestDir(path.join(installPath, 'VC', 'Tools', 'MSVC'))
    if (latestMsvc) {
      const msvcLib = path.join(latestMsvc, 'lib', 'x64')
      if (fs.existsSync(msvcLib)) libs.push(msvcLib)
    }
  }

  libs.push(...findKernel32LibParents())

  if (env.WindowsSdkDir && env.WindowsSDKLibVersion) {
    const ver = stripWrapQuotes(env.WindowsSDKLibVersion)
    for (const seg of ['um', 'ucrt', 'shared']) {
      const p = path.join(stripWrapQuotes(env.WindowsSdkDir), 'Lib', ver, seg, 'x64')
      if (fs.existsSync(p)) libs.push(p)
    }
  }

  const dedup = [...new Set(libs.filter(Boolean))]
  if (!dedup.length) return false
  env.LIB = [...dedup, ...(env.LIB || '').split(';').filter(Boolean)].join(';')
  console.log(`[whcat-timer] configured LIB with ${dedup.length} path(s)`)
  return true
}

function hasKernel32Visible(env) {
  return (env.LIB || '')
    .split(';')
    .filter(Boolean)
    .some((p) => fs.existsSync(path.join(p, 'kernel32.lib')))
}

const env = { ...process.env }
const installPath = getVsInstallPath()
const linker = findLinker(installPath)

const loaded = setupMsvcEnvironment(env, installPath)
const configured = ensureWindowsLibEnv(env, installPath, linker)

if (process.platform === 'win32') {
  const visible = hasKernel32Visible(env)
  console.log(`[whcat-timer] kernel32.lib visible via LIB: ${visible}`)
  if (!visible) {
    console.error('[whcat-timer] kernel32.lib not found.')
    console.error(
      '[whcat-timer] Install Visual Studio components: Desktop development with C++ + Windows 10/11 SDK.',
    )
    console.error(
      '[whcat-timer] Example: vs_installer.exe modify --installPath "C:\\Program Files\\Microsoft Visual Studio\\2022\\Community" --add Microsoft.VisualStudio.Workload.NativeDesktop --add Microsoft.VisualStudio.Component.Windows11SDK.22621 --passive --norestart',
    )
    process.exit(1)
  }
  if (!loaded && !configured) {
    console.warn('[whcat-timer] MSVC env injection was not successful')
  }
}

if (linker) {
  env.CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER = linker
  env.Path = `${path.dirname(linker)};${env.Path || env.PATH || ''}`
  env.PATH = env.Path
  console.log(`[whcat-timer] using linker: ${linker}`)
} else if (process.platform === 'win32') {
  console.warn('[whcat-timer] link.exe unresolved, install Visual Studio C++ Build Tools')
}

const cliJs = path.join(process.cwd(), 'node_modules', '@napi-rs', 'cli', 'dist', 'cli.js')
const args = process.argv.slice(2)
const result = fs.existsSync(cliJs)
  ? spawnSync(process.execPath, [cliJs, ...args], { stdio: 'inherit', env })
  : spawnSync('napi', args, { stdio: 'inherit', shell: true, env })

process.exit(result.status ?? 1)
