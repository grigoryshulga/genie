//! Human delivery preferences and durable tracking of copies of authoritative interactions.
use crate::{GenieError, Result, server_db::ServerDb};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::json;

pub const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS delivery_policies (
  user INTEGER PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
  policy TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS interaction_deliveries (
  outbox INTEGER PRIMARY KEY REFERENCES outbox(id) ON DELETE CASCADE,
  project TEXT NOT NULL,
  reference TEXT NOT NULL,
  version TEXT NOT NULL,
  recipient INTEGER NOT NULL,
  closed INTEGER NOT NULL DEFAULT 0,
  checked_at TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS interaction_deliveries_open ON interaction_deliveries(project,closed,checked_at,outbox);
INSERT OR IGNORE INTO interaction_deliveries(outbox,project,reference,version,recipient)
SELECT id, json_extract(payload,'$.project'),
  COALESCE(json_extract(payload,'$.interaction.id'),CASE
    WHEN json_extract(payload,'$.interaction.proposal') IS NOT NULL THEN json_extract(payload,'$.project')||':proposal:'||json_extract(payload,'$.interaction.proposal')
    WHEN json_extract(payload,'$.interaction.question') IS NOT NULL THEN json_extract(payload,'$.project')||':clarification:'||json_extract(payload,'$.interaction.task')||':'||json_extract(payload,'$.interaction.question') END),
  CAST(json_extract(payload,'$.interaction.version') AS TEXT),user
FROM outbox WHERE json_valid(payload) AND user IS NOT NULL AND json_extract(payload,'$.project') IS NOT NULL
  AND json_extract(payload,'$.interaction.version') IS NOT NULL
  AND (json_extract(payload,'$.interaction.id') IS NOT NULL OR json_extract(payload,'$.interaction.proposal') IS NOT NULL OR json_extract(payload,'$.interaction.question') IS NOT NULL);
INSERT OR IGNORE INTO interaction_deliveries(outbox,project,reference,version,recipient)
SELECT o.id,s.project,s.project||':questionnaire:'||s.id||':'||q.n,s.created||':'||q.n,s.recipient
FROM outbox o JOIN questionnaires s ON s.id=json_extract(o.payload,'$.question.questionnaire')
JOIN questions q ON q.questionnaire=s.id AND q.n=json_extract(o.payload,'$.question.n')
WHERE json_valid(o.payload) AND o.user=s.recipient;
"#;

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeliveryPolicy {
    // Ordered external channels. Empty means web only; absent policy preserves legacy routing.
    pub order: Vec<String>,
    pub fallback_after_attempts: i64,
}
impl DeliveryPolicy {
    pub fn validate(&self) -> Result<()> {
        let mut seen = std::collections::BTreeSet::new();
        if !(1..=crate::inbox::MAX_DELIVERY_ATTEMPTS).contains(&self.fallback_after_attempts)
            || self.order.iter().any(|c| !matches!(c.as_str(), "telegram" | "email") || !seen.insert(c))
        {
            return Err(GenieError::invalid(
                "delivery order must contain unique telegram/email channels; fallbackAfterAttempts must be 1..8",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct InteractionDelivery {
    pub outbox: i64,
    pub project: String,
    pub reference: String,
    pub version: String,
    pub recipient: i64,
}

#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export)]
pub struct InteractionDeliveryReceipt {
    pub channel: String,
    pub status: String,
    pub attempts: i64,
}

impl ServerDb {
    /// Receipt status is visible to the intended recipient; channel addresses and transport
    /// error details never become part of another curator's shared task view.
    pub fn interaction_receipts(
        &self,
        project: &str,
        reference: &str,
        version: &str,
        user: i64,
    ) -> Result<Vec<InteractionDeliveryReceipt>> {
        let mut stmt=self.conn().prepare("SELECT o.channel,o.status,o.attempts FROM interaction_deliveries d JOIN outbox o ON o.id=d.outbox WHERE d.project=?1 AND d.reference=?2 AND d.version=?3 AND d.recipient=?4 ORDER BY o.id")?;
        Ok(stmt
            .query_map(params![project, reference, version, user], |r| {
                Ok(InteractionDeliveryReceipt { channel: r.get(0)?, status: r.get(1)?, attempts: r.get(2)? })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }
    pub fn delivery_policy(&self, user: i64) -> Result<Option<DeliveryPolicy>> {
        let value: Option<String> =
            self.conn().query_row("SELECT policy FROM delivery_policies WHERE user = ?1", [user], |r| r.get(0)).optional()?;
        value.map(|v| serde_json::from_str(&v).map_err(GenieError::from)).transpose()
    }
    pub fn set_delivery_policy(&self, user: i64, policy: Option<&DeliveryPolicy>) -> Result<()> {
        self.user(user)?;
        if let Some(policy) = policy {
            policy.validate()?;
            self.conn().execute(
                "INSERT INTO delivery_policies VALUES (?1,?2) ON CONFLICT(user) DO UPDATE SET policy=excluded.policy",
                params![user, serde_json::to_string(policy)?],
            )?;
        } else {
            self.conn().execute("DELETE FROM delivery_policies WHERE user = ?1", [user])?;
        }
        Ok(())
    }
    pub fn register_interaction_delivery(&self, outbox: i64, project: &str, reference: &str, version: &str, recipient: i64) -> Result<()> {
        self.conn().execute(
            "INSERT OR IGNORE INTO interaction_deliveries(outbox,project,reference,version,recipient) VALUES (?1,?2,?3,?4,?5)",
            params![outbox, project, reference, version, recipient],
        )?;
        Ok(())
    }
    pub fn open_interaction_deliveries(&self, project: &str) -> Result<Vec<InteractionDelivery>> {
        let mut stmt = self
            .conn()
            .prepare("SELECT * FROM interaction_deliveries WHERE project = ?1 AND closed=0 ORDER BY checked_at,outbox LIMIT 200")?;
        Ok(stmt
            .query_map([project], |r| {
                Ok(InteractionDelivery {
                    outbox: r.get("outbox")?,
                    project: r.get("project")?,
                    reference: r.get("reference")?,
                    version: r.get("version")?,
                    recipient: r.get("recipient")?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }
    pub fn checked_interaction_delivery(&self, outbox: i64) -> Result<()> {
        self.conn().execute("UPDATE interaction_deliveries SET checked_at=?2 WHERE outbox=?1", params![outbox, crate::db::now()])?;
        Ok(())
    }
    pub fn interaction_delivery(&self, outbox: i64) -> Result<Option<InteractionDelivery>> {
        Ok(self
            .conn()
            .query_row("SELECT * FROM interaction_deliveries WHERE outbox=?1", [outbox], |r| {
                Ok(InteractionDelivery {
                    outbox: r.get("outbox")?,
                    project: r.get("project")?,
                    reference: r.get("reference")?,
                    version: r.get("version")?,
                    recipient: r.get("recipient")?,
                })
            })
            .optional()?)
    }
    /// Queue a copy update even while the original send is in flight. The sync job waits for
    /// its external reference; an unsent copy is cancelled before a dispatcher can claim it.
    pub fn close_interaction_delivery(&self, outbox: i64, state: &str) -> Result<()> {
        self.tx(|| {
            let changed = self.conn().execute("UPDATE interaction_deliveries SET closed=1 WHERE outbox=?1 AND closed=0", [outbox])?;
            if changed == 0 {
                return Ok(());
            }
            self.conn().execute(
                "UPDATE outbox SET status='cancelled',last_error=?2 WHERE id=?1 AND status IN ('pending','failed')",
                params![outbox, format!("interaction {state}")],
            )?;
            let item = self.outbox_item(outbox)?;
            if item.channel == "telegram" {
                self.enqueue(
                    Some(&format!("interaction-sync:{outbox}")),
                    item.user,
                    "interaction_sync",
                    &item.address,
                    "Interaction resolved",
                    "",
                    &json!({"original":outbox,"state":state,"project":item.payload["project"],"recipient":item.user}),
                )?;
            }
            if let Some(notification) = item.payload["notification"].as_i64() {
                self.conn().execute(
                    "UPDATE notifications SET kind='interaction_closed',body=body||?2 WHERE id=?1 AND kind!='interaction_closed'",
                    params![notification, format!("\n\nСостояние: {state}. Решение уже сохранено либо обращение больше не актуально.")],
                )?;
            }
            Ok(())
        })
    }
}
