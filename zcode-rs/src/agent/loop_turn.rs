use super::provider::{build_request, CompletionResponse, ModelClient};
use crate::session::{ChatMessage, Session, TokenUsage as SessionUsage, ToolCallRecord};
use crate::tools::{Tool, ToolContext};
use crate::truncate_chars;
use anyhow::Result;
use serde_json::Value;
use std::sync::Arc;

/// 跑一轮用户消息：请求模型，执行它声明的工具，再把结果送回去，直到模型不再调用工具。
/// `max_steps` 是模型往返次数上限，用来避免失控循环；它不能代替上下文压缩。
pub fn run_turn(
    client: &dyn ModelClient,
    session: &mut Session,
    user_text: &str,
    tools: &[Arc<dyn Tool>],
    max_steps: usize,
    mut on_step: impl FnMut(&Session),
) -> Result<()> {
    let max_steps = max_steps.clamp(1, 32);
    session.note_user_title(user_text);
    session.messages.push(ChatMessage::user(user_text));
    session.touch();
    on_step(session);

    for _ in 0..max_steps {
        let request = build_request(&session.model, &session.messages, tools, &session.workspace);
        let response = client.complete(&request)?;
        remember_usage(session, &response);
        if response.tool_calls.is_empty() {
            let content = if response.content.trim().is_empty() {
                "（模型没有返回文本）".to_string()
            } else {
                response.content
            };
            session.messages.push(ChatMessage::assistant(content));
            session.touch();
            on_step(session);
            return Ok(());
        }

        let mut assistant = ChatMessage::assistant(response.content);
        assistant.tool_calls = response
            .tool_calls
            .iter()
            .map(|call| ToolCallRecord {
                id: call.id.clone(),
                name: call.name.clone(),
                arguments: call.arguments.clone(),
            })
            .collect();
        session.messages.push(assistant);
        session.touch();
        on_step(session);

        for call in response.tool_calls {
            let output = execute_tool(&call.name, &call.arguments, tools, &session.workspace);
            session
                .messages
                .push(ChatMessage::tool(call.id, call.name, output));
            session.touch();
            on_step(session);
        }
    }

    session.messages.push(ChatMessage::assistant(format!(
        "已达到这一轮的模型往返上限（{max_steps}）。已完成的工具结果都保留在会话里，可以再发一条消息让代理继续。"
    )));
    session.touch();
    on_step(session);
    Ok(())
}

fn remember_usage(session: &mut Session, response: &CompletionResponse) {
    if let Some(usage) = &response.usage {
        session.last_usage = Some(SessionUsage {
            prompt_tokens: usage.prompt_tokens,
            completion_tokens: usage.completion_tokens,
        });
    }
}

fn execute_tool(name: &str, arguments: &str, tools: &[Arc<dyn Tool>], workspace: &str) -> String {
    let Some(tool) = tools.iter().find(|tool| tool.spec().name == name) else {
        return format!("error: 没有名为 {name} 的工具");
    };
    let args: Value = match serde_json::from_str(arguments) {
        Ok(value) => value,
        Err(err) => return format!("error: 工具参数不是 JSON: {err}"),
    };
    let ctx = ToolContext {
        workspace: std::path::Path::new(workspace),
    };
    let text = match tool.invoke(&args, &ctx) {
        Ok(text) => text,
        Err(err) => format!("error: {err}"),
    };
    truncate_chars(&text, 12_000)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extension::ExtensionRegistry;
    use crate::session::Role;
    use crate::testutil::Scratch;
    use anyhow::bail;
    use serde_json::json;
    use std::collections::VecDeque;
    use std::fs;
    use std::sync::Mutex;

    struct Scripted {
        responses: Mutex<VecDeque<CompletionResponse>>,
    }

    impl ModelClient for Scripted {
        fn complete(
            &self,
            request: &super::super::provider::CompletionRequest,
        ) -> Result<CompletionResponse> {
            let offered = request
                .tools
                .iter()
                .any(|tool| tool["function"]["name"] == "read_file");
            if !offered {
                bail!("request did not offer read_file");
            }
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| anyhow::anyhow!("没有更多脚本响应"))
        }
    }

    #[test]
    fn loop_executes_tool_then_answers() {
        let scratch = Scratch::new("loop");
        fs::write(scratch.path().join("note.txt"), "alpha-token\n").unwrap();
        let tools = ExtensionRegistry::with_builtin().tools();
        let scripted = Scripted {
            responses: Mutex::new(VecDeque::from([
                CompletionResponse {
                    content: String::new(),
                    tool_calls: vec![super::super::provider::ToolCall {
                        id: "call_1".into(),
                        name: "read_file".into(),
                        arguments: json!({"path": "note.txt"}).to_string(),
                    }],
                    finish_reason: Some("tool_calls".into()),
                    usage: None,
                },
                CompletionResponse {
                    content: "看到了 alpha-token".into(),
                    tool_calls: vec![],
                    finish_reason: Some("stop".into()),
                    usage: Some(super::super::provider::TokenUsage {
                        prompt_tokens: Some(10),
                        completion_tokens: Some(4),
                    }),
                },
            ])),
        };
        let mut session = Session::create(scratch.path(), "demo");
        let mut updates = 0;
        run_turn(&scripted, &mut session, "读 note.txt", &tools, 8, |_| {
            updates += 1;
        })
        .unwrap();
        assert!(updates >= 3);
        assert_eq!(session.messages[0].role, Role::User);
        assert_eq!(session.messages[1].tool_calls[0].name, "read_file");
        assert!(session.messages[2].content.contains("alpha-token"));
        assert_eq!(session.messages[3].content, "看到了 alpha-token");
        assert_eq!(session.last_usage.unwrap().completion_tokens, Some(4));
        assert_eq!(session.title, "读 note.txt");
    }
}
