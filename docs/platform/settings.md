---
title: Settings and access
summary: Configure accounts, projects, provider keys, usage, and budgets.
type: reference
status: current
---

# Settings and access

Open `/settings`, use the sidebar gear, or press `G` then `S` for personal settings only: account, password, Telegram subscription, provider keys, and CLI/MCP tokens.

Personal settings also include **Appearance** (`/settings/me/appearance`). Choose a light or dark theme, follow the system appearance, and select one of six accent colors. Appearance preferences are stored in this browser and do not change other browsers or users.

The top bar has three destinations. Each label opens the section and its chevron opens a menu of the section's pages:

- **Tasks**: List (`/tasks`), Board (`/board`) and Epics (`/epics`). The label reopens the view used last.
- **Knowledge**: Documentation (`/docs`) and Proposals (`/docs/proposals`).
- **Management** (`/manage`): the project hub, shown under the same top bar as the work pages. Its rail groups the pages that the current person may open:
  - **General settings** (project and server administrators): Project (`/manage/project/general`), Knowledge base, Notifications (the project's Telegram bot) and Repositories and environment.
  - **Agents**: Roles, Team templates, Skills and MCP servers (`/manage/agents/...`), each showing the shared server catalog, plus Automations (rules, runs, observations and their sources). Skills and MCP servers have an **In the project** tab where project administrators choose what the project uses. Jev and Agent work mode are for project administrators. Server administrators edit the catalog.
  - **Administration** (server administrators only): Projects, Accounts, Project members, Providers, Server tools (MCP and Jev), Health, Knowledge vault, Statistics (`/manage/server/stats`: activity, costs and [Factory Analytics](task-intelligence.md#statistics)) and the agent configuration history.

Old addresses (`/agents`, `/statistics`, `/automations`, `/observations` and the earlier `/manage/project/...` sections) redirect to their new pages.

Contextual search filters the current board, task list, epics, documentation, proposals, agent catalog page, automation runs, notifications or statistics projects. Queries use `?q=` and preserve other filters and selections. On detail pages, search opens the task list. `/` focuses search; Escape clears it.

Project general settings include an optional project icon beside the name. Project administrators can upload PNG, JPEG or WebP images or remove the current icon. The browser fits the image into a 256px transparent square; the server stores at most 512 KiB and requires project membership to read it. The header and project switcher show the image, or a monogram derived from the project name when no image is available. The icon is shared by project members and survives browser reloads.

Sections with several views keep a URL per view: repositories, server projects, accounts and project members use `?tab=`, and the Skills, MCP servers and Automations pages use a path per tab. Tool selections and account creation retain drafts when switching tabs on the same page.

The Project page covers naming and the project icon; Agent work mode covers agent autonomy and result integration. Server tools keep MCP connections and the server's Jev settings; Health and Knowledge vault cover server checks and whole-vault synchronization. The Jev connection card shows server configuration and links to provider credentials. Endpoint, model, and enablement still require `decisions` in `config.json` and a server restart.

Project tool selections are stored in `<data>/project-tools/<slug>.json`, with optimistic version checks and configuration history. An omitted/null list inherits current and future server tools; an empty list selects none. Skill selections intersect explicit role lists; repository-local skills remain available. MCP selections intersect server project restrictions and role grants, including gateway authorization and running-agent policies. Changes affect skill loading on subsequent agent starts; MCP policy refresh follows the existing reload behavior. Broken selection files keep the last valid restriction, or deny all tools on a cold load.

Accounts supports creation, copying newly created login credentials, deletion, server administrator promotion, and assignment as administrator of a specific project. Copied passwords remain only in component memory until hidden or leaving the page; they are not returned by account APIs. Deletion cannot target the caller or the last active administrator, and removes sessions, tokens, and memberships while task history remains.

Old `/settings/project/*`, `/settings/server/*`, `/settings/stats`, `/project/*`, and `/server/*` links redirect to the appropriate destination, preserving query parameters on settings redirects. Administrative routes remain protected when opened directly or after changing project.

Pending capabilities have visible placeholders and tracked Feature Requests:

- **GNE-5:** project Git tokens and repository credentials.
- **GNE-6:** dependencies, preparation tools, and project sandbox configuration.
- **GNE-7:** per-member activity history.
- **GNE-8:** project backups, remote knowledge storage, and restore. Existing vault sync is server-wide and is not exposed to project administrators as a project backup facility.

Communication channels currently configures the existing project Telegram bot. Slack and Mattermost remain future integrations.

```bash
genie project tools shop                         # inspect selections and available tools
genie project tools shop --skills code-review --mcp docs
genie project tools shop --skills '' --mcp ''    # select none
genie project tools shop --all-skills --all-mcp  # inherit available server tools
genie user delete former-user                   # server administrator; irreversible
```

Configuration not exposed in the UI remains in `<data>/config.json`. Restart the server after changing that file. Role and team files reload separately without a restart.

## Task intelligence policy

Open **Management → Agents → Jev → Task assessment and mandatory checks**.
The policy is disabled by default. A human project administrator can enable it,
map independent checks to delivery reviewer roles, protect repository path prefixes,
and choose a safe fallback team. Optional semantic assessment uses the existing Jev
connection, external-context grant and provider allowance; it is separate from the
mandatory project rules.

Unknown affected paths retain checks for all protected components. Explicit human
team choices and subsequent resume/completion actions must satisfy mandatory roles.
Policy changes do not rewrite previous routing evidence. See the
[setup and workflow guide](task-intelligence.md#project-policy) before enabling it.

## Accounts and project roles

```bash
printf '%s\n' 'replace-with-a-strong-password' | genie user add anna --name "Anna" --admin --password-stdin
genie project invite shop --role member
genie project member shop anna member
genie user token anna
```

Invitations are single-use and expire after seven days. A server administrator has owner access to every project.

| Role | Access |
| --- | --- |
| `viewer` | Read project work. |
| `member` | Work with tasks and knowledge. |
| `admin` | Also manage project people, automations, and releases. |
| `owner` | Full project control. |

Disabling an account prevents login and invalidates its sessions and tokens. Renaming a login preserves membership, sessions, tokens, and task assignments.

## Provider keys

Use the **+** button in **Personal → Providers and keys** to connect a provider with OAuth or an API key. OAuth is offered for providers supported by pi; LiteLLM is available in the same provider list. Administrators can add shared API keys in **Management → Providers**; users can see that a shared key exists without seeing its value. Credentials stored in the database are encrypted using `<data>/secrets.key`; Pi's private per-user runtime profile is kept with owner-only permissions so refreshed OAuth tokens survive restarts.

The model selector loads the catalogue using the signed-in user's provider credentials. Agents use the credentials resolved for the person they work for. A shared LiteLLM key can serve people who have no personal key. Role model selection is separate: each runnable role must name a model in its [role configuration](agent-roles-and-teams.md).

## Network settings

```json
{
  "bind": "0.0.0.0",
  "allowHosts": ["genie.example.com"],
  "publicUrl": "https://genie.example.com"
}
```

`bind` selects the listening interface; `allowHosts` restricts accepted Host headers; `publicUrl` supplies links in notifications. For Docker, use the corresponding [Compose variables](docker.md#network-access).

## Usage and budgets

Genie records model token usage by day, model, agent, task, epic, and project. View it in **Statistics**, task and epic details, agent chats, or `genie stats`.

Prices are in dollars per million tokens. Configure overrides in `config.json`:

```json
{
  "modelPrices": {
    "provider/model-id": { "input": 2, "output": 12, "cacheRead": 0.5 }
  },
  "budgets": { "perTask": 5, "perEpic": 20, "perDay": 50 },
  "limits": { "maxActiveTeams": 4, "maxActiveTeamsPerEpic": 2 }
}
```

These prices are examples, not a provider tariff. Unknown prices display token counts without inventing a cost. Cache usage requires its own price. Costs are calculated when displayed, so a changed price also changes historical estimates.

LiteLLM tariffs load at startup and on explicit refresh:

```bash
genie server prices
genie server prices --refresh
```

Configure `litellm.baseUrl` and a server-only `litellm.infoToken` or `LITELLM_INFO_TOKEN` with permission to read `/model/info`. Field-level `modelPrices` overrides take precedence over those tariffs. A failed refresh retains the last successful prices and reports the error.

Budget values of zero or omitted values mean unlimited. Task budgets include children; epic budgets include all epic tasks; daily budgets apply per project using UTC days. On exhaustion, affected teams stop with reason `budget`, task authors and owners are notified, and new team launches are rejected. Enforcement follows reported usage, so a request already in flight can exceed the threshold.

Team limits bound concurrency independently of monetary budgets.

## Notifications

People receive web notifications and can link Telegram or email in personal settings.

Project administrators manage a dedicated bot in **Management → General settings → Notifications**. Create
it with `/newbot` in [BotFather](https://t.me/BotFather), then paste its token into Genie.
Genie validates the bot through `getMe`, rejects bots with webhooks and bots already
assigned to another project, seals the token with `secrets.key`, and starts long polling
without a restart. Use a bot dedicated to this Genie instance. Tokens never appear in
API responses or delivery errors. Re-enter the token if the encryption key is lost.

Members, including viewers, connect in the project's Telegram settings or choose a
project under **Personal → Telegram**. Open the private link or scan its locally
rendered QR code, then press **Start** in Telegram. The one-time link expires after
30 minutes; generating another replaces it. Connections are scoped to the project bot.
Only private chats can connect. Disconnecting revokes outstanding codes and queued
messages. Changing the bot identity or deleting it resets connections; rotating the
token of the same bot keeps them. A project with a configured bot never falls back to
the server bot for a disconnected user.

Task authors receive comment and status notifications automatically. Other people can
press **Follow** on any accessible task; following is independent of assignment.
Own actions are silent, and users who lose project access receive no further messages.
Subscriptions survive disconnecting Telegram. Notifications remain available in the
web; Telegram is used when connected, otherwise configured email is used. Project bot
questionnaires contain a link to answer on the web; the legacy server bot retains
its existing reply and `/new` commands.

The optional server-wide bot below remains available for projects without their own bot. Task assignments, mentions, owner questions, and knowledge proposals route to the relevant people. Automations can add project-specific notifications.

Configure channel credentials in `config.json` and restart the server:

```json
{
  "telegram": { "token": "BOT_TOKEN" },
  "smtp": {
    "host": "smtp.example.com",
    "port": 587,
    "security": "starttls",
    "username": "genie",
    "password": "SMTP_PASSWORD",
    "from": "Genie <genie@example.com>"
  }
}
```

Replace the placeholder credentials and omit unused channels. Telegram uses long polling, so it needs no public incoming endpoint. Generate a linking code in Personal settings and send `/start CODE` to the bot. `/new <text>` creates an inbox task; reply to a question message to answer it.

Email delivery uses SMTP; incoming email ingestion is not implemented. Set `publicUrl` so notification links open the correct server. See the [automation reference](automations.md) for recipients and triggers.
