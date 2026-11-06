<script setup lang="ts">
import { withBase } from 'vitepress'

const routes = [
  { n: '01', title: 'Get up and running', text: 'Install Genie, connect a model, create your first task.', link: '/platform/getting-started', tag: 'Start here' },
  { n: '02', title: 'Build your agent team', text: 'Choose roles, skills, tools, and who hands off to whom.', link: '/platform/agent-roles-and-teams', tag: 'Configure' },
  { n: '03', title: 'Bring your repositories', text: 'Connect GitHub or GitLab. Define review and merge policies.', link: '/platform/git-repositories', tag: 'Connect' },
  { n: '04', title: 'Keep things running', text: 'Back up, update, recover, and check server health.', link: '/platform/operations', tag: 'Operate' },
]
</script>

<template>
  <div class="docs-home">
    <div class="home-eyebrow"><span class="eyebrow-line" /> THE GENIE HANDBOOK</div>
    <section class="home-intro" aria-labelledby="home-title">
      <h1 id="home-title">People, knowledge,<br>and <span>agent teams.</span></h1>
      <p class="home-lead">Your team's work, on one self-hosted server.<br>From the first task to the final review.</p>
      <div class="home-actions">
        <a class="home-button primary" :href="withBase('/platform/getting-started.html')">Quick start <span aria-hidden="true">→</span></a>
        <a class="home-button secondary" :href="withBase('/platform/vision.html')">Meet Genie <span aria-hidden="true">↗</span></a>
      </div>
    </section>

    <section class="workflow-panel" aria-labelledby="workflow-title">
      <div class="panel-heading"><span id="workflow-title">A task, from idea to done</span><span class="panel-label">THE WORKFLOW</span></div>
      <div class="workflow">
        <div class="workflow-step"><span class="step-symbol inbox-symbol">＋</span><strong>Describe the work</strong><span>People set the goal.</span></div>
        <span class="flow-arrow" aria-hidden="true">→</span>
        <div class="workflow-step"><span class="step-symbol team-symbol">✧</span><strong>Assemble a team</strong><span>Agents work in focus.</span></div>
        <span class="flow-arrow" aria-hidden="true">→</span>
        <div class="workflow-step"><span class="step-symbol review-symbol">✓</span><strong>Review the result</strong><span>Evidence before done.</span></div>
      </div>
      <div class="panel-foot"><span class="status-dot" /> You choose how much autonomy to give.</div>
    </section>

    <section class="reading-paths" aria-labelledby="paths-title">
      <div class="section-heading"><h2 id="paths-title">Find your next step</h2><span>FOUR WAYS IN</span></div>
      <a v-for="route in routes" :key="route.n" :href="withBase(`${route.link}.html`)" class="reading-path">
        <span class="path-number">{{ route.n }}</span>
        <span class="path-copy"><strong>{{ route.title }}</strong><span>{{ route.text }}</span></span>
        <span class="path-tag">{{ route.tag }}</span>
        <span class="path-arrow" aria-hidden="true">↗</span>
      </a>
    </section>

    <div class="home-bottom">
      <span>Prefer the terminal?</span>
      <a :href="withBase('/reference/cli.html')"><code>genie task create</code> <span aria-hidden="true">→</span> CLI &amp; MCP reference</a>
    </div>
  </div>
</template>
