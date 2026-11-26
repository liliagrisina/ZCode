use super::{
    optional_u64, required_string, resolve_inside, Tool, ToolContext, ToolError, ToolSpec,
};
use serde_json::{json, Value};
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};

pub struct ReadFile;

impl Tool for ReadFile {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "read_file",
            description: "Read a UTF-8 text file inside the workspace and return numbered lines. offset is 1-based. Use limit to read a window instead of the whole file.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Workspace-relative or absolute path inside the workspace"},
                    "offset": {"type": "integer", "minimum": 1, "description": "First line to return, 1-based"},
                    "limit": {"type": "integer", "minimum": 1, "description": "Maximum number of lines, default 200"}
                },
                "required": ["path"]
            }),
        }
    }

    fn invoke(&self, args: &Value, ctx: &ToolContext) -> Result<String, ToolError> {
        let raw = required_string(args, "path")?;
        let offset = optional_u64(args, "offset")?.unwrap_or(1);
        let limit = optional_u64(args, "limit")?.unwrap_or(200).clamp(1, 400);
        if offset == 0 {
            return Err(ToolError::Message("offset 从 1 开始".into()));
        }
        let path = resolve_inside(ctx.workspace, &raw)?;
        if !path.is_file() {
            return Err(ToolError::Message(format!("不是文件: {}", path.display())));
        }

        let mut file = File::open(&path)?;
        let mut probe = [0u8; 8192];
        let probed = file.read(&mut probe)?;
        if probe[..probed].contains(&0) {
            return Err(ToolError::Message("看起来是二进制文件，已拒绝读取".into()));
        }
        file.seek(SeekFrom::Start(0))?;

        let start = offset as usize;
        let end = start + limit as usize - 1;
        let mut shown = Vec::new();
        let mut total = 0usize;
        let mut truncated_scan = false;
        for (index, line) in BufReader::new(file).lines().enumerate() {
            let line = line?;
            let line_no = index + 1;
            total = line_no;
            if line_no >= start && line_no <= end {
                shown.push((line_no, line));
            }
            if line_no >= 200_000 {
                truncated_scan = true;
                break;
            }
        }
        if shown.is_empty() {
            return Ok(format!(
                "文件 {} 共 {total} 行，请求的范围 {start}-{end} 没有内容",
                path.display()
            ));
        }
        let width = shown.last().map(|(n, _)| n.to_string().len()).unwrap_or(1);
        let mut body = format!(
            "文件 {}，行 {start}-{}（文件至少 {total} 行）\n",
            path.display(),
            shown.last().map(|(n, _)| *n).unwrap_or(start)
        );
        for (line_no, line) in shown {
            body.push_str(&format!("{line_no:>width$}|{line}\n"));
        }
        if truncated_scan {
            body.push_str("扫描在 200000 行处停止。\n");
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
    fn reads_a_window_and_blocks_escape() {
        let scratch = Scratch::new("read");
        fs::write(scratch.path().join("note.txt"), "one\ntwo\nthree\n").unwrap();
        let tool = ReadFile;
        let ctx = ToolContext {
            workspace: scratch.path(),
        };
        let text = tool
            .invoke(&json!({"path": "note.txt", "offset": 2, "limit": 1}), &ctx)
            .unwrap();
        assert!(text.contains("2|two"));
        assert!(!text.contains("1|one"));
        assert!(tool.invoke(&json!({"path": "../secret"}), &ctx).is_err());
    }
}
