extern crate structopt;
#[macro_use] extern crate structopt_derive;
extern crate scoped_threadpool;
extern crate serde;
extern crate serde_json;
#[macro_use] extern crate serde_derive;
extern crate failure;
extern crate chrono;

use failure::{Error, err_msg};
use structopt::StructOpt;
use std::process::Command;
use scoped_threadpool::Pool;
use std::collections::HashMap;
use chrono::{DateTime, Utc};
use std::process::exit;

mod config;
use config::Repo;
use config::Config;


#[derive(StructOpt, Debug)]
#[structopt(name = "resticmgr", about = "My Restic manager.")]
struct Args {
    #[structopt(help = "Action")]
    action: String,
    #[structopt(help = "Config file (JSON)", default_value = "./config.json")]
    config_file: String,
}
#[derive(Serialize, Deserialize, Debug)]
struct SnapshotInfo{
    time:  DateTime<Utc>,
    parent: Option<String>,
    tree: String,
    paths: Vec<String>,
    hostname: String,
    username: String,
    uid: isize,
    gid: isize,
    id: String,
    short_id: String,

}

fn main() {
    let args = Args::from_args();
    match config::load(args.config_file){
        Ok( conf ) => {
            match args.action.as_ref() {
                "backup" => backup_all(&conf),
                "verify" => check_last_snapshots_for_all( &conf ),
                _ => eprintln!("Invalid command '{}'", args.action),
            }
        },
        Err( err ) => {
            eprintln!( "Failed to load config file: {}", err );
            exit(1);
        }
    }
}

fn check_last_snapshots_for_all(config: &Config ){
    let mut thread_pool = Pool::new(4);
    let mut restic_results = HashMap::new();
    for (_,repo) in &config.repos {
        thread_pool.scoped(|_scope| {
            restic_results.insert(&repo.url, check_last_snapshot(repo));
        });
    }
    for (repo, output) in restic_results {
        println!("Repository '{}' results:", repo);
        match output {
            Ok(out) => println!("{}", out),
            Err(out) => {
                eprintln!("Failed:\n{}", out);
                // FIXME Send notification email on errors
            }
        }
    }
}

fn check_last_snapshot(repo: &Repo) -> Result<String, Error > {
    println!("Checking snapshots for repo '{}'...", &repo.url);

    let mut command = Command::new("restic");
    command
        .arg("snapshots")
        .arg("--last")
        .arg("--json");

    setup_restic_standard_options( &repo, &mut command);

    // Run
    let output = command.output()?;
    if output.status.success() {
        let res = &*String::from_utf8_lossy(&output.stdout);
        match serde_json::from_str(res) as Result<Vec<SnapshotInfo>, serde_json::Error> {
            Ok(ref mut snapshots) => {
                if let Some(ref snapshot) = snapshots.pop() {
                    let duration_ago = Utc::now().signed_duration_since(snapshot.time);
                    Ok(format!("Last backed up {} days, {} hours, {} mins ago. Paths: {}", duration_ago.num_days(), duration_ago.num_hours(), duration_ago.num_minutes(), snapshot.paths.join(", ")))
                } else {
                    Err( err_msg( "No snapshots found" ) )
                }
            },
            Err( err ) => {
                Err( err_msg( format!( "Couldn't parse response from restic: {}", err ) ) )
            }
        }
    } else {
        Err(err_msg(String::from_utf8_lossy(&output.stderr).to_string()))
    }
}

fn backup_all(config: &Config) {
    let mut thread_pool = Pool::new(4);
    let mut restic_results = HashMap::new();
    for set in &config.backupsets {
        for repo in set.repos( config ) {
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
    println!("Verifying repo '{}'...", &repo.url);

    let mut command = Command::new("restic");
    command
        .arg("backup")
        .arg("--json");


    setup_restic_standard_options( &repo, &mut command);

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

fn setup_restic_standard_options(repo: &Repo, command: &mut Command) {
    // Set repo
    command
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
}
