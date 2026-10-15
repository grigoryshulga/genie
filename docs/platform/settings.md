---
title: Settings and access
summary: Configure accounts, projects, provider keys, usage, and budgets.
type: reference
status: current
---

# Settings and access

Open `/settings`, use the sidebar gear, or press `G` then `S`. The menu separates personal settings from administration:

| Scope | Contents | Who can change it |
| --- | --- | --- |
| Personal | Account, password, Telegram, provider keys, CLI/MCP tokens | The account owner |
| Project settings | General settings, agent autonomy, integration, repositories, and people | Project administrators and owners |
| Automations | Project rules and playbooks | Project administrators and owners |
| Management | Health, server projects, accounts, providers, MCP, and vault | Server administrators |
| Agents | Roles, team templates, skills, and edit history | Server administrators |
| Statistics | Project activity, agent runs, and costs | Server administrators |

Project settings and Automations use the selected project. Server administration is separate from the working catalog. Routes remain under `/settings/project/*`, `/settings/server/*`, and `/settings/stats`.

Configuration not exposed in the UI remains in `<data>/config.json`. Restart the server after changing that file. Role and team files reload separately without a restart.

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

Personal provider keys override server-wide defaults. Administrators set shared keys in **Management → Providers**; users can see that a shared key exists without seeing its value. Keys stored in the database are encrypted using `<data>/secrets.key`.

Agents use the credentials resolved for the person they work for. A shared LiteLLM key can serve people who have no personal key. Role model selection is separate: each runnable role must name a model in its [role configuration](agent-roles-and-teams.md).

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

Genie records model token usage by day, model, agent, task, epic, and project. View it in **Settings → Statistics**, task and epic details, agent chats, or `genie stats`.

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

People receive web notifications and can link Telegram or email in personal settings. Task assignments, mentions, owner questions, and knowledge proposals route to the relevant people. Automations can add project-specific notifications.

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
