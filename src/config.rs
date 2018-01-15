use std::fs::File;
use std::io::Read;
use std::collections::HashMap;
use serde_json;
use failure::{Error};

#[derive(Serialize, Deserialize, Debug)]
pub struct Repo {
    pub url: String,
    pub env: Option<HashMap<String, String>>,
    pub options: Option<HashMap<String, String>>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct BackupSet {
    pub repos: Vec<Repo>,
    pub dirs: Vec<String>,
}

pub fn load(config_file: String) -> Result<Vec<BackupSet>,Error> {
    let mut file = File::open(config_file)?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;
    let config = serde_json::from_str(&contents)?;
    Ok( config )
}
