//! The deliberately small, text-only Chat Completions dialect admitted by the
//! monetary gateway. Extra paid modalities/routing need their own accounting.

use genie_core::usage::Tokens;
use serde_json::{Value, json};

pub(super) struct Request {
    pub body: Value,
    pub output_limit: u64,
    pub stream: bool,
}

impl Request {
    pub(super) fn parse(mut body: Value, model: &str, maximum: u64) -> Result<Self, String> {
        object_keys(
            &body,
            &[
                "model",
                "messages",
                "stream",
                "stream_options",
                "store",
                "max_tokens",
                "max_completion_tokens",
                "temperature",
                "top_p",
                "stop",
                "seed",
                "presence_penalty",
                "frequency_penalty",
                "tools",
                "tool_choice",
                "parallel_tool_calls",
                "reasoning_effort",
                "thinking",
                "n",
            ],
        )?;
        if body["model"] != model {
            return Err("the model does not match this worker's enrollment".into());
        }
        if body.get("n").is_some_and(|n| n.as_u64() != Some(1)) {
            return Err("bounded calls generate exactly one completion".into());
        }
        let field = match (body.get("max_tokens"), body.get("max_completion_tokens")) {
            (Some(_), None) => "max_tokens",
            (None, Some(_)) => "max_completion_tokens",
            _ => return Err("bounded calls need exactly one explicit output token cap".into()),
        };
        let output_limit = body[field].as_u64().filter(|n| *n > 0).ok_or("invalid output token cap")?.min(maximum);
        if output_limit == 0 {
            return Err("the enrolled model has no output allowance".into());
        }
        body[field] = json!(output_limit);
        let stream = match body.get("stream") {
            None => false,
            Some(v) => v.as_bool().ok_or("stream must be a boolean")?,
        };
        if body.get("store").is_some_and(|v| v != false) {
            return Err("bounded calls do not support provider-side storage".into());
        }
        if let Some(options) = body.get("stream_options") {
            object_keys(options, &["include_usage"])?;
        }
        if stream {
            body["stream_options"] = json!({"include_usage":true});
        }
        if let Some(thinking) = body.get("thinking") {
            object_keys(thinking, &["type"])?;
            if !matches!(thinking["type"].as_str(), Some("enabled" | "disabled")) {
                return Err("unsupported reasoning configuration".into());
            }
        }
        if let Some(effort) = body.get("reasoning_effort")
            && !matches!(effort.as_str(), Some("none" | "minimal" | "low" | "medium" | "high" | "xhigh"))
        {
            return Err("unsupported reasoning effort".into());
        }
        let messages = body["messages"].as_array().filter(|a| !a.is_empty()).ok_or("messages must be a nonempty array")?;
        for message in messages {
            object_keys(message, &["role", "content", "name", "tool_call_id", "tool_calls", "reasoning_content"])?;
            if !matches!(message["role"].as_str(), Some("system" | "developer" | "user" | "assistant" | "tool")) {
                return Err("unsupported message role".into());
            }
            match &message["content"] {
                Value::Null | Value::String(_) => {}
                Value::Array(parts) => {
                    for part in parts {
                        object_keys(part, &["type", "text"])?;
                        if part["type"] != "text" || !part["text"].is_string() {
                            return Err("bounded calls support text content only".into());
                        }
                    }
                }
                _ => return Err("unsupported message content".into()),
            }
            if message.get("reasoning_content").is_some_and(|v| !v.is_string()) {
                return Err("reasoning content must be text".into());
            }
            if let Some(calls) = message.get("tool_calls") {
                for call in calls.as_array().ok_or("tool_calls must be an array")? {
                    object_keys(call, &["id", "type", "function"])?;
                    function(&call["function"], &["name", "arguments"])?;
                    if call["type"] != "function" || !call["function"]["arguments"].is_string() {
                        return Err("only ordinary function calls are supported".into());
                    }
                }
            }
        }
        if let Some(tools) = body.get("tools") {
            for tool in tools.as_array().ok_or("tools must be an array")? {
                object_keys(tool, &["type", "function"])?;
                function(&tool["function"], &["name", "description", "parameters", "strict"])?;
                if tool["type"] != "function" {
                    return Err("paid provider tools are outside the bounded text profile".into());
                }
            }
        }
        if let Some(choice) = body.get("tool_choice") {
            if choice.is_object() {
                object_keys(choice, &["type", "function"])?;
                function(&choice["function"], &["name"])?;
                if choice["type"] != "function" {
                    return Err("unsupported tool choice".into());
                }
            } else if !matches!(choice.as_str(), Some("auto" | "none" | "required")) {
                return Err("unsupported tool choice".into());
            }
        }
        Ok(Self { body, output_limit, stream })
    }
}

