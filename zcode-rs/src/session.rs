use crate::truncate_chars;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
    Tool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCallRecord {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_tokens: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCallRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl ChatMessage {
    pub fn user(content: impl Into<String>) -> Self {
        Self::plain(Role::User, content)
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self::plain(Role::Assistant, content)
    }

    pub fn tool(
        id: impl Into<String>,
        name: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        let name = name.into();
        Self {
            role: Role::Tool,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: Some(id.into()),
            name: Some(name),
        }
    }

    fn plain(role: Role, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: None,
            name: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub title: String,
    pub workspace: String,
    pub model: String,
    pub created_at: String,
    pub updated_at: String,
    pub messages: Vec<ChatMessage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_usage: Option<TokenUsage>,
}

impl Session {
    pub fn create(workspace: &Path, model: &str) -> Self {
        let now = timestamp();
        Self {
            id: Uuid::new_v4().to_string(),
            title: "新会话".to_string(),
            workspace: workspace.display().to_string(),
            model: model.to_string(),
            created_at: now.clone(),
            updated_at: now,
            messages: Vec::new(),
            last_usage: None,
        }
    }

    pub fn touch(&mut self) {
        self.updated_at = timestamp();
    }

    pub fn note_user_title(&mut self, text: &str) {
        if self.title != "新会话" {
            return;
        }
        let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
        let title = truncate_chars(&collapsed, 42);
        if !title.is_empty() {
            self.title = title;
        }
    }
}

#[derive(Clone, Debug)]
pub struct SessionStore {
    dir: PathBuf,
}

impl SessionStore {
    pub fn open(data_dir: &Path) -> Result<Self> {
        let dir = data_dir.join("sessions");
        fs::create_dir_all(&dir).with_context(|| format!("无法创建会话目录 {}", dir.display()))?;
        Ok(Self { dir })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn save(&self, session: &Session) -> Result<()> {
        if !safe_id(&session.id) {
            anyhow::bail!("非法会话 id");
        }
        let path = self.path_for(&session.id);
        let tmp = self.dir.join(format!(".{}.json.tmp", session.id));
        let data = serde_json::to_vec_pretty(session)?;
        fs::write(&tmp, data).with_context(|| format!("无法写入 {}", tmp.display()))?;
        fs::rename(&tmp, &path).with_context(|| format!("无法保存会话 {}", path.display()))?;
        Ok(())
    }

    pub fn load(&self, id: &str) -> Result<Session> {
        if !safe_id(id) {
            anyhow::bail!("非法会话 id");
        }
        let path = self.path_for(id);
        let text = fs::read_to_string(&path)
            .with_context(|| format!("无法读取会话 {}", path.display()))?;
        serde_json::from_str(&text).with_context(|| format!("会话文件损坏 {}", path.display()))
    }

    pub fn list(&self) -> Result<Vec<Session>> {
        let mut sessions = Vec::new();
        for entry in fs::read_dir(&self.dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            if let Ok(session) = serde_json::from_str::<Session>(&text) {
                sessions.push(session);
            }
        }
        sessions.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
        Ok(sessions)
    }

    fn path_for(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }
}

pub fn timestamp() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn safe_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 80 && id.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::Scratch;

    #[test]
    fn round_trip_and_skip_corrupt() {
        let scratch = Scratch::new("sessions");
        let store = SessionStore::open(scratch.path()).unwrap();
        let mut session = Session::create(Path::new("/work/demo"), "demo-model");
        session.note_user_title("  修复   登录  ");
        session.messages.push(ChatMessage::user("修复登录"));
        session.touch();
        store.save(&session).unwrap();

        fs::write(store.dir().join("broken.json"), b"{not-json").unwrap();
        fs::write(store.dir().join("note.txt"), b"ignore").unwrap();

        let listed = store.list().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].title, "修复 登录");
        assert_eq!(store.load(&session.id).unwrap().messages.len(), 1);
        assert!(store.load("../etc/passwd").is_err());
    }
}
