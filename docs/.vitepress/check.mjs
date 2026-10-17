import { readFile, readdir, access } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { createMarkdownRenderer } from 'vitepress'

const docs = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const root = path.dirname(docs)
const markdown = await createMarkdownRenderer(docs)
const errors = []

async function pages(directory) {
  const entries = await readdir(directory, { withFileTypes: true })
  const result = []
  for (const entry of entries) {
    if (entry.name.startsWith('.') || entry.name === 'node_modules') continue
    const file = path.join(directory, entry.name)
    if (entry.isDirectory()) result.push(...await pages(file))
    else if (entry.name.endsWith('.md')) result.push(file)
  }
  return result
}

const files = [...await pages(docs), ...await pages(path.join(root, 'bench')), path.join(root, 'README.md'), path.join(root, 'CHANGELOG.md')]
const sources = new Map()
for (const file of files) {
  const source = await readFile(file, 'utf8')
  if (/\p{Script=Cyrillic}/u.test(source)) errors.push(`${path.relative(root, file)}: documentation must be in English`)
  sources.set(file, source.replace(/^---\r?\n[\s\S]*?\r?\n---\r?\n/, ''))
}

const headings = new Map()
async function ids(file) {
  if (!headings.has(file)) {
    const body = sources.get(file) ?? await readFile(file, 'utf8')
    const html = markdown.render(body, { path: file, relativePath: path.relative(docs, file) })
    headings.set(file, new Set([...html.matchAll(/\bid="([^"]+)"/g)].map(match => match[1])))
  }
  return headings.get(file)
}

for (const [file, body] of sources) {
  const tokens = markdown.parse(body, {})
  const links = tokens.flatMap(token => token.children ?? []).flatMap(token => {
    const href = token.type === 'link_open' ? token.attrGet('href') : token.type === 'image' ? token.attrGet('src') : null
    return href ? [href] : []
  })
  for (const href of links) {
    if (/^(?:[a-z][a-z\d+.-]*:|\/\/)/i.test(href)) continue
    const [target, fragment] = href.split('#')
    const local = target.split('?')[0]
    // Source links use Markdown extensions; site navigation is validated by the build.
    const destination = local ? path.resolve(path.dirname(file), decodeURIComponent(local)) : file
    try { await access(destination) } catch {
      errors.push(`${path.relative(root, file)}: missing link target ${href}`)
      continue
    }
    if (fragment && destination.endsWith('.md') && !(await ids(destination)).has(decodeURIComponent(fragment))) {
      errors.push(`${path.relative(root, file)}: missing heading in ${href}`)
    }
  }
}

if (errors.length) {
  for (const error of errors) console.error(error)
  process.exitCode = 1
} else {
  console.log(`Checked ${files.length} English Markdown files and their local links and headings.`)
}
