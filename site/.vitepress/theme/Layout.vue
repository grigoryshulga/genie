<script setup lang="ts">
import DefaultTheme from 'vitepress/theme-without-fonts'
import { useData } from 'vitepress'
const { frontmatter } = useData()
</script>

<template>
  <DefaultTheme.Layout>
    <template #nav-bar-title-after><span class="docs-badge">docs</span></template>
    <template #sidebar-nav-before>
      <a class="sidebar-overview" href="https://github.com/grigoryshulga/genie" target="_blank" rel="noreferrer">
        <span class="overview-dot" /> One server. Your workspace.
        <span aria-hidden="true">↗</span>
      </a>
    </template>
    <template #doc-before>
      <div v-if="frontmatter.type" class="page-kind">{{ frontmatter.type }}</div>
      <div v-if="frontmatter.status === 'draft'" class="editorial-note">Proposal · Describes planned work.</div>
      <div v-if="frontmatter.status === 'deprecated'" class="editorial-note">Historical page · Follow the links below for current documentation.</div>
    </template>
  </DefaultTheme.Layout>
</template>
