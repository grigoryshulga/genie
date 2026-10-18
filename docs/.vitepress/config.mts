import { defineConfig } from 'vitepress'

export default defineConfig({
  lang: 'en-US',
  title: 'genie',
  titleTemplate: ':title · Genie',
  description: 'Run tasks, knowledge, and AI agent teams on one self-hosted server.',
  base: '/genie/',
  appearance: 'force-dark',
  lastUpdated: false,
  sitemap: { hostname: 'https://grigoryshulga.github.io/genie/' },
  head: [
    ['link', { rel: 'icon', type: 'image/svg+xml', href: '/genie/logo.svg' }],
    ['meta', { name: 'theme-color', content: '#0c0d0f' }],
  ],
  themeConfig: {
    logo: { src: '/logo.svg', alt: '' },
    siteTitle: 'genie',
    nav: [
      { text: 'Guide', link: '/platform/getting-started', activeMatch: '/(platform|guide)/' },
      { text: 'Development', link: '/development' },
      { text: 'Changelog', link: '/changelog' },
    ],
    socialLinks: [{ icon: 'github', link: 'https://github.com/grigoryshulga/genie' }],
    search: { provider: 'local', options: { detailedView: true } },
    sidebar: [
      { text: 'Start here', items: [
        { text: 'Introduction', link: '/platform/vision' },
        { text: 'Quick start', link: '/platform/getting-started' },
        { text: 'Docker deployment', link: '/platform/docker' },
      ] },
      { text: 'Work with Genie', items: [
        { text: 'Tasks and epics', link: '/guide/tasks' },
        { text: 'Roles and teams', link: '/platform/agent-roles-and-teams' },
          { text: 'Project and task managers', link: '/platform/project-and-task-managers' },
        { text: 'Repositories and delivery', link: '/platform/git-repositories' },
        { text: 'Knowledge vault', link: '/platform/knowledge-vault' },
        { text: 'Automations', link: '/platform/automations' },
        { text: 'Observation', link: '/platform/observation' },
        { text: 'CLI and MCP', link: '/reference/cli' },
      ] },
      { text: 'Run your server', items: [
        { text: 'Settings and access', link: '/platform/settings' },
        { text: 'Agent runtime', link: '/platform/agent-bus' },
        { text: 'Project environments', link: '/platform/environment' },
        { text: 'Operations', link: '/platform/operations' },
        { text: 'Run a pilot', link: '/platform/pilot' },
      ] },
      { text: 'Build and understand', items: [
        { text: 'Architecture', link: '/platform/backend' },
        { text: 'Documentation contract', link: '/reference/genie-docs-system' },
        { text: 'Development', link: '/development' },
        { text: 'Repository workflow', link: '/development/genie-cli' },
        { text: 'Design decisions', link: '/platform/decisions' },
        { text: 'Roadmap', link: '/platform/roadmap' },
        { text: 'Product proposals', link: '/platform/proposals' },
        { text: 'Changelog', link: '/changelog' },
      ] },
    ],
    outline: { level: [2, 3], label: 'On this page' },
    editLink: {
      pattern: ({ filePath }) => `https://github.com/grigoryshulga/genie/edit/main/${filePath === 'changelog.md' ? 'CHANGELOG.md' : `docs/${filePath}`}`,
      text: 'Edit this page on GitHub',
    },
    docFooter: { prev: 'Previous', next: 'Next' },
    footer: { message: 'Self-hosted. Open source. Yours.', copyright: 'Genie · MIT License' },
  },
})
