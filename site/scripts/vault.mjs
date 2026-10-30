#!/usr/bin/env node
// Clones or updates the knowledge vault checkout the site is built from.
//
// The vault is private: the remote is reached over SSH with a key that has read access. CI checks
// the vault out with a deploy key instead (see .github/workflows/docs.yml), and a local run may point
// GENIE_VAULT_DIR at any existing checkout.

import { execFileSync } from 'node:child_process'
import { existsSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const site = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const root = path.dirname(site)
const remote = process.env.GENIE_VAULT_REMOTE ?? 'git@github.com:grigoryshulga/genie-vault.git'

if (process.env.GENIE_VAULT_DIR) {
  console.log(`GENIE_VAULT_DIR is set; using ${process.env.GENIE_VAULT_DIR} instead of a local clone.`)
  process.exit(0)
}

const target = path.join(site, '.vault')
const git = (args) => execFileSync('git', args, { stdio: 'inherit' })

if (existsSync(path.join(target, '.git'))) {
  console.log(`Updating the knowledge vault in ${path.relative(root, target)}.`)
  git(['-C', target, 'pull', '--ff-only'])
} else {
  console.log(`Cloning ${remote} into ${path.relative(root, target)}.`)
  git(['clone', remote, target])
}
