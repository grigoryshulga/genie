// Fixtures for screens whose data the disposable seed cannot produce: they need a GitHub App and a
// provider to talk to. They are synthetic and read-only, merged into the recorded fixtures on export
// so that regenerating them keeps these screens showable.

const at = "2026-05-01T12:00:00Z";

const github = { id: "github", kind: "github", url: "https://github.com" };
const gitlab = { id: "gitlab", kind: "gitlab", url: "https://gitlab.com" };

const repo = (name: string, remote: string, over: Record<string, unknown>) => ({
  project: "demo",
  name,
  remote,
  mount: name === "api" ? "." : name,
  defaultBranch: "main",
  access: "write",
  policy: {},
  policyValid: true,
  created: at,
  ...over,
});

const connection = (id: string, over: Record<string, unknown>) => ({
  id,
  provider: "github",
  host: "github",
  owner: 1,
  ownerLogin: "owner",
  accountId: 7,
  accountType: "Organization",
  permissions: {},
  statusNote: "",
  checked: at,
  created: at,
  updated: at,
  blocked: null,
  manageUrl: "https://github.com/settings/installations/42",
  ...over,
});

const providerRepo = (id: number, fullName: string, over: Record<string, unknown> = {}) => ({
  id,
  fullName,
  defaultBranch: "main",
  private: true,
  canPush: true,
  boundAs: null,
  boundElsewhere: false,
  ...over,
});

export const synthetic: Record<string, unknown> = {
  "/api/git/hosts": { errors: [], hosts: [{ ...github, transport: "https", problems: [] }, { ...gitlab, transport: "https", problems: [] }] },
  "/api/repos": [
    repo("api", "acme/api", {
      host: { ...github, webUrl: "https://github.com/acme/api" },
      connection: "gc_demo",
      providerRepoId: 501,
      token: { set: false },
    }),
    repo("web", "acme/web", {
      host: { ...gitlab, webUrl: "https://gitlab.com/acme/web" },
      token: { set: true, hint: "…a1b2", updated: at },
    }),
  ],
  "/api/git/connections": {
    github: { configured: true, slug: "genie-demo", host: "github" },
    canConnect: true,
    connections: [
      connection("gc_demo", { accountLogin: "acme", installationId: 42, selection: "selected", health: "ok", repos: 1 }),
      connection("gc_orphan", {
        accountLogin: "labs-guild",
        accountType: "User",
        installationId: 43,
        selection: "all",
        owner: null,
        ownerLogin: null,
        health: "owner_removed",
        repos: 0,
        blocked: "the connection to labs-guild (github) cannot be used: its owner was removed or disabled; an administrator must reassign the connection",
        manageUrl: "https://github.com/settings/installations/43",
      }),
    ],
  },
  "/api/git/connections/gc_demo/repositories": {
    connection: "gc_demo",
    repositories: [
      providerRepo(501, "acme/api", { boundAs: "api" }),
      providerRepo(502, "acme/web"),
      providerRepo(503, "acme/docs", { private: false }),
      providerRepo(504, "acme/infrastructure-and-deployment-manifests-for-every-environment", { defaultBranch: "trunk" }),
      providerRepo(505, "acme/mobile", { canPush: false }),
      providerRepo(506, "acme/billing", { boundElsewhere: true }),
    ],
  },
  // The project chat: an exchange with the project manager, ending in a question it is still answering,
  // so the typing line shows. Authors alternate, so no two messages share a header.
  "/api/chat": {
    messages: [
      chatMessage(1, "owner", "human", "Что сейчас в работе по проекту?", { mentions: ["manager"] }),
      chatMessage(2, "orchestrator", "orchestrator", "Сейчас в работе **три задачи**:\n\n- GNE-12 — ждёт ревью\n- GNE-14 — агент пишет код\n- GNE-19 — менеджер готовит описание", { replyTo: 1 }),
      chatMessage(3, "owner", "human", "Создай задачу: проверить поиск по документации на мобильном", { mentions: ["manager"] }),
      chatMessage(4, "orchestrator", "orchestrator", "Создал **GNE-23** в подготовке. Когда описание будет готово, переведу её в очередь.", { replyTo: 3 }),
      chatMessage(5, "owner", "human", "Поставь её после GNE-14: поиск зависит от него. Когда закроется GNE-14?", { mentions: ["manager"] }),
    ],
    manager: { state: "working", pending: 0 },
  },
};

function chatMessage(id: number, author: string, authorRole: string, text: string, over: { mentions?: string[]; replyTo?: number } = {}) {
  return { id, at, author, authorRole, text, mentions: over.mentions ?? [], replyTo: over.replyTo ?? null };
}
