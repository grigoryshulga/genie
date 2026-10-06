/** Tokens of model responses. `input` is read fresh, cache reads and writes are apart. */
export interface Tokens {
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
}

/** What `modelPrices` in config.json says a model costs, dollars per million tokens. */
export interface ModelPrice {
  input: number;
  output: number;
  cacheRead: number | null;
  cacheWrite: number | null;
}

/** One model's share of some work. */
export interface ModelSpend {
  /** `provider/model`. */
  model: string;
  calls: number;
  tokens: Tokens;
  /** Dollars; `null` when the model has no price. */
  cost: number | null;
  /** Tokens no price covered (cache without a cache price; all of them when the model has no price). */
  unpricedTokens: number;
  price: ModelPrice | null;
}

/** Tokens and money of some work: a chat, a task, an epic, a project. */
export interface Spend {
  calls: number;
  tokens: Tokens;
  /** Dollars, of the models with a price. */
  cost: number;
  /** Tokens of models without a price, not in `cost`. */
  unpricedTokens: number;
  /** Most expensive first. */
  models: ModelSpend[];
}

/**
 * An epic, a task or a chat with its spend. Epics also come as `""` (tasks outside
 * epics) and `"-"` (work on no task: the orchestrator, jobs without a task).
 */
export interface SpendItem {
  id: string;
  title: string;
  /** A task's epic, a chat's task. */
  parent?: string;
  status?: string;
  spend: Spend;
}

export const NO_TOKENS: Tokens = { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 };
export const NO_SPEND: Spend = { calls: 0, tokens: NO_TOKENS, cost: 0, unpricedTokens: 0, models: [] };

export const tokensTotal = (t: Tokens) => t.input + t.output + t.cacheRead + t.cacheWrite;

const addTokens = (a: Tokens, b: Tokens): Tokens => ({
  input: a.input + b.input,
  output: a.output + b.output,
  cacheRead: a.cacheRead + b.cacheRead,
  cacheWrite: a.cacheWrite + b.cacheWrite,
});

/** Several spends as one: models of the same name merged, most expensive first. */
export function mergeSpends(list: Spend[]): Spend {
  const models = new Map<string, ModelSpend>();
  let out: Spend = { ...NO_SPEND, models: [] };
  for (const s of list) {
    out = { ...out, calls: out.calls + s.calls, tokens: addTokens(out.tokens, s.tokens), cost: out.cost + s.cost, unpricedTokens: out.unpricedTokens + s.unpricedTokens };
    for (const m of s.models) {
      const was = models.get(m.model);
      models.set(
        m.model,
        was
          ? { ...was, calls: was.calls + m.calls, tokens: addTokens(was.tokens, m.tokens), cost: was.cost === null || m.cost === null ? (was.cost ?? m.cost) : was.cost + m.cost, unpricedTokens: (was.unpricedTokens ?? 0) + (m.unpricedTokens ?? 0) }
          : { ...m },
      );
    }
  }
  out.models = [...models.values()].sort((a, b) => (b.cost ?? -1) - (a.cost ?? -1) || tokensTotal(b.tokens) - tokensTotal(a.tokens));
  return out;
}

/** Items of several projects in one list, most expensive first; ids are kept per project. */
export function topItems(lists: { project: string; items: SpendItem[] }[], limit: number): (SpendItem & { project: string })[] {
  return lists
    .flatMap((l) => l.items.map((i) => ({ ...i, project: l.project })))
    .sort((a, b) => b.spend.cost - a.spend.cost || tokensTotal(b.spend.tokens) - tokensTotal(a.spend.tokens))
    .slice(0, limit);
}

/** Dollars: «$12,40», «$0,042» for small sums, «$0» for nothing. */
export function moneyText(x: number): string {
  if (!x) return "$0";
  const digits = x >= 1 ? 2 : x >= 0.01 ? 3 : 4;
  return `$${x.toFixed(digits).replace(".", ",")}`;
}

/** Tokens in words: «850», «12 тыс.», «1,4 млн». */
export function tokensText(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1).replace(".", ",").replace(",0", "")} млн`;
  if (n >= 1_000) return `${Math.round(n / 1_000)} тыс.`;
  return String(n);
}

/** The model without its provider: «claude-opus-5-5». */
export const modelName = (m: string) => m.replace(/^[^/]+\//, "");

/** A chat in words: the orchestrator, a job, or a member of a team. */
export function chatTitle(agent: string): { name: string; sub: string } {
  if (agent === "orchestrator") return { name: "Оркестратор", sub: "чат проекта" };
  const job = /^job\/(\d+)$/.exec(agent);
  if (job) return { name: `Задание #${job[1]}`, sub: "разовое задание" };
  const [team, member] = agent.split("/");
  return member ? { name: member, sub: `команда ${team}` } : { name: agent, sub: "" };
}

/** A spend in one line: «$4,20 · 1,3 млн токенов», or tokens only when no model has a price. */
export function spendText(s: Spend): string {
  const tokens = `${tokensText(tokensTotal(s.tokens))} токенов`;
  if (!s.models.some((m) => m.cost !== null)) return `${tokens} · цена модели не указана`;
  return `${moneyText(s.cost)} · ${tokens}`;
}
