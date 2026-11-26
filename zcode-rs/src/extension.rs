use crate::tools::{EditFile, ListDir, ReadFile, RunCommand, Tool};
use std::sync::Arc;

/// 扩展点。一个扩展可以提供零个或多个工具。
/// 当前只支持进程内注册，没有插件市场、MCP 或 Hook。
pub trait Extension: Send + Sync {
    fn id(&self) -> &str;
    fn tools(&self) -> Vec<Arc<dyn Tool>>;
}

pub struct BuiltinExtension;

impl Extension for BuiltinExtension {
    fn id(&self) -> &str {
        "builtin"
    }

    fn tools(&self) -> Vec<Arc<dyn Tool>> {
        vec![
            Arc::new(ReadFile),
            Arc::new(ListDir),
            Arc::new(EditFile),
            Arc::new(RunCommand),
        ]
    }
}

#[derive(Default)]
pub struct ExtensionRegistry {
    extensions: Vec<Box<dyn Extension>>,
}

impl ExtensionRegistry {
    pub fn with_builtin() -> Self {
        let mut registry = Self::default();
        registry.register(Box::new(BuiltinExtension));
        registry
    }

    pub fn register(&mut self, extension: Box<dyn Extension>) {
        self.extensions.push(extension);
    }

    pub fn extension_ids(&self) -> Vec<String> {
        self.extensions
            .iter()
            .map(|extension| extension.id().to_string())
            .collect()
    }

    /// 同名工具以后注册的为准，便于测试或本地扩展替换内置实现。
    pub fn tools(&self) -> Vec<Arc<dyn Tool>> {
        let mut ordered: Vec<(String, Arc<dyn Tool>)> = Vec::new();
        for extension in &self.extensions {
            for tool in extension.tools() {
                let name = tool.spec().name.to_string();
                if let Some(index) = ordered.iter().position(|(existing, _)| existing == &name) {
                    ordered[index] = (name, tool);
                } else {
                    ordered.push((name, tool));
                }
            }
        }
        ordered.into_iter().map(|(_, tool)| tool).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::{ToolContext, ToolError, ToolSpec};
    use serde_json::{json, Value};

    struct OverrideRead;

    impl Tool for OverrideRead {
        fn spec(&self) -> ToolSpec {
            ToolSpec {
                name: "read_file",
                description: "test override",
                parameters: json!({"type": "object", "properties": {}}),
            }
        }

        fn invoke(&self, _args: &Value, _ctx: &ToolContext) -> Result<String, ToolError> {
            Ok("override".into())
        }
    }

    struct ExtraExtension;

    impl Extension for ExtraExtension {
        fn id(&self) -> &str {
            "extra"
        }

        fn tools(&self) -> Vec<Arc<dyn Tool>> {
            vec![Arc::new(OverrideRead)]
        }
    }

    #[test]
    fn builtin_tools_and_override() {
        let registry = ExtensionRegistry::with_builtin();
        let names: Vec<_> = registry
            .tools()
            .iter()
            .map(|tool| tool.spec().name.to_string())
            .collect();
        assert_eq!(
            names,
            vec!["read_file", "list_dir", "edit_file", "run_command"]
        );

        let mut registry = ExtensionRegistry::with_builtin();
        registry.register(Box::new(ExtraExtension));
        assert_eq!(
            registry.extension_ids(),
            vec!["builtin".to_string(), "extra".to_string()]
        );
        let read = registry
            .tools()
            .into_iter()
            .find(|tool| tool.spec().name == "read_file")
            .unwrap();
        let ctx = ToolContext {
            workspace: std::path::Path::new("."),
        };
        assert_eq!(read.invoke(&json!({}), &ctx).unwrap(), "override");
    }
}
