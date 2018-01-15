extern crate structopt;
#[macro_use] extern crate structopt_derive;
extern crate scoped_threadpool;
extern crate serde;
extern crate serde_json;
#[macro_use] extern crate serde_derive;
extern crate failure;

use failure::{Error, err_msg};
use structopt::StructOpt;
use std::process::Command;
use scoped_threadpool::Pool;
use std::collections::HashMap;

mod config;
use config::BackupSet;
use config::Repo;
use std::process::exit;


#[derive(StructOpt, Debug)]
#[structopt(name = "resticmgr", about = "My Restic manager.")]
struct Args {
    #[structopt(help = "Action")]
    action: String,
    #[structopt(help = "Config file (JSON)", default_value = "./config.json")]
    config_file: String,
}

fn main() {
    let args = Args::from_args();
    match config::load(args.config_file){
        Ok( conf ) => {
            match args.action.as_ref() {
                "backup" => backup_all(&conf),
                _ => eprintln!("Invalid command '{}'", args.action),
            }
        },
        Err( err ) => {
            eprintln!( "Failed to load config file: {}", err );
            exit(1);
        }
    }
}

fn backup_all(config: &[BackupSet]) {
    let mut thread_pool = Pool::new(4);
    let mut restic_results = HashMap::new();
    for set in config {
        for repo in &set.repos {
            thread_pool.scoped(|_scope| {
                restic_results.insert(&repo.url, backup_to_single_repo(repo, &set.dirs));
            });
        }
    }
    for (repo, output) in restic_results {
        println!("Repository '{}' results:", repo);
        match output {
            Ok(out) => println!("Succeeded:\n{}", out),
            Err(out) => {
                eprintln!("Failed:\n{}", out);
                // FIXME Send notification email on errors
            }
        }
    }
}

fn backup_to_single_repo(repo: &Repo, dirs: &[String]) -> Result<String, Error > {
    println!("Starting backup to repo '{}'...", &repo.url);

    let mut command = Command::new("restic");
    command.arg("backup")
        .arg("-r")
        .arg(&repo.url);

    // Set env vars
    if let Some(ref env) = repo.env {
        command.envs(env);
    }

    // Set options
    if let Some(ref options) = repo.options {
        for option in options {
            command.arg("-o");
            command.arg(format!("{}={}", option.0, option.1));
        }
    }

    // Set directories
    command.args(dirs.iter());

    // Run
    let output = command.output()?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(err_msg(String::from_utf8_lossy(&output.stderr).to_string()))
    }
}
