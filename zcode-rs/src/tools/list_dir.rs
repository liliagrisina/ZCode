use super::{resolve_inside, Tool, ToolContext, ToolError, ToolSpec};
use serde_json::{json, Value};
use std::fs;

pub struct ListDir;

impl Tool for ListDir {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "list_dir",
            description:
                "List a single directory level inside the workspace. Directory names end with /.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Directory path. Defaults to the workspace root."}
                },
                "required": []
            }),
        }
    }

    fn invoke(&self, args: &Value, ctx: &ToolContext) -> Result<String, ToolError> {
        let raw = match args.get("path") {
            Some(Value::String(path)) if !path.trim().is_empty() => path.clone(),
            _ => ".".to_string(),
        };
        let path = resolve_inside(ctx.workspace, &raw)?;
        if !path.is_dir() {
            return Err(ToolError::Message(format!("不是目录: {}", path.display())));
        }
        let mut entries = Vec::new();
        for entry in fs::read_dir(&path)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let is_dir = entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false);
            entries.push((name, is_dir));
        }
        entries.sort_by(|left, right| left.0.cmp(&right.0));
        let total = entries.len();
        let shown = total.min(500);
        let mut body = format!("目录 {}（{total} 项）\n", path.display());
        for (name, is_dir) in entries.into_iter().take(shown) {
            if is_dir {
                body.push_str(&name);
                body.push_str("/\n");
            } else {
                body.push_str(&name);
                body.push('\n');
            }
        }
        if total > shown {
            body.push_str(&format!("… 另有 {} 项未列出\n", total - shown));
        }
        Ok(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::Scratch;
    use serde_json::json;
    use std::fs;

    #[test]
    fn lists_one_level() {
        let scratch = Scratch::new("list");
        fs::create_dir(scratch.path().join("src")).unwrap();
        fs::write(scratch.path().join("Cargo.toml"), b"[package]\n").unwrap();
        let text = ListDir
            .invoke(
                &json!({}),
                &ToolContext {
                    workspace: scratch.path(),
                },
            )
            .unwrap();
        assert!(text.contains("src/"));
        assert!(text.contains("Cargo.toml"));
    }
}
