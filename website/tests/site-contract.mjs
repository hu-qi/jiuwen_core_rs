import { execFileSync } from 'node:child_process'
import { existsSync, readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { join } from 'node:path'

const websiteRoot = fileURLToPath(new URL('../', import.meta.url))
const root = join(websiteRoot, 'docs')
const repositoryRoot = fileURLToPath(new URL('../../', import.meta.url))
const required = [
  'index.md',
  'guide/index.md',
  'guide/harness.md',
  'guide/plugin.md',
  'guide/cli-example.md',
  'guide/plugins.md',
  'guide/profile.md',
  'guide/events.md',
  'guide/plugin-lifecycle.md',
  'concepts/index.md',
  'concepts/services.md',
  'concepts/events.md',
  'integrations/index.md',
  'reference/index.md',
  'reference/plugins.md',
  'reference/plugin-api.md',
  'reference/plugin-catalog.md',
  'operations/index.md',
  'contributing/index.md',
]

const missing = required.filter((file) => !existsSync(join(root, file)))
if (missing.length) {
  console.error(`Missing required Harness pages:\n${missing.join('\n')}`)
  process.exit(1)
}

const files = required.map((file) => readFileSync(join(root, file), 'utf8')).join('\n')
for (const forbidden of ['agent-core', 'agent-core_rs', 'Python SDK', 'openjiuwen-examples']) {
  if (files.includes(forbidden)) {
    console.error(`Harness site contains forbidden cross-project content: ${forbidden}`)
    process.exit(1)
  }
}

const metadata = JSON.parse(execFileSync(
  'cargo',
  ['metadata', '--no-deps', '--format-version', '1'],
  { cwd: repositoryRoot, encoding: 'utf8' },
))
const allPluginPackages = metadata.packages.filter(({ name }) => name.startsWith('ah-plugins-'))
const exampleRoot = `${join(repositoryRoot, 'example')}/`
const pluginNames = allPluginPackages
  .filter(({ manifest_path }) => !manifest_path.startsWith(exampleRoot))
  .map(({ name }) => name)
const examplePluginNames = allPluginPackages
  .filter(({ manifest_path }) => manifest_path.startsWith(exampleRoot))
  .map(({ name }) => name)
const catalog = readFileSync(join(root, 'reference/plugins.md'), 'utf8')
const missingPlugins = pluginNames.filter((name) => !catalog.includes(`| ${name} |`))
if (missingPlugins.length) {
  console.error(`Plugin catalog is missing ${missingPlugins.length} workspace plugins:\n${missingPlugins.join('\n')}`)
  process.exit(1)
}

const misplacedExamplePlugins = examplePluginNames.filter((name) => catalog.includes(`| ${name} |`))
if (misplacedExamplePlugins.length) {
  console.error(`Example-only plugins must not be in the built-in catalog:\n${misplacedExamplePlugins.join('\n')}`)
  process.exit(1)
}

const home = readFileSync(join(root, 'index.md'), 'utf8')
if (!home.includes('agent-harness')) {
  console.error('Homepage does not name agent-harness')
  process.exit(1)
}

console.log(`Harness site contract passed: ${required.length} pages and ${pluginNames.length} plugins`)