fn object_keys(value: &Value, allowed: &[&str]) -> Result<(), String> {
    let object = value.as_object().ok_or("expected an object in bounded model request")?;
    if object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err("unsupported field in bounded text model request".into());
    }
    Ok(())
}
fn function(value: &Value, allowed: &[&str]) -> Result<(), String> {
    object_keys(value, allowed)?;
    if value["name"].as_str().is_none_or(str::is_empty) {
        return Err("function needs a name".into());
    }
    Ok(())
}

/// OpenAI completion_tokens includes reasoning. Cache hits/writes are parts of
/// prompt_tokens, never additional tokens on top of it (also DeepSeek's wire).
pub(super) fn usage(value: &Value) -> Result<Tokens, String> {
    let count = |v: &Value| v.as_u64().filter(|n| *n <= i64::MAX as u64).ok_or("invalid provider token count".to_string());
    let prompt = count(&value["prompt_tokens"])?;
    let output = count(&value["completion_tokens"])?;
    let mut hits = None;
    for raw in [&value["prompt_tokens_details"]["cached_tokens"], &value["prompt_cache_hit_tokens"], &value["cached_tokens"]] {
        if !raw.is_null() {
            let n = count(raw)?;
            if hits.is_some_and(|previous| previous != n) {
                return Err("conflicting provider cache usage".into());
            }
            hits = Some(n);
        }
    }
    let cache_read = hits.unwrap_or(0);
    let cache_write = match &value["prompt_tokens_details"]["cache_write_tokens"] {
        Value::Null => 0,
        n => count(n)?,
    };
    let input = prompt.checked_sub(cache_read).and_then(|n| n.checked_sub(cache_write)).ok_or("cache usage exceeds prompt usage")?;
    if let Some(total) = value.get("total_tokens")
        && count(total)? != prompt.checked_add(output).ok_or("provider token total overflows")?
    {
        return Err("inconsistent provider token total".into());
    }
    if let Some(miss) = value.get("prompt_cache_miss_tokens")
        && count(miss)? != prompt - cache_read
    {
        return Err("inconsistent provider cache miss usage".into());
    }
    if let Some(reasoning) = value["completion_tokens_details"].get("reasoning_tokens")
        && count(reasoning)? > output
    {
        return Err("reasoning usage exceeds completion usage".into());
    }
    // These features can introduce charges outside text token tariffs.
    for extra in ["audio_tokens", "accepted_prediction_tokens", "rejected_prediction_tokens"] {
        for details in ["prompt_tokens_details", "completion_tokens_details"] {
            if let Some(v) = value[details].get(extra)
                && count(v)? > 0
            {
                return Err("provider reported usage outside the bounded text profile".into());
            }
        }
    }
    Ok(Tokens { input, output, cache_read, cache_write })
}

