use std::path::PathBuf;

pub trait FileName {
    fn string_file_name(&self) -> &str;
}

impl FileName for PathBuf {
    fn string_file_name(&self) -> &str {
        self.file_name().and_then(|s| s.to_str()).unwrap_or("unknown")
    }
}
