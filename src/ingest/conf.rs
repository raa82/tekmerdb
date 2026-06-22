use serde::Deserialize;

#[derive(Deserialize, Default)]
pub struct ServerConf {
    pub domain: Option<String>,
    pub engine_host: Option<String>,
    pub engine_port: Option<u16>,
}

impl ServerConf {
    pub fn engine_url(&self) -> Option<String> {
        if self.engine_host.is_none() && self.engine_port.is_none() {
            return None;
        }
        let host = self.engine_host.as_deref().unwrap_or("127.0.0.1");
        let port = self.engine_port.unwrap_or(3000);
        Some(format!("http://{}:{}", host, port))
    }
}

pub fn find_and_load(explicit: Option<&str>) -> Option<(ServerConf, String)> {
    let home = std::env::var("HOME").unwrap_or_default();
    let candidates: Vec<String> = if let Some(p) = explicit {
        vec![p.to_string()]
    } else {
        vec![
            "tekmerdb-server.conf".to_string(),
            format!("{}/.config/tekmerdb/tekmerdb-server.conf", home),
            "/etc/tekmerdb/tekmerdb-server.conf".to_string(),
        ]
    };

    for path in &candidates {
        if let Ok(raw) = std::fs::read_to_string(path) {
            if let Ok(conf) = toml::from_str::<ServerConf>(&raw) {
                return Some((conf, path.clone()));
            }
        }
    }
    None
}
