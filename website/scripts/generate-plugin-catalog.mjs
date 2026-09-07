import { execFileSync } from 'node:child_process'
import { mkdirSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const websiteRoot = fileURLToPath(new URL('..', import.meta.url))
const repositoryRoot = fileURLToPath(new URL('../../', import.meta.url))
const output = join(websiteRoot, 'docs/reference/plugins.md')
const metadata = JSON.parse(execFileSync(
  'cargo',
  ['metadata', '--no-deps', '--format-version', '1'],
  { cwd: repositoryRoot, encoding: 'utf8' },
))

const plugins = metadata.packages
  .filter(({ name }) => name.startsWith('ah-plugins-'))
  .sort((a, b) => a.name.localeCompare(b.name))

const categories = [
  {
    title: '运行时与基础能力',
    prefixes: ['ability', 'agent-control', 'agent-loop', 'application', 'context', 'context-evolver', 'controller', 'json-parser', 'manifest', 'model-', 'pregel', 'prompt', 'runner', 'session-log', 'skill', 'stream', 'tag-manager', 'tokenizer', 'workflow'],
  },
  {
    title: '工具、安全与工作区',
    prefixes: ['code', 'common-tools', 'lsp', 'rails', 'reliability-', 'sandbox', 'security', 'sysop', 'tools', 'workspace', 'worktree'],
  },
  {
    title: '模型与外部协议',
    prefixes: ['a2a', 'anthropic', 'checkpointer', 'credentials', 'external', 'mcp', 'messager', 'oauth', 'openai', 'queue', 'store', 'telemetry', 'tracer-otel', 'transport', 'web'],
  },
  {
    title: '团队与协作',
    prefixes: ['external-format', 'inbound-render', 'interaction-router', 'member-optimizer', 'subagent', 'subagents', 'team-', 'teams'],
  },
  {
    title: '评估、演进与自动化',
    prefixes: ['agentbuilder', 'autoharness', 'bridge-compose', 'data-loader', 'dataset-curator', 'evolving', 'experience-scorer', 'git', 'optimizer', 'operator', 'rsi', 'sharing', 'signals', 'tune', 'trainer'],
  },
]

function categoryFor(name) {
  const shortName = name.slice('ah-plugins-'.length)
  return categories.find(({ prefixes }) => prefixes.some((prefix) => shortName === prefix || shortName.startsWith(prefix)))?.title
    ?? '其他插件'
}

function escapeCell(value) {
  return value
    .replaceAll('|', '\\|')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('{', '&#123;')
    .replaceAll('}', '&#125;')
    .replaceAll('\n', ' ')
}

const grouped = new Map(categories.map(({ title }) => [title, []]))
grouped.set('其他插件', [])
for (const plugin of plugins) grouped.get(categoryFor(plugin.name)).push(plugin)

const lines = [
  '# 内置插件目录',
  '',
  `当前 Workspace 共包含 **${plugins.length} 个 ah-plugins-* 插件 crate**。本页由 scripts/generate-plugin-catalog.mjs 根据 Cargo metadata 生成，避免新增插件遗漏在网站目录中。`,
  '',
  '> 插件的稳定名称、构造方式、提供的 ServiceKey 和 Profile 组合仍以对应 crate、`crates/ah-app/src/lib.rs` 与 `profiles/*.toml` 为准。表格中的 description 来自各插件的 `Cargo.toml`。',
  '',
  '## 如何使用目录',
  '',
  '1. 先按场景找到插件 crate。',
  '2. 阅读对应 `crates/<plugin>/src/lib.rs` 的公共类型和 `Plugin` 实现。',
  '3. 查看 `provides()`、`inject()` 和 `apply()`，确认依赖与注册行为。',
  '4. 在 `ah-app` Catalog 和 Profile 中确认可组合名称。',
  '5. 运行该插件的测试以及 mount、resolve、invoke、unmount 集成测试。',
  '',
]

for (const [title, items] of grouped) {
  if (!items.length) continue
  lines.push(`## ${title}`, '', '| 插件 crate | Cargo 描述 |', '| --- | --- |')
  for (const { name, description } of items) {
    lines.push(`| ${name} | ${escapeCell(description || '未在 Cargo.toml 声明描述。')} |`)
  }
  lines.push('')
}

lines.push(
  '## 从目录进入源码',
  '',
  '插件 crate 的生产依赖应保持在 `ah-contracts`、`ah-hub` 和必要的外部基础库范围内；插件之间禁止直接依赖其他插件的具体类型。插件 API、生命周期和测试方法见：',
  '',
  '- [插件简介与开发](/guide/plugin)',
  '- [插件生命周期与测试](/guide/plugin-lifecycle)',
  '- [Plugin API](/reference/plugin-api)',
  '- [Catalog 与 Profile API](/reference/plugin-catalog)',
  '',
)

mkdirSync(dirname(output), { recursive: true })
writeFileSync(output, `${lines.join('\n')}\n`, 'utf8')
console.log(`generated ${output} with ${plugins.length} plugins`)
