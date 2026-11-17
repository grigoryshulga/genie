//! The catalog as MCP tools: one tool per group (`genie_task`), its `action`
//! picks the operation and the other arguments are those of the operations,
//! merged. A person's agent and genie's own agents see the same tools, each
//! with only the actions its token may use.

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

use super::{Entry, GROUPS, catalog};

/// The tool of a group.
pub fn name(group: &str) -> String {
    format!("genie_{group}")
}

/// The operation a tool and an action name (`artifact_read` is `artifact-read`).
pub fn find(tool: &str, action: &str) -> Option<&'static Entry> {
    let group = tool.strip_prefix("genie_")?;
    let action = action.replace('_', "-");
    catalog().iter().find(|e| e.group == group && e.name == action)
}

/// The tools of the operations a caller may use, in the order of the groups.
/// `note` adds a remark to an action (the statuses a role may set); `extra` are
/// arguments every tool takes (the project, for a person).
pub fn tools(sees: &dyn Fn(&Entry) -> bool, note: &dyn Fn(&Entry) -> Option<String>, extra: &[(&str, Value)]) -> Vec<Value> {
    GROUPS
        .iter()
        .filter_map(|(group, about)| {
            let entries: Vec<&Entry> = catalog().iter().filter(|e| e.group == *group && sees(e)).collect();
            (!entries.is_empty()).then(|| tool(group, about, &entries, note, extra))
        })
        .collect()
}

fn tool(group: &str, about: &str, entries: &[&Entry], note: &dyn Fn(&Entry) -> Option<String>, extra: &[(&str, Value)]) -> Value {
    let mut uses: BTreeMap<String, Vec<(Value, &str)>> = BTreeMap::new();
    let mut order: Vec<String> = Vec::new();
    let mut defs = Map::new();
    let mut actions = Vec::new();
    for e in entries {
        if let Some(d) = e.schema.get("$defs").and_then(Value::as_object) {
            defs.extend(d.clone());
        }
        let required: Vec<&str> = e.schema["required"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
        let mut args = Vec::new();
        for k in arguments(e) {
            if !uses.contains_key(&k) {
                order.push(k.clone());
            }
            uses.entry(k.clone()).or_default().push((e.schema["properties"][&k].clone(), e.name));
            args.push(if required.contains(&k.as_str()) { k } else { format!("{k}?") });
        }
        let remark = note(e).map(|n| format!(" ({n})")).unwrap_or_default();
        actions.push(format!("- {}({}){remark}: {}", e.name, args.join(", "), e.summary()));
    }
    let mut props = Map::new();
    props.insert(
        "action".into(),
        json!({ "type": "string", "enum": entries.iter().map(|e| e.name).collect::<Vec<_>>(), "description": "The operation; its arguments are listed in the tool's description." }),
    );
    for k in order {
        props.insert(k.clone(), merge(&uses[&k], entries.len()));
    }
    for (k, v) in extra {
        props.insert(k.to_string(), v.clone());
    }
    let mut schema = json!({ "type": "object", "properties": props, "required": ["action"] });
    if !defs.is_empty() {
        schema["$defs"] = Value::Object(defs);
    }
    json!({
        "name": name(group),
        "description": format!("{about}. Actions (arguments with ? may be left out):\n{}", actions.join("\n")),
        "inputSchema": schema,
    })
}

/// The JSON arguments of an operation in the order of its command line (then any others).
fn arguments(e: &Entry) -> Vec<String> {
    let props = e.schema["properties"].as_object().cloned().unwrap_or_default();
    let camel = |id: &str| {
        let mut out = String::new();
        let mut up = false;
        for c in id.chars() {
            match c {
                '_' => up = true,
                c if up => {
                    out.extend(c.to_uppercase());
                    up = false;
                }
                c => out.push(c),
            }
        }
        out
    };
    let mut out: Vec<String> = Vec::new();
    for a in e.command(e.name).get_arguments() {
        let id = a.get_id().as_str();
        if let Some(k) = [id.to_string(), camel(id)].into_iter().find(|k| props.contains_key(k))
            && !out.contains(&k)
        {
            out.push(k);
        }
    }
    out.extend(props.keys().filter(|k| !out.contains(k)).cloned().collect::<Vec<_>>());
    out
}

/// One argument of several operations: its schema (all of them when they
/// differ) and what it means for each.
fn merge(uses: &[(Value, &str)], actions: usize) -> Value {
    let bare = |v: &Value| {
        let mut v = v.clone();
        if let Some(o) = v.as_object_mut() {
            o.remove("description");
            o.remove("default");
        }
        v
    };
    let mut kinds: Vec<Value> = Vec::new();
    for (v, _) in uses {
        let b = bare(v);
        if !kinds.contains(&b) {
            kinds.push(b);
        }
    }
    let mut out = if kinds.len() == 1 { kinds.swap_remove(0) } else { json!({ "anyOf": kinds }) };
    let mut meanings: Vec<(&str, Vec<&str>)> = Vec::new();
    for (v, action) in uses {
        let d = v["description"].as_str().unwrap_or_default();
        match meanings.iter_mut().find(|(m, _)| *m == d) {
            Some((_, a)) => a.push(action),
            None => meanings.push((d, vec![action])),
        }
    }
    // An action that says nothing about an argument means what another says.
    if meanings.len() > 1
        && let Some(i) = meanings.iter().position(|(d, _)| d.is_empty())
    {
        let (_, silent) = meanings.remove(i);
        meanings[0].1.extend(silent);
    }
    let text = if meanings.len() == 1 && uses.len() == actions {
        meanings[0].0.to_string()
    } else {
        meanings
            .iter()
            .map(|(d, a)| if d.is_empty() { format!("({})", a.join(", ")) } else { format!("{d} ({})", a.join(", ")) })
            .collect::<Vec<_>>()
            .join(" ")
    };
    if !text.is_empty() {
        out["description"] = json!(text);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::Need;

    #[test]
    fn a_group_is_one_tool_with_its_actions_and_their_arguments() {
        let all = tools(&|_| true, &|_| None, &[]);
        let task = all.iter().find(|t| t["name"] == "genie_task").unwrap();
        let actions: Vec<&str> =
            task["inputSchema"]["properties"]["action"]["enum"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
        assert!(actions.contains(&"show") && actions.contains(&"artifact-read"), "{actions:?}");
        let desc = task["description"].as_str().unwrap();
        assert!(desc.contains("- status(context?, status, task?, note?, force?, ownerAction?, options?, repo?): Move a task"), "{desc}");
        // One argument, several meanings: each is said with its actions.
        let text = task["inputSchema"]["properties"]["text"]["description"].as_str().unwrap_or_default();
        assert!(text.contains("(comment)") || text.contains("comment"), "{text}");
        assert_eq!(task["inputSchema"]["required"], json!(["action"]));
        assert!(find("genie_task", "artifact_read").is_some_and(|e| e.name == "artifact-read"));
        assert!(find("genie_nope", "show").is_none());

        // Only what the caller sees: no admin groups for a reader.
        let reader = tools(&|e| e.need == Need::Read, &|_| None, &[]);
        assert!(reader.iter().all(|t| t["name"] != "genie_user"), "user operations are for admins");
    }
}
