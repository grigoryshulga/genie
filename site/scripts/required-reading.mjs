import { readFile, realpath } from 'node:fs/promises'
import path from 'node:path'

// Keep the limit and body extraction aligned with REQUIRED_PAGE_LIMIT and parse_doc in Rust.
const requiredPageLimit = 12_000

function bodyOf(markdown) {
  const text = markdown.replace(/^\uFEFF/, '')
  const opening = text.startsWith('---\n') ? 4 : text.startsWith('---\r\n') ? 5 : 0
  if (!opening) return text
  const rest = text.slice(opening)
  let end = rest.indexOf('\n---\n')
  if (end < 0) end = rest.indexOf('\n---\r\n')
  if (end < 0 && rest.endsWith('\n---')) end = rest.length - 4
  if (end < 0) return text
  return rest.slice(end).replace(/^\n+/, '').replace(/^(?:---)+/, '').replace(/^[\r\n]+/, '')
}

function inside(root, file) {
  const relative = path.relative(root, file)
  return relative !== '' && relative !== '..' && !relative.startsWith(`..${path.sep}`) && !path.isAbsolute(relative)
}

/** Validate required reading from the vault itself, before assembly drops its hidden configuration. */
export async function checkRequiredReading(vault) {
  const errors = []
  const configPath = path.join(vault, '.genie', 'vault.json')
  let required
  try {
    const config = JSON.parse(await readFile(configPath, 'utf8'))
    required = config.spaces?.genie?.required ?? []
    if (!Array.isArray(required)) throw new Error('spaces.genie.required must be a list')
  } catch (error) {
    return [`required reading: cannot read .genie/vault.json: ${error.message}`]
  }

  const space = path.join(await realpath(vault), 'genie')
  const seen = new Set()
  for (const entry of required) {
    const page = typeof entry?.page === 'string' ? entry.page.trim() : ''
    if (seen.has(page)) continue
    seen.add(page)
    const file = path.resolve(vault, page)
    if (!page || page.includes('\\') || page.includes('\0') || page.split('/').some(part => part === '..')
      || path.isAbsolute(page) || !inside(path.resolve(vault, 'genie'), file)) {
      errors.push(`required reading: ${JSON.stringify(page)} must be a page inside genie/`)
      continue
    }
    try {
      // A lexically valid path may still escape through a symlink, including the space root itself.
      const resolved = await realpath(file)
      if (!inside(space, resolved)) {
        errors.push(`required reading: ${page} resolves outside genie/`)
        continue
      }
      const body = bodyOf(await readFile(resolved, 'utf8'))
      const count = [...body].length
      if (count > requiredPageLimit) {
        errors.push(`required reading: ${page} has ${count} characters (limit ${requiredPageLimit}); shorten it or move optional material to another page`)
      }
    } catch (error) {
      errors.push(`required reading: ${page} is missing or unreadable (${error.code ?? error.message})`)
    }
  }
  return errors
}
