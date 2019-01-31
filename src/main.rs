extern crate structopt;
#[macro_use]
extern crate structopt_derive;
extern crate scoped_threadpool;
extern crate serde;
extern crate serde_json;
#[macro_use]
extern crate serde_derive;
extern crate failure;
extern crate chrono;
extern crate lettre;
extern crate lettre_email;

use failure::{Error, err_msg};
use structopt::StructOpt;
use std::process::Command;
use scoped_threadpool::Pool;
use std::collections::HashMap;
use chrono::{DateTime, Utc};
use std::process::exit;

use lettre::smtp::authentication::{Credentials, Mechanism};
use lettre::{EmailTransport, SmtpTransport};
use lettre::smtp::ConnectionReuseParameters;
use lettre_email::EmailBuilder;
use lettre::smtp::error::SmtpResult;

mod config;
use config::Repo;
use config::Config;
use config::SmtpNotificationConfig;

#[derive(StructOpt, Debug)]
#[structopt(name = "resticmgr", about = "My Restic manager.")]
struct Args {
    /// Whether to redirect success output to email
    #[structopt(short = "e", long = "mailonsuccess", help = "On success output to email")]
    mail_on_success: bool,
    #[structopt(short = "r", long = "reponame", help = "Repo name when running init")]
    repo_name: Option<String>,
    #[structopt(help = "Action")]
    action: String,
    #[structopt(help = "Config file (JSON)", default_value = "./config.json")]
    config_file: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct SnapshotInfo {
    time: DateTime<Utc>,
    parent: Option<String>,
    tree: String,
    paths: Vec<String>,
    hostname: String,
    username: Option<String>,
    uid: Option<isize>,
    gid: Option<isize>,
    id: String,
    short_id: String,
}

type ThreadResults<'a> = HashMap<&'a String, Result<String, Error>>;

fn main() {
    let args = Args::from_args();
    match config::load(args.config_file) {
        Ok(conf) => {
            match match args.action.as_ref() {
                "backup" => backup_all(&conf, args.mail_on_success),
                "verify" => check_last_snapshots_for_all(&conf, args.mail_on_success),
                "init" => init_repo(&conf, args.repo_name),
                "testsmtp" => test_smtp(&conf.smtpnotify),
                _ => {
                    eprintln!("Invalid command '{}'", args.action);
                    Err(err_msg("Invalid command"))
                }
            } {
                Ok(_) => exit(0),
                Err(err) => {
                    eprintln!("{}", err);
                    exit(1)
                }
            }
        }
        Err(err) => {
            eprintln!("Failed to load config file: {}", err);
            exit(1);
        }
    }
}

fn init_repo(conf: &Config, repo_name_arg: Option<String>) -> Result<(), Error> {
    let mut command = Command::new("restic");
    command.arg("init");

    if let Some(repo_name) = repo_name_arg {
        if let Some(repo) = conf.repos.get(&repo_name) {
            setup_restic_standard_options(repo, &mut command);

            // Run
            let output = command.output()?;
            if output.status.success() {
                Ok(())
            } else {
                Err(err_msg(String::from_utf8_lossy(&output.stderr).to_string()))
            }
        } else {
            Err(err_msg(format!("Couldn't find a repo named {}", repo_name)))
        }
    } else {
        Err(err_msg("reponame is a required argument for the init command"))
    }



}

fn backup_all(conf: &Config, mail_on_success: bool) -> Result<(), Error> {
    if !mail_on_success {
        println!("Backing up...");
    }

    let mut thread_pool = Pool::new(4);
    let mut restic_results = HashMap::new();
    for set in &conf.backupsets {
        let repos = set.repos(conf)?;
        for repo in repos {
            thread_pool.scoped(|_scope| {
                restic_results.insert(&repo.url, backup_to_single_repo(repo, &set.dirs));
            });
        }
    }
    handle_thread_results(conf, mail_on_success, restic_results)
}

fn check_last_snapshots_for_all(conf: &Config, mail_on_success: bool) -> Result<(), Error> {
    if !mail_on_success {
        println!("Checking snapshots...");
    }

    let mut thread_pool = Pool::new(4);
    let mut restic_results = HashMap::new();
    let repos = conf.repos.values();
    if repos.len() < 1 {
        return Err( err_msg( "No repositories found to verify.") );
    }
    for repo in repos {
        thread_pool.scoped(|_scope| {
            restic_results.insert(&repo.url, check_last_snapshot(repo));
        });
    }
    handle_thread_results(conf, mail_on_success, restic_results)
}

fn test_smtp(conf: &SmtpNotificationConfig) -> Result<(), Error> {
    match send_smtp(conf,
                    "Resticmgr SMTP notification test",
                    "This is a test email from resticmgr.") {
        Ok(_) => {
            println!("SMTP test sent");
            Ok(())
        }
        Err(err) => Err(err_msg(format!("SMTP test failed: {}", err))),
    }
}

fn backup_to_single_repo(repo: &Repo, dirs: &[String]) -> Result<String, Error> {
    let mut command = Command::new("restic");
    command.arg("backup")
        .arg("--json");


    setup_restic_standard_options(repo, &mut command);

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

fn check_last_snapshot(repo: &Repo) -> Result<String, Error> {
    let mut command = Command::new("restic");
    command.arg("snapshots")
        .arg("--last")
        .arg("--json");

    setup_restic_standard_options(repo, &mut command);

    // Run
    let output = command.output()?;
    if output.status.success() {
        let res = &*String::from_utf8_lossy(&output.stdout);
        match serde_json::from_str(res) as Result<Vec<SnapshotInfo>, serde_json::Error> {
            Ok(ref mut snapshots) => {
                if let Some(ref snapshot) = snapshots.pop() {
                    let ago = Utc::now().signed_duration_since(snapshot.time);
                    Ok(format!("Last backed up {} days, ({} hours), ({} mins) ago. Paths: {}",
                               ago.num_days(),
                               ago.num_hours(),
                               ago.num_minutes(),
                               snapshot.paths.join(", ")))
                } else {
                    Err(err_msg("No snapshots found"))
                }
            }
            Err(err) => Err(err_msg(format!("Couldn't parse response from restic: {}", err))),
        }
    } else {
        Err(err_msg(String::from_utf8_lossy(&output.stderr).to_string()))
    }
}

fn setup_restic_standard_options(repo: &Repo, command: &mut Command) {
    // Set repo
    command.arg("-r")
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

fn handle_thread_results(conf: &Config,
                         mail_on_success: bool,
                         restic_results: ThreadResults)
                         -> Result<(), Error> {
    let mut msgs: Vec<String> = vec![];
    let mut errors: Vec<String> = vec![];
    for (repo, output) in restic_results {
        msgs.push(format!("Repository '{}' results:", repo));
        match output {
            Ok(out) => msgs.push(format!("{}", out)),
            Err(err) => {
                errors.push(format!("Error for '{}': {}", repo, err));
            }
        }
    }
    if !msgs.is_empty() {
        if mail_on_success {
            match send_smtp(&conf.smtpnotify, "Restic results", &msgs.join("\n")) {
                Ok(_) => {}
                Err(_) => errors.push("Unable to send notification email for results!".into()),
            }
        } else {
            println!("{}", msgs.join("\n"));
        }
    }
    if !errors.is_empty() {
        eprintln!("{}", errors.join("\n"));
        match send_smtp(&conf.smtpnotify, "Restic results", &errors.join("\n")) {
            Ok(_) => {}
            Err(_) => eprintln!("Unable to send notification email for errors!"),
        }
        Err(err_msg(" "))
    } else {
        Ok(())
    }
}

fn send_smtp(conf: &SmtpNotificationConfig, subject: &str, msg: &str) -> SmtpResult {
    let email = EmailBuilder::new()
        .to(conf.to.clone())
        .from(conf.from.clone())
        .subject(subject)
        .text(msg)
        .build()
        .unwrap();

    // Connect to a remote server on a custom port
    let mut mailer = SmtpTransport::simple_builder(conf.server.clone())
        .unwrap()
        .credentials(Credentials::new(conf.username.clone(), conf.password.clone()))
        .smtp_utf8(true)
        .authentication_mechanism(Mechanism::Login)
        .connection_reuse(ConnectionReuseParameters::ReuseUnlimited)
        .build();
    mailer.send(&email)
}
