use super::ToolError;
use std::path::{Component, Path, PathBuf};

/// 把用户给出的路径解析到工作区内部。
/// 已存在的符号链接会按真实路径再检查一次，避免链到工作区外面。
pub fn resolve_inside(workspace: &Path, raw: &str) -> Result<PathBuf, ToolError> {
    let workspace = if workspace.exists() {
        workspace.canonicalize()?
    } else {
        lexical_normalize(workspace)
    };
    if workspace.as_os_str().is_empty() {
        return Err(ToolError::Message("工作区路径为空".into()));
    }
    let requested = Path::new(raw);
    let joined = if raw.trim().is_empty() {
        workspace.clone()
    } else if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        workspace.join(requested)
    };
    let normalized = lexical_normalize(&joined);
    if !is_inside(&workspace, &normalized) {
        return Err(ToolError::OutsideWorkspace(raw.to_string()));
    }

    if normalized.exists() {
        let canonical = normalized.canonicalize()?;
        let workspace_canonical = workspace.canonicalize().unwrap_or(workspace);
        if !is_inside(&workspace_canonical, &canonical) {
            return Err(ToolError::OutsideWorkspace(raw.to_string()));
        }
        return Ok(canonical);
    }

    if let Some(parent) = normalized.parent() {
        if parent.exists() {
            let parent_canonical = parent.canonicalize()?;
            let workspace_canonical = workspace
                .canonicalize()
                .unwrap_or_else(|_| workspace.clone());
            if !is_inside(&workspace_canonical, &parent_canonical) {
                return Err(ToolError::OutsideWorkspace(raw.to_string()));
            }
            if let Some(name) = normalized.file_name() {
                return Ok(parent_canonical.join(name));
            }
        }
    }

    Ok(normalized)
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn is_inside(root: &Path, path: &Path) -> bool {
    path.starts_with(root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::Scratch;
    use std::fs;

    #[test]
    fn accepts_relative_and_rejects_escape() {
        let scratch = Scratch::new("path");
        let nested = scratch.path().join("src");
        fs::create_dir_all(&nested).unwrap();
        let file = nested.join("main.rs");
        fs::write(&file, b"fn main() {}\n").unwrap();

        let resolved = resolve_inside(scratch.path(), "src/main.rs").unwrap();
        assert_eq!(resolved, file.canonicalize().unwrap());
        assert!(resolve_inside(scratch.path(), "../outside").is_err());
        assert!(resolve_inside(scratch.path(), "/etc/passwd").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_that_leaves_workspace() {
        let scratch = Scratch::new("symlink");
        let outside = Scratch::new("outside");
        let secret = outside.path().join("secret.txt");
        fs::write(&secret, b"nope").unwrap();
        std::os::unix::fs::symlink(&secret, scratch.path().join("link")).unwrap();
        assert!(resolve_inside(scratch.path(), "link").is_err());
    }
}
