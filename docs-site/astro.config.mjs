import { defineConfig } from 'astro/config';
import mermaid from 'astro-mermaid';
import starlight from '@astrojs/starlight';

export default defineConfig({
  integrations: [
    mermaid({
      theme: 'base',
      autoTheme: true,
      enableLog: false,
      mermaidConfig: {
        themeVariables: {
          primaryColor: '#f8fafc',
          primaryTextColor: '#0f172a',
          primaryBorderColor: '#cbd5e1',
          lineColor: '#64748b',
          secondaryColor: '#f0fdf4',
          tertiaryColor: '#eff6ff',
          fontFamily:
            'Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif',
        },
        flowchart: {
          curve: 'basis',
          padding: 18,
        },
      },
    }),
    starlight({
      title: 'sivtr',
      description:
        'Documentation for sivtr, a unified local-first agent memory workspace for humans and agents.',
      social: [
        {
          icon: 'github',
          label: 'GitHub',
          href: 'https://github.com/Ariestar/sivtr',
        },
      ],
      locales: {
        root: { label: 'English', lang: 'en' },
        'zh-cn': { label: '简体中文', lang: 'zh-CN' },
      },
      defaultLocale: 'root',
      favicon: '/favicon.svg',
      customCss: ['./src/styles/custom.css'],
      tableOfContents: {
        minHeadingLevel: 2,
        maxHeadingLevel: 3,
      },
      lastUpdated: true,
      sidebar: [
        {
          label: 'Overview',
          translations: { 'zh-CN': '概览' },
          link: '/',
        },
        {
          label: 'Start',
          translations: { 'zh-CN': '开始' },
          items: [
            'start/installation',
            'start/quickstart',
            'start/core-concepts',
          ],
        },
        {
          label: 'Guides',
          translations: { 'zh-CN': '指南' },
          items: [
            'usage/capture-output',
            'usage/browse-and-select',
            'usage/copy-command-blocks',
            'usage/compare-command-blocks',
            'usage/ai-sessions',
            'usage/search-and-show',
            'usage/web-ui',
            'usage/skills',
            'usage/remote-access',
            'usage/history',
            'usage/configuration',
            'usage/launchers-and-hotkeys',
          ],
        },
        {
          label: 'Playbooks',
          translations: { 'zh-CN': '玩法实例' },
          items: [
            'playbooks',
            'playbooks/fix-terminal-error',
            'playbooks/recent-work-timeline',
            'playbooks/continue-after-interruption',
            'playbooks/agent-handoff',
            'playbooks/remote-collaboration-memory',
          ],
        },
        {
          label: 'Reference',
          translations: { 'zh-CN': '参考' },
          items: [
            'reference/cli',
            'reference/selectors-and-filters',
            'reference/keybindings',
            'reference/config-file',
            'reference/data-locations',
            'reference/troubleshooting',
          ],
        },
        {
          label: 'Explanation',
          translations: { 'zh-CN': '解释' },
          items: [
            'explanation/architecture',
            'explanation/session-model',
            'explanation/local-first-privacy',
          ],
        },
        {
          label: 'Project',
          translations: { 'zh-CN': '项目' },
          items: ['project/roadmap', 'project/release-notes', 'project/sponsor'],
        },
      ],
    }),
  ],
});
