import { defineConfig } from 'vitepress'

export default defineConfig({
  title: 'agent-harness',
  description: 'A composable, observable, reversible Rust Agent Runtime.',
  lang: 'zh-CN',
  cleanUrls: true,
  lastUpdated: true,
  head: [
    ['meta', { name: 'theme-color', content: '#10100f' }],
    ['meta', { name: 'color-scheme', content: 'dark light' }],
  ],
  themeConfig: {
    logo: '/mark.svg',
    siteTitle: 'agent-harness / docs',
    outline: { level: [2, 3], label: '本页内容' },
    returnToTopLabel: '返回顶部',
    sidebarMenuLabel: '目录',
    darkModeSwitchLabel: '主题',
    docFooter: { prev: '上一页', next: '下一页' },
    lastUpdated: { text: '最后更新' },
    socialLinks: [
      { icon: 'github', link: 'https://github.com/openjiuwen' },
    ],
    search: {
      provider: 'local',
      options: {
        translations: {
          button: { buttonText: '搜索文档', buttonAriaLabel: '搜索文档' },
          modal: {
            noResultsText: '没有找到相关内容',
            resetButtonTitle: '清除查询',
            footer: { selectText: '选择', navigateText: '切换', closeText: '关闭' },
          },
        },
      },
    },
    nav: [
      { text: '指南', link: '/guide/' },
      { text: '概念', link: '/concepts/' },
      { text: '集成', link: '/integrations/' },
      { text: '运维', link: '/operations/' },
      { text: '参考', link: '/reference/' },
      { text: '贡献', link: '/contributing/' },
    ],
    sidebar: {
      '/guide/': [
        {
          text: '开始使用',
          items: [
            { text: '启动 Harness', link: '/guide/' },
            { text: 'CLI 开发工具示例', link: '/guide/cli-example' },
            { text: '开发插件', link: '/guide/plugin' },
            { text: '内置插件概览', link: '/guide/plugins' },
            { text: 'Profile 组合', link: '/guide/profile' },
            { text: '事件监听', link: '/guide/events' },
            { text: '插件生命周期与测试', link: '/guide/plugin-lifecycle' },
          ],
        },
        {
          text: '运行能力',
          items: [
            { text: '模型 Provider', link: '/guide/connect-llm' },
            { text: '工具插件', link: '/guide/custom-tool' },
            { text: 'Session 与恢复', link: '/guide/session' },
          ],
        },
      ],
      '/concepts/': [
        {
          text: '核心概念',
          items: [
            { text: '概览', link: '/concepts/' },
            { text: 'Service 与 Seam', link: '/concepts/services' },
            { text: '事件与 Effect', link: '/concepts/events' },
            { text: 'Agent Loop 与 Workflow', link: '/concepts/agent-workflow' },
            { text: '模型、提示词与工具', link: '/concepts/model-prompt-tool' },
            { text: '状态与生命周期', link: '/concepts/state' },
            { text: 'Memory 与 Retrieval', link: '/concepts/memory-retrieval' },
            { text: '团队与多 Agent', link: '/concepts/multi-agent' },
            { text: '插件与运行时组合', link: '/concepts/runtime-composition' },
          ],
        },
      ],
      '/integrations/': [
        {
          text: '外部集成',
          items: [
            { text: '概览', link: '/integrations/' },
            { text: '模型 Provider', link: '/integrations/providers' },
            { text: 'MCP 与 A2A', link: '/integrations/protocols' },
            { text: '存储与 Checkpoint', link: '/integrations/storage' },
            { text: '可观测性', link: '/integrations/observability' },
          ],
        },
      ],
      '/operations/': [
        {
          text: '生产运维',
          items: [
            { text: '上线与部署', link: '/operations/' },
            { text: '外部集成', link: '/integrations/' },
            { text: '配置与环境变量', link: '/reference/configuration' },
            { text: '可观测性', link: '/integrations/observability' },
          ],
        },
      ],
      '/reference/': [
        {
          text: '参考资料',
          items: [
            { text: '概览', link: '/reference/' },
            { text: '内置插件全量目录', link: '/reference/plugins' },
            { text: 'CLI 命令', link: '/reference/cli' },
            { text: '配置与环境变量', link: '/reference/configuration' },
            { text: '能力与实现状态', link: '/reference/compatibility' },
            { text: 'Plugin API', link: '/reference/plugin-api' },
            { text: 'Catalog 与 Profile API', link: '/reference/plugin-catalog' },
            { text: '术语表', link: '/reference/glossary' },
          ],
        },
      ],
      '/contributing/': [
        {
          text: '参与贡献',
          items: [
            { text: '贡献指南', link: '/contributing/' },
            { text: '能力与实现状态', link: '/reference/compatibility' },
          ],
        },
      ],
    },
    footer: {
      message: '以 Apache License 2.0 发布',
      copyright: 'agent-harness contributors',
    },
  },
})
