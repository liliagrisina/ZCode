use super::{optional_u64, required_string, Tool, ToolContext, ToolError, ToolSpec};
use crate::truncate_chars;
use serde_json::{json, Value};
use std::io::Read;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const OUTPUT_CHARS: usize = 12_000;

pub struct RunCommand;

impl Tool for RunCommand {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "run_command",
            description: "Run a shell command with its working directory set to the workspace. Returns the exit code plus truncated stdout and stderr. This is not an OS sandbox: a command can still affect the machine outside the workspace.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "command": {"type": "string"},
                    "timeout_sec": {"type": "integer", "minimum": 1, "description": "Timeout in seconds, default 30, maximum 120"}
                },
                "required": ["command"]
            }),
        }
    }

    fn invoke(&self, args: &Value, ctx: &ToolContext) -> Result<String, ToolError> {
        let command = required_string(args, "command")?;
        if command.trim().is_empty() {
            return Err(ToolError::Message("command 不能为空".into()));
        }
        if command.len() > 16_000 {
            return Err(ToolError::Message("command 过长".into()));
        }
        let timeout_sec = optional_u64(args, "timeout_sec")?
            .unwrap_or(30)
            .clamp(1, 120);
        let workspace = ctx
            .workspace
            .canonicalize()
            .map_err(|err| ToolError::Message(format!("无法进入工作区: {err}")))?;
        if !workspace.is_dir() {
            return Err(ToolError::Message("工作区不是目录".into()));
        }
        run_shell(&command, &workspace, Duration::from_secs(timeout_sec))
    }
}

fn run_shell(command: &str, cwd: &std::path::Path, timeout: Duration) -> Result<String, ToolError> {
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_remove("ZCODE_API_KEY")
        .env_remove("OPENAI_API_KEY")
        .env_remove("GH_TOKEN")
        .env_remove("GITHUB_TOKEN")
        .spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| ToolError::Message("无法读取命令标准输出".into()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| ToolError::Message("无法读取命令标准错误".into()))?;
    let stdout_task = thread::spawn(move || read_capped(stdout));
    let stderr_task = thread::spawn(move || read_capped(stderr));
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            let stdout = stdout_task.join().unwrap_or_default();
            let stderr = stderr_task.join().unwrap_or_default();
            return Err(ToolError::Message(format!(
                "命令超时（{} 秒）并已终止\n{}",
                timeout.as_secs(),
                format_output(None, &stdout, &stderr)
            )));
        }
        thread::sleep(Duration::from_millis(15));
    };
    let stdout = stdout_task.join().unwrap_or_default();
    let stderr = stderr_task.join().unwrap_or_default();
    Ok(format_output(status.code(), &stdout, &stderr))
}

fn format_output(code: Option<i32>, stdout: &str, stderr: &str) -> String {
    format!(
        "exit: {}\n--- stdout ---\n{}\n--- stderr ---\n{}",
        code.map(|code| code.to_string())
            .unwrap_or_else(|| "unknown".into()),
        truncate_chars(stdout, OUTPUT_CHARS),
        truncate_chars(stderr, OUTPUT_CHARS)
    )
}

fn read_capped(mut reader: impl Read) -> String {
    let mut stored = Vec::new();
    let mut tmp = [0u8; 4096];
    let mut truncated = false;
    let byte_cap = OUTPUT_CHARS * 4;
    loop {
        match reader.read(&mut tmp) {
            Ok(0) => break,
            Ok(size) => {
                if stored.len() < byte_cap {
                    let take = size.min(byte_cap - stored.len());
                    stored.extend_from_slice(&tmp[..take]);
                    if take < size {
                        truncated = true;
                    }
                } else {
                    truncated = true;
                }
            }
            Err(_) => break,
        }
    }
    let mut text = String::from_utf8_lossy(&stored).into_owned();
    if truncated {
        text.push_str("\n…[truncated]");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::Scratch;
    use serde_json::json;

    #[test]
    fn runs_in_workspace_and_times_out() {
        let scratch = Scratch::new("cmd");
        let ctx = ToolContext {
            workspace: scratch.path(),
        };
        let output = RunCommand
            .invoke(&json!({"command": "pwd && echo hello"}), &ctx)
            .unwrap();
        assert!(output.contains("exit: 0"));
        assert!(output.contains("hello"));
        assert!(output.contains(&scratch.path().canonicalize().unwrap().display().to_string()));

        let err = RunCommand
            .invoke(&json!({"command": "sleep 5", "timeout_sec": 1}), &ctx)
            .unwrap_err();
        assert!(err.to_string().contains("超时"));
    }
}
