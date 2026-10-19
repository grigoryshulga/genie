---
title: Project memory — external evidence
summary: Primary-source evidence (vendor docs, engineering posts, papers, SQLite docs, Mnemopi source) on long-term memory for coding agents, mapped to the choices in docs/platform/memory.md.
type: reference
status: draft
---

# Project memory — external evidence

Collected 2026-10-19 for the design note [Project memory](memory.md). The draft is treated as a set of hypotheses. Each finding below says whether external evidence **confirms**, **refutes**, or **adds to** a choice. Vendor numbers are self-reported unless stated otherwise. Russian examples are transliterated (the docs site is English-only); the tests used Cyrillic strings. The headline numbers in this note were re-checked against their sources by a second, independent pass on the same day. "pp" means percentage points.

## Key findings

1. **Refutes "memory as an index-free pinned block plus search" as the only default; adds the index pattern.** Every production system that loads something by default loads either *short curated instructions* or a *one-line-per-entry index*, and loads bodies on demand. Examples: Claude Code `MEMORY.md` (first 200 lines/25 KB) with topic files read on demand; Agent Skills (name and description, body on demand); Tool Search (85% fewer tool tokens); Devin, Kiro and Windsurf triggers. The draft has no index. A list of term names and subjects in the block may serve agents better than the full text of a few pinned facts. Measure it.
2. **Confirms the narrow default and the boundary with documentation.** The ETH study (2602.11988) found that context files do not generally improve task success and add about 20% to inference cost. LLM-generated files cost −0.5 to −2 pp of success. Developer-written files gave +2.4 pp, which was not significant. Repository overviews did not help. The files helped only by stating non-standard tools and practices. Claude Code auto memory likewise skips anything derivable from the code. Pinned facts should be non-derivable constraints, not overviews.
3. **Refutes calendar-only staleness (`review_at` of three months, pinned never ages).** GitHub Copilot stores each memory with code citations and verifies them against the current branch at read time. Unused memories are deleted after 28 days. Its A/B test showed a PR merge rate of 83% → 90%. The draft's `source` points to a task or comment, so nothing in it can be checked mechanically. Adding path citations and checking them at read time is cheap and has evidence behind it. A calendar timer has none.
4. **Confirms gating writes, and argues for gating them harder than the draft does.** Selective addition beat add-everything by 15–26 pp across four agents (2505.16067). Agents copy retrieved experience, so bad entries propagate. Vendors restrict writes: Copilot accepts memories only from users with write access, Devin saves knowledge only after user approval, and Cursor removed auto-generated Memories in 2.1 (Nov 2025) in favour of Rules. The draft lets executor agents add entries freely, with the author visible. Evidence favours a quarantine or approval step, or retention decided by later use.
5. **Adds a security requirement.** Memory poisoning is practical. MINJA injected memories through queries alone, with 98.2% injection success and 76.8% attack success. AgentPoison reached >80% attack success at a <0.1% poison rate. Prompt-based detection was weak. OWASP lists this as ASI06 (Dec 2025). The pin permission covers only the prompt path. Unpinned search results also reach other agents' context and need provenance shown and a "data, not instruction" framing.
6. **Weakens "no embeddings" as a free choice; it stays defensible for v1 only with instrumentation.** On LongMemEval, BM25 Recall@5 was 0.634 against 0.723 for dense retrieval at session level, and 0.472 against 0.660 at round level. Cursor measured +12.5% answer accuracy from adding semantic search to grep. Against that, Claude Code dropped its local vector index for agentic search, Sourcegraph removed embeddings in 5.3, and Letta's grep-and-files agent scored 74.0% on LoCoMo against the 68.5% Mem0 reported for its graph variant (different models, indicative only). Short curated entries plus iterative agent search are the favourable case for BM25. Paraphrase misses are expected and must be logged.
7. **Refutes the claim that FTS5 cannot handle Russian inflection without aliases.** FTS5 prefix queries (`vygruzk*`) match both the singular *vygruzka* and the plural *vygruzki* ("export"). I tested this on SQLite 3.54. The trigram tokenizer (SQLite 3.34.0 and later) does substring matching. A Rust-side Snowball Russian stemmer (`rust-stemmers`) can feed a stemmed shadow column. `unicode61` does fold Cyrillic case but does **not** fold yo (U+0451) to e, because `remove_diacritics` applies to Latin script only, so the draft's `normalize()` is required.
8. **Adds that full loading can beat retrieval at small scale.** On LoCoMo, full context scored 72.9 J against 66.9 for Mem0, at about 26k context tokens per query against about 1.8k memory tokens per query for Mem0 (its 7k figure is stored memory per conversation). Retrieval wins on cost and latency, not accuracy, until the corpus is large. A project with a few hundred 400-character entries is in the range where a terse index, or even full loading, should be measured against search.
9. **Adds a risk to the weekly merge.** ACE (2510.04618) names "context collapse": iterative rewriting erodes detail. It recommends incremental delta updates over wholesale rewrites. The draft's dry-run merge is the right instinct. Merges should be rare and should keep the originals reachable, which supersede already does.
10. **Confirms supersede-not-delete.** Zep/Graphiti invalidates contradicted edges by setting `t_invalid` rather than deleting. It keeps separate world-validity and ingestion timelines. Copilot stores a corrected memory instead of editing in place.
11. **Confirms the Mnemopi analysis in substance, with corrections to its numbers.** Details are in §8.

## 1. Production memory in coding agents

### GitHub Copilot Memory (agentic memory)

