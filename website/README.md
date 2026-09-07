# agent-harness documentation website

本目录是 `agent-harness` 专属的 VitePress 使用文档站点，不包含其他子项目的用户指南。

```sh
cd agent-harness/website
npm install
npm run dev
npm run build
npm run preview
npm test
```

内容入口位于 `docs/`；站点配置位于 `docs/.vitepress/config.mjs`；视觉主题位于 `docs/.vitepress/theme/custom.css`。

`reference/plugins.md` 是生成页面，不手工编辑。`npm run generate:plugins` 从当前 Cargo Workspace 读取全部 `ah-plugins-*` crate；`dev`、`build` 和 `test` 会自动刷新它。
