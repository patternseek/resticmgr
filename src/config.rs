use std::fs::File;
use std::io::Read;
use std::collections::HashMap;
use serde_json;
use failure::{Error, err_msg};

#[derive(Serialize, Deserialize, Debug)]
pub struct Config {
    pub repos: HashMap<String, Repo>,
    pub backupsets: Vec<BackupSet>,
    pub smtpnotify: SmtpNotificationConfig,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Repo {
    pub url: String,
    pub env: Option<HashMap<String, String>>,
    pub options: Option<HashMap<String, String>>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct BackupSet {
    pub reponames: Vec<String>,
    pub dirs: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct SmtpNotificationConfig {
    pub server: String,
    pub port: isize,
    pub username: String,
    pub password: String,
    pub from: String,
    pub to: String,
}

impl BackupSet {
    pub fn repos<'a>(&'a self, config: &'a Config) -> Result<Vec<&'a Repo>, Error> {
        let mut repos: Vec<&'a Repo> = vec![];
        for name in &self.reponames {
            if let Some(ref mut repo) = config.repos.get(name) {
                repos.push(repo);
            } else {
                return Err(
                    err_msg(
                        format!("Backupset config refers to a repo named {}, but none was found in repos config.", name)
                    )
                );
            }
        }
        Ok(repos)
    }
}

pub fn load(config_file: String) -> Result<Config, Error> {
    let mut file = File::open(config_file)?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;
    let config = serde_json::from_str(&contents)?;
    Ok(config)
}
