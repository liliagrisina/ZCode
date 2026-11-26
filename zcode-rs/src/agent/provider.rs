use crate::config::chat_completions_url;
use crate::session::{ChatMessage, Role, ToolCallRecord};
use crate::tools::{tool_json, Tool};
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenUsage {
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct CompletionRequest {
    pub model: String,
    pub messages: Vec<Value>,
    pub tools: Vec<Value>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletionResponse {
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
    pub finish_reason: Option<String>,
    pub usage: Option<TokenUsage>,
}

pub trait ModelClient {
    fn complete(&self, request: &CompletionRequest) -> Result<CompletionResponse>;
}

pub struct UreqClient {
    url: String,
    api_key: String,
    agent: ureq::Agent,
}

impl UreqClient {
    pub fn new(api_base: &str, api_key: &str) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(20))
            .timeout(Duration::from_secs(180))
            .build();
        Self {
            url: chat_completions_url(api_base),
            api_key: api_key.to_string(),
            agent,
        }
    }
}

impl ModelClient for UreqClient {
    fn complete(&self, request: &CompletionRequest) -> Result<CompletionResponse> {
        let body = json!({
            "model": request.model,
            "messages": request.messages,
            "tools": request.tools,
            "tool_choice": "auto",
        });
        let response = self
            .agent
            .post(&self.url)
            .set("Authorization", &format!("Bearer {}", self.api_key))
            .set("Content-Type", "application/json")
            .set("Accept", "application/json")
            .send_json(body);
        let value = match response {
            Ok(response) => response.into_json::<Value>().context("模型响应不是 JSON")?,
            Err(ureq::Error::Status(code, response)) => {
                let text = response.into_string().unwrap_or_default();
                if let Ok(parsed) = serde_json::from_str::<Value>(&text) {
                    if let Err(err) = parse_completion(&parsed) {
                        bail!("模型请求失败 HTTP {code}: {err}");
                    }
                }
                bail!(
                    "模型请求失败 HTTP {code}: {}",
                    crate::truncate_chars(&text, 500)
                );
            }
            Err(err) => return Err(err).context("模型请求没有得到响应"),
        };
        parse_completion(&value)
    }
}

pub fn build_request(
    model: &str,
    session_messages: &[ChatMessage],
    tools: &[Arc<dyn Tool>],
    workspace: &str,
) -> CompletionRequest {
    let mut messages = vec![json!({
        "role": "system",
        "content": system_prompt(workspace),
    })];
    messages.extend(session_messages.iter().cloned().map(message_json));
    let tools = tools.iter().map(|tool| tool_json(&tool.spec())).collect();
    CompletionRequest {
        model: model.to_string(),
        messages,
        tools,
    }
}

fn system_prompt(workspace: &str) -> String {
    format!(
        "You are zcode-rs, a local coding agent. The workspace is `{workspace}`.\n\
         Available tools: read_file, list_dir, edit_file, run_command.\n\
         Use tools to inspect and change the workspace. Do not claim a file changed or a command succeeded unless the tool result says so.\n\
         File paths must stay inside the workspace. Prefer workspace-relative paths.\n\
         edit_file replaces exactly one occurrence of old_string. Create a new file by setting old_string to an empty string.\n\
         run_command starts in the workspace directory and is not an operating-system sandbox.\n\
         When you are finished, answer in the user's language and summarize what you actually did."
    )
}

fn message_json(message: ChatMessage) -> Value {
    match message.role {
        Role::User => json!({"role": "user", "content": message.content}),
        Role::Tool => json!({
            "role": "tool",
            "tool_call_id": message.tool_call_id.unwrap_or_default(),
            "content": message.content,
        }),
        Role::Assistant => {
            let mut value = json!({
                "role": "assistant",
                "content": if message.content.is_empty() && !message.tool_calls.is_empty() {
                    Value::Null
                } else {
                    Value::String(message.content)
                }
            });
            if !message.tool_calls.is_empty() {
                value["tool_calls"] =
                    Value::Array(message.tool_calls.into_iter().map(tool_call_json).collect());
            }
            value
        }
    }
}

fn tool_call_json(call: ToolCallRecord) -> Value {
    json!({
        "id": call.id,
        "type": "function",
        "function": {
            "name": call.name,
            "arguments": call.arguments,
        }
    })
}

pub fn parse_completion(value: &Value) -> Result<CompletionResponse> {
    if let Some(error) = value.get("error") {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("未知错误");
        bail!("模型返回错误: {message}");
    }
    let choice = value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .context("响应里没有 choices")?;
    let message = choice.get("message").context("响应里没有 message")?;
    let content = match message.get("content") {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    };
    let mut tool_calls = parse_tool_calls(message.get("tool_calls"));
    if tool_calls.is_empty() {
        if let Some(function_call) = message.get("function_call") {
            let wrapped = json!([{
                "id": "function_call",
                "function": function_call,
            }]);
            tool_calls = parse_tool_calls(Some(&wrapped));
        }
    }
    let usage = value.get("usage").map(|usage| TokenUsage {
        prompt_tokens: number_u64(usage.get("prompt_tokens")),
        completion_tokens: number_u64(usage.get("completion_tokens")),
    });
    Ok(CompletionResponse {
        content,
        tool_calls,
        finish_reason: choice
            .get("finish_reason")
            .and_then(Value::as_str)
            .map(str::to_string),
        usage,
    })
}