/// Incremental SSE parser: raw bytes can be split inside UTF-8 and CRLF. Usage
/// is accepted only from a completed stream with a single consistent receipt.
#[derive(Default)]
pub(super) struct Stream {
    pending: Vec<u8>,
    frame: Vec<u8>,
    data: Vec<String>,
    tokens: Option<Tokens>,
    done: bool,
}
impl Stream {
    /// Return complete wire events except [DONE]. The caller releases that
    /// terminator only after the authoritative receipt commits.
    pub(super) fn push(&mut self, bytes: &[u8]) -> Result<Vec<u8>, String> {
        if self.pending.len().saturating_add(bytes.len()) > 2 * 1024 * 1024 {
            return Err("provider event exceeds the bounded wire limit".into());
        }
        self.pending.extend_from_slice(bytes);
        let mut forward = Vec::new();
        while let Some(end) = self.pending.iter().position(|c| *c == b'\n') {
            let raw = self.pending.drain(..=end).collect::<Vec<_>>();
            if self.frame.len().saturating_add(raw.len()) > 2 * 1024 * 1024 {
                return Err("provider event is too large".into());
            }
            self.frame.extend_from_slice(&raw);
            let line = std::str::from_utf8(&raw).map_err(|_| "invalid UTF-8 in provider stream")?.trim_end_matches(['\r', '\n']);
            if line.is_empty() {
                if self.event()? {
                    self.frame.clear();
                } else {
                    forward.append(&mut self.frame);
                }
            } else if let Some(data) = line.strip_prefix("data:") {
                if self.data.iter().map(String::len).sum::<usize>() + data.len() > 2 * 1024 * 1024 {
                    return Err("provider event is too large".into());
                }
                self.data.push(data.strip_prefix(' ').unwrap_or(data).to_string());
            }
        }
        Ok(forward)
    }
    fn event(&mut self) -> Result<bool, String> {
        if self.data.is_empty() {
            return Ok(false);
        }
        let data = std::mem::take(&mut self.data).join("\n");
        if self.done {
            return Err("provider sent data after the end of the stream".into());
        }
        if data.trim() == "[DONE]" {
            self.done = true;
            return Ok(true);
        }
        let value: Value = serde_json::from_str(&data).map_err(|_| "invalid JSON in provider stream")?;
        if value.get("error").is_some() {
            return Err("provider stream ended with an error".into());
        }
        if let Some(raw) = value.get("usage").filter(|v| !v.is_null()) {
            let tokens = usage(raw)?;
            if self.tokens.is_some_and(|old| old != tokens) {
                return Err("provider sent conflicting usage receipts".into());
            }
            self.tokens = Some(tokens);
        }
        Ok(false)
    }
    pub(super) fn finish(self) -> Result<Tokens, String> {
        if !self.done || !self.pending.iter().all(u8::is_ascii_whitespace) || !self.data.is_empty() {
            return Err("provider stream ended without a complete usage receipt".into());
        }
        self.tokens.ok_or("provider stream omitted usage; maximum exposure retained".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_tool_loops_keep_the_smaller_requested_output_cap() {
        let request = Request::parse(json!({"model":"model","messages":[{"role":"user","content":[{"type":"text","text":"continue"}]},{"role":"assistant","content":null,"reasoning_content":"reasoning","tool_calls":[{"id":"one","type":"function","function":{"name":"read","arguments":"{}"}}]},{"role":"tool","tool_call_id":"one","content":"result"}],"tools":[{"type":"function","function":{"name":"read","parameters":{"type":"object"}}}],"stream":true,"max_completion_tokens":3}), "model", 100).unwrap();
        assert_eq!(request.output_limit, 3);
        assert_eq!(request.body["max_completion_tokens"], 3);
        assert_eq!(request.body["stream_options"]["include_usage"], true);
        assert!(Request::parse(json!({"model":"model","messages":[],"max_tokens":1,"max_completion_tokens":1}), "model", 10).is_err());
    }

    #[test]
    fn usage_does_not_double_count_cache_hits_or_reasoning() {
        let tokens = usage(&json!({"prompt_tokens":100,"completion_tokens":20,"total_tokens":120,"prompt_cache_hit_tokens":30,"prompt_cache_miss_tokens":70,"completion_tokens_details":{"reasoning_tokens":15}})).unwrap();
        assert_eq!(tokens, Tokens { input: 70, output: 20, cache_read: 30, cache_write: 0 });
        for invalid in [
            json!({"prompt_tokens":1}),
            json!({"prompt_tokens":1,"completion_tokens":-1}),
            json!({"prompt_tokens":1,"completion_tokens":1,"total_tokens":1}),
            json!({"prompt_tokens":1,"completion_tokens":1,"prompt_cache_hit_tokens":2}),
            json!({"prompt_tokens":1,"completion_tokens":1,"completion_tokens_details":{"audio_tokens":1}}),
        ] {
            assert!(usage(&invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn sse_parser_handles_split_utf8_crlf_and_requires_a_complete_receipt() {
        let wire = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"Текст\"}}]}\r\n\r\n",
            "data: {\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":3,\"total_tokens\":13}}\r\n\r\n",
            "data: [DONE]\r\n\r\n"
        );
        for size in 1..wire.len() {
            let mut parser = Stream::default();
            let mut forwarded = Vec::new();
            for chunk in wire.as_bytes().chunks(size) {
                forwarded.extend(parser.push(chunk).unwrap());
            }
            assert!(!String::from_utf8(forwarded).unwrap().contains("[DONE]"), "success must wait for settlement");
            assert_eq!(parser.finish().unwrap(), Tokens { input: 10, output: 3, ..Default::default() });
        }
        for incomplete in ["data: [DONE]\n\n", "data: {\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":1}}\n\n"] {
            let mut parser = Stream::default();
            parser.push(incomplete.as_bytes()).unwrap();
            assert!(parser.finish().is_err());
        }
        let mut parser = Stream::default();
        assert!(parser.push(b"data: [DONE]\n\ndata: {}\n\n").is_err());
    }
}
