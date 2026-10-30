# genie

The agent guide, the coding standards and all product documentation live in the project's knowledge vault (project `genie`, space `genie/`), not in this repository. Before working, read every page your role requires, whole:

```bash
genie --project genie docs required --role <your role>
```

`genie/agents/guide.md` applies to all roles; `genie/agents/coding-standards.md` applies to executor, reviewer, tester and documenter. Host tools load this file by name, so this repository keeps only this pointer. A running tracker is required: without access to `genie --project genie` or to https://github.com/grigoryshulga/genie-vault (private), ask the owner for the pages and the connection instead of working from memory. Files that stay in the repository are `README.md`, `CHANGELOG.md`, the brand sources in `design-system/brand/` and code comments. The documentation site in `site/` builds from the vault: `npm ci --prefix site`, `npm run docs:vault`, `npm run docs:dev`.

For browser-to-Figma capture, offline plugin work or native import validation, use the separate private [genie-figma-tools repository](https://github.com/grigoryshulga/genie-figma-tools) and its [agent workflow](https://github.com/grigoryshulga/genie-figma-tools/blob/main/docs/agent-workflow.md). This repository owns the application's tokens and exporter; the toolkit owns capture/import code and tests.
