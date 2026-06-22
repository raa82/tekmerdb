use anyhow::Result;

pub fn read(path: &str) -> Result<String> {
    Ok(std::fs::read_to_string(path)?)
}
