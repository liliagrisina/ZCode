use super::{required_string, resolve_inside, Tool, ToolContext, ToolError, ToolSpec};
use serde_json::{json, Value};
use std::fs;

pub struct EditFile;

impl Tool for EditFile {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "edit_file",
            description: "Replace exactly one occurrence of old_string with new_string in a workspace text file. To create a new file, use an empty old_string. Fails when old_string matches zero or multiple times.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "old_string": {"type": "string"},
                    "new_string": {"type": "string"}
                },
                "required": ["path", "old_string", "new_string"]
            }),
        }
    }

    fn invoke(&self, args: &Value, ctx: &ToolContext) -> Result<String, ToolError> {
        let raw = required_string(args, "path")?;
        let old = required_string(args, "old_string")?;
        let new = required_string(args, "new_string")?;
        let path = resolve_inside(ctx.workspace, &raw)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        if old.is_empty() {
            if path.exists() {
                return Err(ToolError::Message(
                    "文件已存在。空 old_string 只用于创建新文件；修改时请给出要替换的原文。".into(),
                ));
            }
            write_atomic(&path, &new)?;
            return Ok(format!("已创建 {}（{} 字节）", path.display(), new.len()));
        }

        if !path.is_file() {
            return Err(ToolError::Message(
                "文件不存在。创建新文件时 old_string 必须为空字符串。".into(),
            ));
        }
        let original = fs::read_to_string(&path)?;
        let count = original.matches(&old).count();
        if count == 0 {
            return Err(ToolError::Message(
                "没有找到与 old_string 完全一致的文本。请包含足够的上下文，并保持空格和换行一致。"
                    .into(),
            ));
        }
        if count > 1 {
            return Err(ToolError::Message(format!(
                "old_string 匹配了 {count} 处。请提供更长的上下文，使它只匹配一处。"
            )));
        }
        let updated = original.replacen(&old, &new, 1);
        write_atomic(&path, &updated)?;
        Ok(format!("已更新 {}（替换 1 处）", path.display()))
    }
}

fn write_atomic(path: &std::path::Path, contents: &str) -> Result<(), ToolError> {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".into());
    let tmp = path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join(format!(".{name}.zcode-tmp"));
    fs::write(&tmp, contents)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::Scratch;
    use serde_json::json;
    use std::fs;

    #[test]
    fn creates_and_replaces_once() {
        let scratch = Scratch::new("edit");
        let ctx = ToolContext {
            workspace: scratch.path(),
        };
        EditFile
            .invoke(
                &json!({"path": "src/a.txt", "old_string": "", "new_string": "alpha beta alpha"}),
                &ctx,
            )
            .unwrap();
        let err = EditFile
            .invoke(
                &json!({"path": "src/a.txt", "old_string": "alpha", "new_string": "ALPHA"}),
                &ctx,
            )
            .unwrap_err();
        assert!(err.to_string().contains("2"));
        EditFile
            .invoke(
                &json!({"path": "src/a.txt", "old_string": "alpha beta", "new_string": "ALPHA beta"}),
                &ctx,
            )
            .unwrap();
        let text = fs::read_to_string(scratch.path().join("src/a.txt")).unwrap();
        assert_eq!(text, "ALPHA beta alpha");
    }
}
