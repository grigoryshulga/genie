#!/usr/bin/env node
// Assembles the documentation site's content directory from its sources:
//
// - the `genie` space of the knowledge vault (GENIE_VAULT_DIR, default `site/.vault`);
// - the brand sources in `design-system/brand/`, copied to `assets/brand/` where pages link them;
// - the static shell files in `site/public/`, which VitePress serves from `<srcDir>/public`;
// - this repository's `CHANGELOG.md`, which the generated `changelog.md` page includes, because the
//   vault keeps its own project changelog at `genie/changelog.md`.
//
// `site/content/` is generated and ignored by git. Run `npm run docs:vault` first to clone the vault.

import { cp, mkdir, readdir, rm, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const site = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const root = path.dirname(site)
const vault = path.resolve(process.env.GENIE_VAULT_DIR ?? path.join(site, '.vault'))
const space = path.join(vault, 'genie')
const content = path.join(site, 'content')

// The site's Changelog page keeps rendering this repository's release notes, not the vault's.
const changelog = `---
title: Changelog
summary: Release notes and unreleased changes.
---

<!--@include: ../../CHANGELOG.md-->
`

async function copyTree(from, to) {
  const entries = await readdir(from, { withFileTypes: true })
  await mkdir(to, { recursive: true })
  for (const entry of entries) {
    if (entry.name.startsWith('.') || entry.name === 'node_modules') continue
    const source = path.join(from, entry.name)
    const target = path.join(to, entry.name)
    if (entry.isDirectory()) await copyTree(source, target)
    else if (entry.isFile()) await cp(source, target)
  }
}

let entries
try {
  entries = await readdir(space)
} catch {
  console.error(
    `The knowledge vault space is missing: ${path.join(space, '')}\n` +
      `Clone or update the vault with \`npm run docs:vault\`, or set GENIE_VAULT_DIR to a checkout ` +
      `containing the \`genie\` space.`,
  )
  process.exit(1)
}
if (!entries.length) {
  console.error(`The knowledge vault space is empty: ${path.join(space, '')}`)
  process.exit(1)
}

await rm(content, { recursive: true, force: true })
await copyTree(space, content)
await copyTree(path.join(root, 'design-system', 'brand'), path.join(content, 'assets', 'brand'))
await copyTree(path.join(site, 'public'), path.join(content, 'public'))
await writeFile(path.join(content, 'changelog.md'), changelog)

console.log(`Assembled ${path.relative(root, content)} from ${path.relative(root, space)}.`)
