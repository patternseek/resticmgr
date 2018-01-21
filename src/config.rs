use std::fs::File;
use std::io::Read;
use std::collections::HashMap;
use serde_json;
use failure::{Error};

#[derive(Serialize, Deserialize, Debug)]
pub struct Config {
    pub repos: HashMap<String, Repo>,
    pub backupsets: Vec<BackupSet>,
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

impl BackupSet{
    pub fn repos<'a>( &'a self, config: &'a Config ) -> Vec<&'a Repo>{
        let mut repos: Vec<&'a Repo> = vec!();
        for name in &self.reponames{
            if let Some( ref mut repo ) = config.repos.get( name ){
                repos.push(repo );
            }
        }
        repos
    }
}

pub fn load(config_file: String) -> Result<Config,Error> {
    let mut file = File::open(config_file)?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;
    let config = serde_json::from_str(&contents)?;
    Ok( config )
}