- **Write path.** Agents call a `store_memory` tool when something is "likely to have actionable implications for future tasks". Each record has `subject`, `fact`, `citations` (file:line) and `reason`. [GitHub blog, Jan 2026](https://github.blog/ai-and-ml/github-copilot/building-an-agentic-memory-system-for-github-copilot/)
- **Read path.** At session start the system retrieves "the most recent memories for the target repository" into the prompt, so retrieval is by recency, not on demand. A search tool and "weighted prioritization" are listed as future work. [same](https://github.blog/ai-and-ml/github-copilot/building-an-agentic-memory-system-for-github-copilot/)
- **Staleness.** "Just-in-time verification": the agent checks the cited code before using a memory. If the code contradicts the memory, the agent stores a corrected version. If the memory checks out, the agent re-stores it to refresh its timestamp. The team rejected an offline curation service as too costly. [same](https://github.blog/ai-and-ml/github-copilot/building-an-agentic-memory-system-for-github-copilot/)
- **Expiry.** Unused facts and preferences are deleted after 28 days. The timer resets when Copilot successfully validates and uses an entry. [GitHub docs, accessed 2026-10-19](https://docs.github.com/en/copilot/concepts/agents/copilot-memory)
- **Who writes and reads.** Repository facts come only from activity by users with write access, and only users with read access can use them. Repository owners can review and delete facts. Personal preferences are separate and scoped to the user. Code review uses repository facts only. [GitHub docs](https://docs.github.com/en/copilot/concepts/agents/copilot-memory)
- **Results.** Code review offline: +3% precision and +4% recall. Coding agent A/B: PR merge rate 83% → 90%. Code-review A/B: positive feedback 75% → 77%. Both A/B results reported at p < 0.00001. [GitHub blog](https://github.blog/ai-and-ml/github-copilot/building-an-agentic-memory-system-for-github-copilot/)
- **Adversarial test.** The team seeded memories that contradicted the code or cited nonexistent locations. Agents "consistently verified citations" and the memory pool "self-healed". The report is qualitative, with no numbers. [same](https://github.blog/ai-and-ml/github-copilot/building-an-agentic-memory-system-for-github-copilot/)
- *Implication:* this is the strongest production evidence on staleness, and it favours **validation at use** over review dates. It also uses **usage-driven retention**: entries that keep being used and validated survive.

### Claude Code: CLAUDE.md, rules, auto memory

- **Loading.** CLAUDE.md files above the working directory load at launch. CLAUDE.md files in subdirectories load on demand, when Claude touches a file in that subdirectory. `.claude/rules/` files with `paths:` frontmatter load only for matching files. `@imports` "don't reduce its context cost". [Claude Code docs, accessed 2026-10-19](https://code.claude.com/docs/en/memory)
- **Size.** "target under 200 lines per CLAUDE.md file. Longer files consume more context and reduce adherence." Claude Code warns at startup when a file, or the files combined, exceed the limit. [same](https://code.claude.com/docs/en/memory)
- **Trust level.** CLAUDE.md "is delivered as a user message after the system prompt, not as part of the system prompt itself". It is context, not enforcement; use hooks to block actions. [same](https://code.claude.com/docs/en/memory)
- **Auto memory.** Claude writes the notes itself, scoped per repository and shared across worktrees. They are typed `user`, `feedback`, `project` or `reference`. Claude "skips anything it can derive from the codebase" and anything already in CLAUDE.md. [same](https://code.claude.com/docs/en/memory)
- **Index plus bodies on demand.** `MEMORY.md` is "one line per memory". Its first 200 lines or 25 KB load every session. Topic files are read on demand with ordinary file tools. When the index nears its limit, Claude Code reminds Claude to shorten it ("merge or drop stale entries"). Over the limit, the write succeeds but returns an error, because the rest is dropped at the next load. [same](https://code.claude.com/docs/en/memory)
- **Audit.** `/doctor prompt-audit` looks for outdated, contradictory or dangling instructions and proposes edits without applying them. The `InstructionsLoaded` hook logs which instruction files loaded and why. [same](https://code.claude.com/docs/en/memory)
- **Retrieval method.** "Claude Code doesn't use RAG currently. In our testing we found that agentic search out-performed RAG". [Boris Cherny, HN, 2025-02-24](https://news.ycombinator.com/item?id=43164253)

### Claude API memory tool and context editing

- **Mechanism.** The tool is client-side and file-based (`view`, `create`, `str_replace`, `insert`, `delete`, `rename` under `/memories`). The API auto-injects "ALWAYS VIEW YOUR MEMORY DIRECTORY BEFORE DOING ANYTHING ELSE." The application owns the storage. [Claude docs, accessed 2026-10-19](https://platform.claude.com/docs/en/agents-and-tools/tool-use/memory-tool)
- **Operator guidance.** Cap file sizes and cap what `view` returns. "Periodically delete memory files that haven't been accessed in a long time." Validate paths against traversal. Strip sensitive data before writing. [same](https://platform.claude.com/docs/en/agents-and-tools/tool-use/memory-tool)
- **Results.** On an internal agentic-search eval, memory plus context editing gave +39% over baseline, and context editing alone gave +29%. On a 100-turn web-search eval, context editing cut token use by 84%. [Anthropic, 2025-09-29](https://claude.com/blog/context-management) Self-reported; task details are not public.

### Cursor Rules and Memories

- Cursor staff on 2025-11-25: Memories were "intentionally removed starting from version 2.1.x". The recommended path is "Export memories" to an `.mdc` file and moving the content into User or Project Rules. No reason was given. [Cursor forum](https://forum.cursor.com/t/are-my-memories-gone/144057) A later staff reply in another thread says only the management UI was removed. The record is inconsistent (see Unverified).

### Windsurf (Cascade), now under Devin docs

- **Memories.** Auto-generated, workspace-scoped, stored locally and "not committed to your repository". Retrieved "when it believes they're relevant". They apply only to the legacy Cascade agent; the Devin Local agent does not persist them. [Devin/Windsurf docs, accessed 2026-10-19](https://docs.devin.ai/desktop/cascade/memories)
- **Rules.** Activation modes are `always_on`, `model_decision` (only the description is shown; the body is read when relevant), `glob` and `manual`. Limits: 6,000 characters for the global rules file and 12,000 per workspace rule file. For knowledge Cascade should reliably reuse, the docs recommend Rules or AGENTS.md over memories, because rules are "version-controlled, shareable with your team". [same](https://docs.devin.ai/desktop/cascade/memories)

### Devin Knowledge

- **Retrieval.** Trigger-based: "Devin retrieves Knowledge when relevant, not all at once". An item pinned to a repo "is always used" in that repo. [Devin docs, accessed 2026-10-19](https://docs.devin.ai/product-guides/knowledge)
- **Writes.** Devin proposes items, and the user edits or dismisses them before they are saved. Items should be "a handful of sentences", one workflow per item. Disable rather than delete. [same](https://docs.devin.ai/product-guides/knowledge)
- The page says Knowledge is being replaced by Skills (`SKILL.md` committed to the repo). [same](https://docs.devin.ai/product-guides/knowledge), [Skills](https://docs.devin.ai/product-guides/skills)

### Cline Memory Bank, OpenAI Codex, Kiro, Augment

- **Cline Memory Bank** is six Markdown files. The agent "MUST read ALL memory bank files at the start of EVERY task". Updates happen on milestones or on "update memory bank". There is no retrieval and no cost discussion. [Cline docs](https://docs.cline.bot/prompting/cline-memory-bank) Cline also refuses codebase indexing, arguing that chunking tears code logic apart, an index is "a snapshot frozen in time", and embeddings are a second copy of the code to secure. The post contains no measurements. [Cline blog, 2025-05-27](https://cline.bot/blog/why-cline-doesnt-index-your-codebase-and-why-thats-a-good-thing)
- **OpenAI Codex** concatenates AGENTS.md from the global level and from the repo root down to the current directory, at most one file per directory, with `AGENTS.override.md` taking precedence. The combined cap is 32 KiB (`project_doc_max_bytes`), and the chain is built once per run. [OpenAI docs, accessed 2026-10-19](https://learn.chatgpt.com/docs/agent-configuration/agents-md)
- **Amazon Kiro** steering inclusion modes are `always` (the default), `fileMatch`, `manual` and `auto` (loaded when the request matches the description). The docs say to "Treat steering changes like code changes - require reviews", and unresolved references are marked visibly. [Kiro docs](https://kiro.dev/docs/steering/)
- **Augment.** "Memories are stored locally and are applied to all Agent requests". They are added by the agent, a remember button, or manual edits. [Augment docs](https://docs.augmentcode.com/using-augment/agent) No staleness mechanism is documented.

### Cross-vendor pattern

| Concern | What production systems do | Draft |
|---|---|---|
| Always loaded | Short human-curated rules (Codex 32 KiB cap, Windsurf 6k/12k characters, CLAUDE.md under 200 lines) or a one-line index (Claude `MEMORY.md`); Copilot loads recent memories | Pinned full text only, about 300 units |
| On demand | Path or glob triggers (Claude rules, Kiro, Windsurf), description triggers (Devin, Kiro `auto`, Windsurf `model_decision`, Skills), file reads | `genie_memory search` and `show`; path linking deferred |
| Staleness | Copilot: citation validation plus 28-day expiry when unused. Others: manual review, audits, lint | `review_at` calendar timer |
| Who writes | Copilot: activity of users with write access. Devin: user-approved suggestions. Cursor and Windsurf moved reliable knowledge to versioned rules | Any agent adds; pinning is restricted |

Trend from 2025 to 2026: Cursor removed Memories, Windsurf limits memories to the legacy agent and recommends rules, and Devin moved Knowledge to Skills. Agent-written memory survives in products that added validation (Copilot) or a hard index cap with a "skip derivable" rule (Claude Code).

## 2. Empirical studies: context files, memory and experience reuse

- **ETH, "Evaluating AGENTS.md" ([arXiv 2602.11988](https://arxiv.org/abs/2602.11988), v1 2026-02-12, v3 2026-09-29).** Setup: CTXbench (138 instances, 12 Python repos with developer-committed context files) and SWE-bench Lite (300 tasks), run on Claude Code/Sonnet-4.5, Codex/GPT-5.2 and GPT-5.1 mini, and Qwen Code. [HTML](https://arxiv.org/html/2602.11988)
  - **LLM-generated files:** −0.5 pp (SWE-bench) and −2 pp (CTXbench); neither is significant. Cost rose about 20% and 23%, with +2.45 and +3.92 steps.
  - **Developer-written files:** +2.4 pp on CTXbench (p = 0.21, not significant), up to +19% cost and +3.34 steps.
  - **Overviews:** they did not reduce the steps needed to reach the relevant file.
  - **Mentions drive behaviour:** a tool mentioned in the file was used about 1.6 times per instance against under 0.01 otherwise.
  - **Recommendation:** omit LLM-generated files; human-written files should contain only what the README lacks (conventions, non-functional requirements). File length showed no clear correlation with success or cost.
  - *Implication:* every line of the default block costs money on every call and should earn its place as a non-standard rule. The "pinned facts exceed N, use `genie_memory search`" hint will raise tool-call counts. Measure whether those calls pay off.
- **Lulla et al., "On the Impact of AGENTS.md Files on the Efficiency of AI Coding Agents" ([arXiv 2601.20404](https://arxiv.org/abs/2601.20404), 2026-01-28, rev. 2026-03-30).** On 124 PRs in 10 repos, AGENTS.md was associated with 28.64% lower median runtime and 16.58% fewer output tokens, with comparable completion. This is an efficiency result that partly contradicts the ETH cost finding. The two designs differ: real PRs against benchmark instances.
- **Copilot Memory A/B:** +7 pp PR merge rate (§1). This is the only production A/B result for agent-written repository memory that I found.
- **Experience reuse:**
  - **Agent Workflow Memory:** +24.6% relative success on Mind2Web and +51.1% on WebArena. [arXiv 2409.07429, 2024-09-11](https://arxiv.org/abs/2409.07429)
  - **ReasoningBank (ICLR 2026):** SWE-bench Verified 34.2 → 38.8 (Gemini-2.5-flash) and 54.0 → 57.4 (Gemini-2.5-pro); up to 16% fewer steps. Adding failed trajectories raised WebArena-Shopping from 46.5 to 49.7, while the same change *lowered* AWM from 44.4 to 42.2. [arXiv 2509.25140](https://arxiv.org/html/2509.25140) Failures are useful only when distilled into strategies; raw failure logs are not.
  - **SWE-Exp:** an experience bank of successful and failed repair attempts. Its SWE-bench Verified number differs between versions: 41.6% (v1, open-source models) and 73.0% (v2, Claude 4 Sonnet). [arXiv 2507.23361](https://arxiv.org/abs/2507.23361)
  - *Implication:* the `tried` kind has evidence behind it, provided entries are distilled lessons with a source, not transcripts.
- **Memory quality, Xiong et al. ([arXiv 2505.16067](https://arxiv.org/html/2505.16067), 2025-05).** Agents show "experience-following": similar inputs produce outputs similar to the retrieved record, and errors propagate. Strict selective addition beat add-all by +25.5 pp (CIC-IoT), +25.45 (EHRAgent), +18.7 (AgentDriver) and +15.5 (RegAgent). History-based deletion added −1.2 to +4.2 pp. The authors suggest that later task outcomes serve as "free quality labels" for stored memory.
- **ACE ([arXiv 2510.04618](https://arxiv.org/abs/2510.04618), 2025-10, ICLR 2026 submission).** It names "brevity bias" and "context collapse": "iterative rewriting erodes details over time". Incremental delta updates gave +10.6% on agent benchmarks.

## 3. Context-length degradation and progressive disclosure

- **Lost in the Middle.** Accuracy is highest when relevant information sits at the start or end of the context and degrades in the middle. [Liu et al., arXiv 2307.03172, TACL](https://arxiv.org/abs/2307.03172) Published 2023–24, older than the preferred window.
- **NoLiMa (ICML 2025).** When the needle and the question share no literal words, 11 of 13 models fell below 50% of their short-context score at 32K tokens; GPT-4o went from 99.3% to 69.7%. [arXiv 2502.05167](https://arxiv.org/abs/2502.05167) This is also an argument that lexical match is what models, and BM25, find easiest.
- **Chroma, "Context Rot" (July 2025).** Across 18 models, performance drops with input length even on simple tasks, and distractors compound the drop. On LongMemEval, focused prompts (about 300 tokens) beat full prompts (about 113k tokens) for all models. [Chroma research](https://www.trychroma.com/research/context-rot)
- **Anthropic context engineering (2025-09-29).** Context is "a finite resource with diminishing marginal returns". The post recommends just-in-time retrieval: keep lightweight identifiers ("file paths, stored queries, web links") and load content with tools. Claude Code is the hybrid example: CLAUDE.md up front, grep and glob just in time. [Anthropic engineering](https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents)
- **Agent Skills (2025-10-16).** Three levels: name and description in the system prompt, `SKILL.md` body on relevance, linked files only when needed. [Anthropic engineering](https://www.anthropic.com/engineering/equipping-agents-for-the-real-world-with-agent-skills)
- **Tool Search Tool (2025-11-24).** `defer_loading` reduced tool-definition tokens by 85% (about 77K → 8.7K). MCP-eval accuracy rose from 49% to 74% (Opus 4) and from 79.5% to 88.1% (Opus 4.5). Recommended when definitions exceed 10K tokens or there are 10+ tools. [Anthropic engineering](https://www.anthropic.com/engineering/advanced-tool-use)
- **Prompt caching.** The cache key is a cumulative prefix hash in the order `tools` → `system` → `messages`. "Changing any block at or before the breakpoint produces a different hash." A cache read costs 0.1× and a 5-minute write 1.25× the base input price. [Claude docs](https://platform.claude.com/docs/en/build-with-claude/prompt-caching)
- *Implication:* a 300-unit block is small next to these degradation regimes. The main costs are per-call overhead (ETH) and misplaced trust, not attention dilution. Ordering by `pinned_at` keeps the block byte-identical between pin changes. Any pin change still changes the block's hash, so the ordering does little for the cache. The stronger reason for any fixed order is reproducibility in `preview`.

## 4. General memory systems and benchmarks

- **MemGPT/Letta.** OS-style "virtual context management" moves data between tiers. [arXiv 2310.08560, 2023](https://arxiv.org/abs/2310.08560) Letta core memory blocks are "always visible - no retrieval needed". Each block has a `label`, a `description`, a character `limit` and an optional `read_only`. [Letta docs](https://docs.letta.com/guides/agents/memory-blocks) This matches the draft's pinned block with a hard cap and a write restriction (`read_only` ≈ no agent pin).
- **Letta, "Benchmarking AI Agent Memory" (2025-08-12).** A GPT-4o-mini agent with plain file tools (grep, `search_files`, `open`) scored 74.0% on LoCoMo, against the 68.5% Mem0 reported for its graph variant (different models and harnesses, so indicative only). Conclusion: "memory is more about how agents manage context than the exact retrieval mechanism used". Simple, familiar tools are used more reliably than complex ones. [Letta blog](https://www.letta.com/blog/benchmarking-ai-agent-memory)
- **Mem0 ([arXiv 2504.19413](https://arxiv.org/html/2504.19413), 2025-04-28).** LoCoMo J scores: full-context **72.9**, Mem0g 68.4, Mem0 66.9, Zep 66.0, best RAG 61.0, LangMem 58.1, OpenAI 52.9. p95 latency: 17.1 s (full context) against 1.44 s (Mem0). Footprint: about 26k context tokens per query for full context against 1,764 memory tokens per query for Mem0; Mem0's stored memory is about 7k tokens per conversation (Table 2, §4.5). The paper's headline "26% over OpenAI" hides that full context was the most accurate. Mem0's own numbers for other systems are disputed (Letta could not reproduce its MemGPT results).
- **Zep/Graphiti ([arXiv 2501.13956](https://arxiv.org/html/2501.13956), 2025-01-20).** Bi-temporal design: timeline T holds `t_valid`/`t_invalid` (when a fact was true in the world), and T′ holds `t'created`/`t'expired` (ingestion audit). Contradicted edges get `t_invalid` instead of being deleted. LongMemEval with gpt-4o: full context 60.2% against Zep 71.2%, 115k against 1.6k tokens, 28.9 s against 2.58 s. The single-session-assistant category fell from 94.6% to 80.4%. *Implication:* the draft's `superseded_by` plus `created` is a one-timeline version of this. A `valid_from` field is unnecessary until someone asks "what was true on date X".
- **A-MEM ([arXiv 2502.12110](https://arxiv.org/abs/2502.12110), NeurIPS 2025).** Zettelkasten-style notes with links, plus "memory evolution" that rewrites older notes. The abstract gives no numbers. Given the ACE and Xiong results, rewriting older notes risks erosion and error propagation.
- **LongMemEval ([arXiv 2410.10813](https://arxiv.org/abs/2410.10813), ICLR 2025).**
  - **Long context:** GPT-4o fell from 0.870 (oracle) to 0.606 with the full history.
  - **Fact-augmented keys:** +9.4% recall@k and +5.4% accuracy.
  - **Time-aware query expansion:** +6.8% to +11.3% recall.
  - **Retrievers ([HTML](https://arxiv.org/html/2410.10813)):** "Both dense retrieval embeddings have significantly higher performance than the BM25". Session-level Recall@5 was 0.634 (BM25), 0.723 (Contriever) and 0.720 (Stella); round-level, 0.472 against 0.660 (Stella).
- **Lexical against dense retrieval more broadly:**
  - **BEIR (NeurIPS 2021, older):** BM25 is a strong zero-shot baseline that dense retrievers "often underperform". [arXiv 2104.08663](https://arxiv.org/abs/2104.08663)
  - **Cursor (2025-11-06):** semantic search on top of grep gave +12.5% average answer accuracy (6.5–23.5% by model). Code retention rose 0.3% overall and 2.6% on repos of 1,000+ files. "The combination of these two leads to the best outcomes". [Cursor blog](https://cursor.com/blog/semsearch)
  - **Sourcegraph:** removed embeddings in 5.3 in favour of its search, citing scale and maintenance. [Cody FAQ](https://sourcegraph.com/docs/cody/faq)
  - *Implication:* the case for BM25 rests on curated, short, term-bearing entries, an agent that can re-query, and keeping data local. The cost is a measurable recall gap on paraphrased queries. Fact-augmented keys, which LongMemEval found useful, map onto the draft's `term` and `refs` columns.

## 5. Risks

- **Injection without write access.** MINJA ([arXiv 2503.03704](https://arxiv.org/html/2503.03704), v1 2025-03, v5 2026-02) injects malicious records "by only interacting with the agent via queries". Averages: 98.2% injection success, 76.8% attack success. Detection prompts either missed (RAP: 0 of 135 flagged) or produced false positives (34 of 50 on one setup). Malicious and benign records were "highly entangled in the embedding space".
- **Poisoned knowledge bases.** AgentPoison ([arXiv 2407.12784](https://arxiv.org/abs/2407.12784), 2024-07) reached >80% attack success at a <0.1% poison rate, with <1% impact on benign tasks.
- **Delayed tool invocation.** Injected text plants a trigger, and the agent writes attacker-chosen memories when the user later says the trigger word. Demonstrated on Gemini (Feb 2025) and ChatGPT (2024). Rehberger recommends confirmation for dangerous memory operations and limits on how many memories one turn can write. [Embrace The Red, 2025](https://embracethered.com/blog/posts/2025/gemini-memory-persistence-prompt-injection/), [2024](https://embracethered.com/blog/posts/2024/chatgpt-hacking-memories/)
- **OWASP ASI06, Memory & Context Poisoning (Top 10 for Agentic Applications, 2025-12-09).** The ASI06 co-lead says memory "should be treated as part of the attack surface" and gets the same scrutiny as credentials. [OWASP, 2025-12-09](https://genai.owasp.org/2025/12/09/owasp-top-10-for-agentic-applications-the-benchmark-for-agentic-security-in-the-age-of-autonomous-ai/), [OWASP, 2026-05-13](https://genai.owasp.org/2026/05/13/memory-is-a-feature-it-is-also-an-attack-surface/) The OWASP Agent Memory Guard reference project uses integrity baselines, read and write policies, and snapshot rollback. [OWASP project](https://owasp.org/www-project-agent-memory-guard/)
- **Staleness and drift.** Copilot's design treats this as "the hard problem" (§1). Claude Code ships an audit for "references to files or commands that don't exist". Kiro marks unresolved references visibly.
- **Write amplification and erosion.** Add-all memory underperforms selective addition by 15–26 pp (Xiong). Iterative rewriting erodes detail (ACE). Cursor removed auto-memories. The Claude API docs add an anti-clutter instruction. Together these support the draft's open question 4 ("explicit add, otherwise memory becomes a feed").
- *Implications for genie:*
  - Store and show `author` and `source` on every search result, not only in the block.
  - Have executor-written entries start unconfirmed: hidden from other roles' search, or ranked last, until confirmed by a human or the PM role, or until a later successful task cites them.
  - Rate-limit writes per turn.
  - Keep the "data, not instruction" framing on tool output too.

## 6. SQLite FTS5 for Russian and English

Sources: [sqlite.org/fts5.html](https://www.sqlite.org/fts5.html) and [release history](https://www.sqlite.org/changes.html), accessed 2026-10-19.

- **`unicode61` (default).** Case-insensitive per Unicode 6.1. `remove_diacritics` takes `0`, `1` (default) or `2`, and diacritics are removed "from Latin script characters" only. Options: `categories` (default `L* N* Co`), `tokenchars` and `separators`. Genie's `pages_fts` already uses `unicode61 remove_diacritics 2` (`crates/genie-core/src/vault.rs`).
- **`porter`.** A wrapper tokenizer whose Porter algorithm "is designed for use with English language terms". It does nothing useful for Russian.
- **`trigram` (SQLite 3.34.0, 2020-12-01, per the [release log](https://www.sqlite.org/releaselog/3_34_0.html); the FTS5 page gives no version).** Matches substrings. Options are `case_sensitive` (default 0) and `remove_diacritics` (0 or 1, only with case-insensitive matching). It also supports indexed `LIKE`/`GLOB` unless `remove_diacritics` is set. Caveats: substrings shorter than 3 characters match nothing; `detail=none|column` limits full-text tokens to 3 characters; `LIKE ... ESCAPE` cannot use the index. The index is larger than a token index.
- **Locale and custom tokenizers.** SQLite 3.47.0 (2024-10-21) added the `fts5_tokenizer_v2` API and the `locale=1` option for locale-aware custom tokenizers.
- **Snowball.** [`abiliojr/fts5-snowball`](https://github.com/abiliojr/fts5-snowball) is a loadable C extension (`tokenize = 'snowball russian english unicode61'`). Its last push was 2021-05-29. With `rusqlite` `bundled` (genie uses 0.40.2), it would need extension loading or a Rust-registered tokenizer. The Rust crate `rust-stemmers` 1.2.0 ships `Algorithm::Russian` and `Algorithm::English`; it is old (2019) but widely used. I did not verify a rusqlite path for registering FTS5 tokenizers (`rusqlite-ext` claims one).
- **`bm25(table, w1, w2, …)`.** Weights follow column order, and missing weights default to 1.0. Lower scores are better: the result is multiplied by −1 so that `ORDER BY bm25(t)` sorts best first. k1 = 1.2 and b = 0.75 are hard-coded. With a corpus of a few hundred rows, IDF is noisy, so the draft's weights (8/1/2/5) should be calibrated on a fixed query set.
- **My test on SQLite 3.54.0 (Python `sqlite3`):**

  | Query | `unicode61 remove_diacritics 2` | `trigram` |
  |---|---|---|
  | `vygruzka` | matches capitalized *Vygruzka* (case folded) | matches |
  | `vygruzk*` | matches *vygruzka* and *vygruzki* | — |
  | `"vygruzk"` | no match | matches both |
  | *otchetov* spelled with e | does **not** match the spelling with yo | does **not** match the spelling with yo |
  | `CS` (2 characters) | — | no match |

- *Implications for genie:*
  - Keep `normalize()` (yo to e). FTS5 will not fold Cyrillic yo.
  - Cheapest Russian recall fix: rewrite each query token of 5+ characters into a prefix query (strip to a stem-like prefix and add `*`), or store a Snowball-stemmed shadow column built in Rust and stem queries the same way. Both are deterministic and testable, unlike embeddings.
  - Trigram is an alternative when substring and identifier search matters (`snake_case` fragments, paths).
  - The draft's claim that *vygruzka* and *vygruzki* are different queries holds for exact-token MATCH only.

## 7. Measuring whether memory pays off

Metrics the sources used:

| Source | Metric |
|---|---|
| GitHub Copilot | Offline precision and recall on PR review; online A/B on PR merge rate and positive comment feedback |
| Cursor | Offline QA accuracy (Context Bench); online A/B on code retention and dissatisfied follow-up requests |
| ETH 2602.11988 | Task success, cost, steps, reasoning tokens, steps to the first relevant file, usage of mentioned tools |
| Lulla 2601.20404 | Wall-clock time and output tokens on the same PRs, with and without the file |
| Anthropic | Internal eval score delta; token consumption over 100 turns |
| Mem0 and Zep | Answer accuracy (LLM judge), p95 latency, context tokens per query |
| Xiong 2505.16067 | Downstream success with add-all against selective addition; later task outcome as a label for stored records |

*Implications for genie (the draft's metrics plus the missing ones):*

- **A/B or interleaved runs on a fixed task set**, with the block on and off and the tool on and off. Measure tokens, steps, wall time and task outcome. Without this, none of the draft's metrics show savings.
- **Retrieval log:** query, returned IDs, which IDs were then `show`n or cited, and zero-result rate. Follow each zero result with a check of whether a relevant entry existed; that check gives a lexical-miss rate and is the trigger for stemming or embeddings.
- **Per-entry utility:** times served, times cited, and outcome of tasks that used the entry. This lets retention be driven by use, as in Copilot, rather than by calendar.
- **Cost of the default block:** units per call multiplied by calls per day, against the measured savings above.

## 8. Mnemopi claims checked against source

I checked `@oh-my-pi/pi-mnemopi` 18.12.2 (npm, published 2026-10-18) and the [oh-my-pi repo](https://github.com/can1357/oh-my-pi/tree/main/packages/mnemopi) at commit `5d72f5d` (2026-10-19).

| Draft claim | Verdict |
|---|---|
| "30k lines" | **Roughly confirmed:** `src` 20,422 lines of TypeScript, `test` 12,093, `bench` 201, about 32.7k in total. The engine alone is about 20k. |
| "14 kinds" | **Confirmed:** `MemoryType` in `core/typed-memory.ts` has 14 values, including `unknown`. |
| "9 priority classes" | **Confirmed:** `TypePriority` has 9 values. |
| "four voices of search" | **Refuted in detail:** `PolyphonicVoice` has **5** values: hybrid, vector, graph, fact, temporal. |
| "decay table (14 kinds × 2 params) imported only by tests" | **Confirmed in substance, wrong in detail:** `WEIBULL_PARAMS` in `core/weibull.ts` covers **21** types × 2 params (k, eta). `getDecayRate()` covers 14 types with 1 rate. Neither is imported anywhere in `src`; only `test/weibull-mmr-intent.test.ts` and `test/typed-memory-aaak.test.ts` use them. |
| "all kinds age by one curve 0.7 + 0.3·exp(−age/72 h)" | **Confirmed:** `core/beam/recall.ts` computes `score = baseScore * (0.7 + 0.3 * decay)` with `recencyDecay(ts, 72)` = `exp(-ageHours/72)`. The parameter is named `halfLifeHours` but acts as an e-folding time. |
| "confidence: `unknown` 0.8 > `tool` 0.5" | **Confirmed:** `VERACITY_WEIGHTS` are stated 1.0, unknown 0.8, inferred 0.7, imported 0.6, tool 0.5. They multiply the recall score (`recall.ts`: `score *= veracityWeight …`). |
| "six versioning columns declared and written by no one" | **Partly confirmed:** `validator`, `validated_at`, `validation_count` and `corrected_by` on `working_memory` have no write path in `src`. The `memory_validations` table (with a trim trigger) is never inserted into, and the MCP "attest" action returns a status without recording anything. "Six" was not established; `valid_until` and `superseded_by` *are* written. |
| "50+ tuning constants without a single measurement" | **Unverifiable as stated:** a crude count found 43 top-level numeric `UPPER_CASE` constants, with more inline. The only benchmark in the repo (`bench/native-vectors.bench.ts`) measures vector-kernel speed, not retrieval quality. |

## Sources

- GitHub blog, "Building an agentic memory system for GitHub Copilot", Jan 2026: https://github.blog/ai-and-ml/github-copilot/building-an-agentic-memory-system-for-github-copilot/
- GitHub docs, Copilot Memory: https://docs.github.com/en/copilot/concepts/agents/copilot-memory
- Claude Code docs, memory: https://code.claude.com/docs/en/memory
- Claude docs, memory tool: https://platform.claude.com/docs/en/agents-and-tools/tool-use/memory-tool
- Claude docs, prompt caching: https://platform.claude.com/docs/en/build-with-claude/prompt-caching
- Anthropic, context management, 2025-09-29: https://claude.com/blog/context-management
- Anthropic engineering, effective context engineering, 2025-09-29: https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents
- Anthropic engineering, Agent Skills, 2025-10-16: https://www.anthropic.com/engineering/equipping-agents-for-the-real-world-with-agent-skills
- Anthropic engineering, advanced tool use, 2025-11-24: https://www.anthropic.com/engineering/advanced-tool-use
- Boris Cherny, HN comment, 2025-02-24: https://news.ycombinator.com/item?id=43164253
- Cursor forum, Memories removal, 2025-11-25: https://forum.cursor.com/t/are-my-memories-gone/144057
- Cursor blog, semantic search, 2025-11-06: https://cursor.com/blog/semsearch
- Windsurf/Devin docs, Cascade memories and rules: https://docs.devin.ai/desktop/cascade/memories
- Devin docs, Knowledge: https://docs.devin.ai/product-guides/knowledge ; Skills: https://docs.devin.ai/product-guides/skills
- Cline docs, Memory Bank: https://docs.cline.bot/prompting/cline-memory-bank ; Cline blog, 2025-05-27: https://cline.bot/blog/why-cline-doesnt-index-your-codebase-and-why-thats-a-good-thing
- OpenAI Codex, AGENTS.md: https://learn.chatgpt.com/docs/agent-configuration/agents-md
- Kiro, steering: https://kiro.dev/docs/steering/
- Augment, Agent: https://docs.augmentcode.com/using-augment/agent
- Sourcegraph Cody FAQ: https://sourcegraph.com/docs/cody/faq
- arXiv 2602.11988 (ETH, AGENTS.md): https://arxiv.org/abs/2602.11988
- arXiv 2601.20404 (AGENTS.md efficiency): https://arxiv.org/abs/2601.20404
- arXiv 2505.16067 (experience-following): https://arxiv.org/abs/2505.16067
- arXiv 2510.04618 (ACE): https://arxiv.org/abs/2510.04618
- arXiv 2409.07429 (AWM): https://arxiv.org/abs/2409.07429
- arXiv 2509.25140 (ReasoningBank): https://arxiv.org/abs/2509.25140
- arXiv 2507.23361 (SWE-Exp): https://arxiv.org/abs/2507.23361
- arXiv 2307.03172 (Lost in the Middle): https://arxiv.org/abs/2307.03172
- arXiv 2502.05167 (NoLiMa): https://arxiv.org/abs/2502.05167
- Chroma, Context Rot, July 2025: https://www.trychroma.com/research/context-rot
- arXiv 2310.08560 (MemGPT): https://arxiv.org/abs/2310.08560 ; Letta memory blocks: https://docs.letta.com/guides/agents/memory-blocks
- Letta blog, benchmarking memory, 2025-08-12: https://www.letta.com/blog/benchmarking-ai-agent-memory
- arXiv 2504.19413 (Mem0): https://arxiv.org/abs/2504.19413
- arXiv 2501.13956 (Zep): https://arxiv.org/abs/2501.13956
- arXiv 2502.12110 (A-MEM): https://arxiv.org/abs/2502.12110
- arXiv 2410.10813 (LongMemEval): https://arxiv.org/abs/2410.10813
- arXiv 2104.08663 (BEIR): https://arxiv.org/abs/2104.08663
- arXiv 2503.03704 (MINJA): https://arxiv.org/abs/2503.03704
- arXiv 2407.12784 (AgentPoison): https://arxiv.org/abs/2407.12784
- OWASP Top 10 for Agentic Applications, 2025-12-09: https://genai.owasp.org/2025/12/09/owasp-top-10-for-agentic-applications-the-benchmark-for-agentic-security-in-the-age-of-autonomous-ai/
- OWASP, "Memory Is a Feature. It Is Also an Attack Surface", 2026-05-13: https://genai.owasp.org/2026/05/13/memory-is-a-feature-it-is-also-an-attack-surface/
- OWASP Agent Memory Guard: https://owasp.org/www-project-agent-memory-guard/
- Embrace The Red (Rehberger), Gemini memory, 2025: https://embracethered.com/blog/posts/2025/gemini-memory-persistence-prompt-injection/ ; ChatGPT memory, 2024: https://embracethered.com/blog/posts/2024/chatgpt-hacking-memories/
- SQLite FTS5: https://www.sqlite.org/fts5.html ; release history: https://www.sqlite.org/changes.html
- fts5-snowball: https://github.com/abiliojr/fts5-snowball ; rust-stemmers: https://crates.io/crates/rust-stemmers
- Mnemopi: https://www.npmjs.com/package/@oh-my-pi/pi-mnemopi ; https://github.com/can1357/oh-my-pi/tree/main/packages/mnemopi

## Unverified / could not access

- **Cursor's reason for removing Memories.** Staff gave none. Staff replies also conflict: "intentionally removed" against "only the UI was removed". I found no official changelog entry.
- **Claude Code v2.1.50 change.** The OWASP post (2026-05-13) says Claude Code v2.1.50 "removed user memories from the system prompt" after disclosure. The public `CHANGELOG.md` entry for 2.1.50 does not mention it.
- **Copilot blog date.** The URL slug says 2026-01-09; the page says 2026-01-15. The number of memories Copilot loads per session is not disclosed.
- **Anthropic's +39% and 84% figures.** Internal evals with no public task definition.
- **Cherny's later statements.** The February 2026 X post and the Latent Space podcast are cited only second-hand. Only the 2025-02-24 HN comment was verified.
- **The ASI06 entry itself.** The official mitigation list on genai.owasp.org was not retrieved. Mitigations above come from the co-lead's post and the Memory Guard project.
- **Paper details.** MINJA's numbers come from the HTML first 100k characters. The A-MEM abstract has no numbers. ReasoningBank's relative percentages are my own arithmetic.
- **Rust FTS5 tokenizer path.** Whether `rusqlite` 0.40 can register a custom FTS5 tokenizer, or whether `rusqlite-ext` works, was not tested.
- **Augment.** Its internal memory retrieval is undocumented.
- **Mnemopi.** "Six never-written versioning columns" and "50+ constants" could not be reproduced exactly (§8).
