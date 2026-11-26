mod edit_file;
mod list_dir;
mod path;
mod read_file;
mod run_command;

pub use edit_file::EditFile;
pub use list_dir::ListDir;
pub use path::resolve_inside;
pub use read_file::ReadFile;
pub use run_command::RunCommand;

use serde_json::Value;
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ToolError {
    #[error("路径超出工作区: {0}")]
    OutsideWorkspace(String),
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub struct ToolContext<'a> {
    pub workspace: &'a Path,
}

pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub parameters: Value,
}

pub trait Tool: Send + Sync {
    fn spec(&self) -> ToolSpec;
    fn invoke(&self, args: &Value, ctx: &ToolContext) -> Result<String, ToolError>;
}

pub fn tool_json(spec: &ToolSpec) -> Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": spec.name,
            "description": spec.description,
            "parameters": spec.parameters,
        }
    })
}

fn required_string(args: &Value, key: &str) -> Result<String, ToolError> {
    match args.get(key) {
        Some(Value::String(value)) => Ok(value.clone()),
        _ => Err(ToolError::Message(format!("缺少字符串参数 {key}"))),
    }
}

fn optional_u64(args: &Value, key: &str) -> Result<Option<u64>, ToolError> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(number)) => number
            .as_u64()
            .map(Some)
            .ok_or_else(|| ToolError::Message(format!("{key} 必须是非负整数"))),
        Some(_) => Err(ToolError::Message(format!("{key} 必须是非负整数"))),
    }
}
