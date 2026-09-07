---
layout: home
hero:
  name: agent-harness
  text: 让 Agent Runtime 可组合、可观察、可回滚
  tagline: openJiuwen 的 Rust 版本实现。用 Seam、Plugin、EventBus 和 Profile，把模型、工具、会话与团队能力组装成可验证的 Agent Runtime。
  image:
    src: /mark.svg
    alt: agent-harness mark
  actions:
    - theme: brand
      text: 开始使用
      link: /guide/
    - theme: alt
      text: 阅读架构
      link: /concepts/
features:
  - icon: ◇
    title: Seam first
    details: 先定义稳定服务契约，再分别实现 Provider 和 Consumer，避免插件之间的具体类型耦合。
  - icon: ↯
    title: 可逆装配
    details: 注册、事件监听和后台资源绑定 Effect；卸载插件即可回滚，关闭流程可验证。
  - icon: ⌁
    title: Profile 驱动
    details: 用 Profile 表达开发、生产和混合组合，按 provides/inject 拓扑安全挂载插件。
---

<div class="home-signal">
  <span class="eyebrow">AGENT-HARNESS / RUST</span>
  <span>从契约到生产组合</span>
</div>

## 先理解一条运行链

<div class="path-grid">
  <a class="path-card" href="/guide/harness">
    <span class="path-index">01 / BOOT</span>
    <strong>启动一个 Profile</strong>
    <span>从开发 Profile 开始，观察 Catalog、依赖拓扑、服务挂载和显式失败。</span>
  </a>
  <a class="path-card" href="/concepts/runtime-composition">
    <span class="path-index">02 / PLUGIN</span>
    <strong>写一个插件</strong>
    <span>实现 Seam Provider，返回 Effect，并通过 Context 让 Consumer 解析服务。</span>
  </a>
  <a class="path-card" href="/integrations/">
    <span class="path-index">03 / OPERATE</span>
    <strong>接入真实依赖</strong>
    <span>连接模型、MCP、A2A、存储与观测系统，验证失败、取消和清理路径。</span>
  </a>
</div>

## 学习路径

1. **[启动 Harness](/guide/)**：运行 dev Profile，确认本地组合可用。
2. **[理解服务与事件](/concepts/)**：掌握 Registry、EventBus 和 Effect 生命周期。
3. **[编写插件](/guide/plugin)**：实现 Provider、Consumer 和稳定 ServiceKey。
4. **[定义 Profile](/guide/profile)**：声明插件组合和依赖关系。
5. **[验证生产路径](/operations/)**：接入真实服务，不以 Mock 结果替代生产证据。

::: warning 生产边界
开发 Profile 中的 Mock Provider 只用于本地冒烟。生产 Profile 必须显式失败，不得在真实依赖不可用时静默回退到 Mock 或进程内存。
:::

<div class="home-footnote">
  <span>RUST ONLY</span>
  <span>APACHE LICENSE 2.0</span>
  <span>PLUGIN RUNTIME / SEAM / PROFILE</span>
</div>