fn parse_tool_calls(value: Option<&Value>) -> Vec<ToolCall> {
    let Some(calls) = value.and_then(Value::as_array) else {
        return Vec::new();
    };
    calls
        .iter()
        .filter_map(|call| {
            let function = call.get("function").unwrap_or(call);
            let name = function.get("name")?.as_str()?.to_string();
            if name.is_empty() {
                return None;
            }
            let arguments = match function.get("arguments") {
                Some(Value::String(text)) => text.clone(),
                Some(other) => other.to_string(),
                None => "{}".to_string(),
            };
            Some(ToolCall {
                id: call
                    .get("id")
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty())
                    .unwrap_or("call")
                    .to_string(),
                name,
                arguments,
            })
        })
        .collect()
}

fn number_u64(value: Option<&Value>) -> Option<u64> {
    let value = value?;
    value
        .as_u64()
        .or_else(|| value.as_f64().map(|number| number as u64))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn parses_tool_call_and_text_content() {
        let parsed = parse_completion(&json!({
            "choices": [{
                "finish_reason": "tool_calls",
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": {"name": "read_file", "arguments": {"path": "a.txt"}}
                    }]
                }
            }],
            "usage": {"prompt_tokens": 3, "completion_tokens": 1.0}
        }))
        .unwrap();
        assert_eq!(parsed.tool_calls[0].name, "read_file");
        assert!(parsed.tool_calls[0].arguments.contains("a.txt"));
        assert_eq!(parsed.usage.unwrap().prompt_tokens, Some(3));
    }

    #[test]
    fn http_client_reads_completion_and_hides_key() {
        let body = r#"{"choices":[{"finish_reason":"stop","message":{"role":"assistant","content":"pong"}}]}"#;
        let (url, request) = spawn_server(200, body);
        let client = UreqClient::new(&url, "test-key");
        let response = client
            .complete(&CompletionRequest {
                model: "demo".into(),
                messages: vec![json!({"role": "user", "content": "ping"})],
                tools: vec![],
            })
            .unwrap();
        assert_eq!(response.content, "pong");
        let seen = request.recv_timeout(Duration::from_secs(3)).unwrap();
        let redacted = seen.replace("test-key", "***");
        assert!(seen.contains("chat/completions"), "request was: {redacted}");
        assert!(seen.contains("Bearer test-key"), "request was: {redacted}");
        assert!(seen.contains("demo"), "request was: {redacted}");

        let (url, _) = spawn_server(401, r#"{"error":{"message":"bad key"}}"#);
        let client = UreqClient::new(&url, "test-key");
        let err = client
            .complete(&CompletionRequest {
                model: "demo".into(),
                messages: vec![],
                tools: vec![],
            })
            .unwrap_err()
            .to_string();
        assert!(err.contains("bad key") || err.contains("401"));
        assert!(!err.contains("test-key"));
    }

    fn spawn_server(status: u16, body: &str) -> (String, std::sync::mpsc::Receiver<String>) {
        let body = body.to_string();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut buf = Vec::new();
            let mut tmp = [0u8; 2048];
            let started = std::time::Instant::now();
            while started.elapsed() < Duration::from_secs(3) {
                match stream.read(&mut tmp) {
                    Ok(0) => break,
                    Ok(size) => {
                        buf.extend_from_slice(&tmp[..size]);
                        if let Some(header_end) = find_header_end(&buf) {
                            let header = String::from_utf8_lossy(&buf[..header_end]).to_string();
                            let length = content_length(&header).unwrap_or(0);
                            while buf.len() < header_end + length
                                && started.elapsed() < Duration::from_secs(3)
                            {
                                match stream.read(&mut tmp) {
                                    Ok(0) | Err(_) => break,
                                    Ok(size) => buf.extend_from_slice(&tmp[..size]),
                                }
                            }
                            break;
                        }
                    }
                    Err(err)
                        if err.kind() == std::io::ErrorKind::WouldBlock
                            || err.kind() == std::io::ErrorKind::TimedOut =>
                    {
                        if find_header_end(&buf).is_some() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            let _ = tx.send(String::from_utf8_lossy(&buf).into_owned());
            let response = format!(
                "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        });
        (format!("http://{addr}/v1"), rx)
    }

    fn find_header_end(buf: &[u8]) -> Option<usize> {
        buf.windows(4)
            .position(|window| window == b"\r\n\r\n")
            .map(|index| index + 4)
    }

    fn content_length(header: &str) -> Option<usize> {
        header.lines().find_map(|line| {
            let (name, value) = line.split_once(':')?;
            if name.eq_ignore_ascii_case("content-length") {
                value.trim().parse().ok()
            } else {
                None
            }
        })
    }
}
