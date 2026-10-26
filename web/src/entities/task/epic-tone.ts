/** Stable visual identity, independent of task order, status and the roles in its team. */
export function epicTone(id: string): number {
  let hash = 0;
  for (const char of id) hash = (Math.imul(hash, 31) + char.charCodeAt(0)) | 0;
  return (hash >>> 0) % 4 + 1;
}
