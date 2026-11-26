use std::fs;
use std::path::{Path, PathBuf};

pub struct Scratch(PathBuf);

impl Scratch {
    pub fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!("zcode-rs-{label}-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
