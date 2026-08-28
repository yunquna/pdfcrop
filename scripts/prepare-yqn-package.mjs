import fs from 'node:fs'
import path from 'node:path'

const root = process.cwd()
const outputDirectory = path.resolve(root, process.argv[2] ?? 'pkg-worker')
const packagePath = path.join(outputDirectory, 'package.json')

if (!fs.existsSync(packagePath)) {
  throw new Error(`generated package not found: ${packagePath}`)
}

const manifest = fs.readFileSync(path.join(root, 'Cargo.toml'), 'utf8')
const version = manifest.match(/^version = "([^"]+)"/m)?.[1]
if (!version) {
  throw new Error('Cargo package version not found')
}

const packageJson = JSON.parse(fs.readFileSync(packagePath, 'utf8'))
packageJson.name = '@yunquna/pdfcrop-wasm'
packageJson.version = version
packageJson.license = 'MIT'
packageJson.repository = {
  type: 'git',
  url: 'https://github.com/yunquna/pdfcrop',
}
packageJson.files = [
  ...new Set([
    ...(packageJson.files ?? []),
    'LICENSE-MIT',
    'README.md',
  ]),
]

fs.copyFileSync(
  path.join(root, 'LICENSE'),
  path.join(outputDirectory, 'LICENSE-MIT'),
)
fs.writeFileSync(packagePath, `${JSON.stringify(packageJson, null, 2)}\n`)

console.log(`${packageJson.name}@${packageJson.version}`)
