use structopt;
#[macro_use]
extern crate structopt_derive;

use serde_json;
#[macro_use]
extern crate serde_derive;

use serde_derive::Serialize;

use chrono::{DateTime, Utc};
use scoped_threadpool::Pool;
use std::collections::HashMap;
use std::process::exit;
use std::process::Command;
use structopt::StructOpt;

// use lettre::smtp::authentication::{Credentials, Mechanism};
// use lettre::smtp::ConnectionReuseParameters;
// use lettre::{Transport};
// use lettre_email::EmailBuilder;
// use lettre::SmtpClient;

use lettre::transport::smtp::authentication::{Credentials, Mechanism};
use lettre::{Message, SmtpTransport, Transport};

mod config;
use crate::config::Config;
use crate::config::Repo;
use crate::config::SmtpNotificationConfig;
use std::error::Error;
use std::fmt;
use std::ops::Add;

#[derive(StructOpt, Debug)]
#[structopt(name = "resticmgr", about = "My Restic manager.")]
struct Args {
    /// Whether to redirect success output to email
    #[structopt(
    short = "e",
    long = "mailonsuccess",
    help = "On success output to email"
    )]
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

type ThreadResults<'a> = HashMap<&'a String, BoxResult<String>>;

#[derive(Debug)]
struct MyError {
    details: String
}
impl MyError {
    fn new<M: Into<String>>(msg: M) -> MyError {
        MyError{details: msg.into()}
    }
}
impl fmt::Display for MyError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f,"{}",self.details)
    }
}
impl Error for MyError {
    fn description(&self) -> &str {
        &self.details
    }
}

type BoxResult<T> = Result<T,Box<dyn Error>>;

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
                    Err( MyError::new("Invalid command").into() )
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

fn init_repo(conf: &Config, repo_name_arg: Option<String>) -> BoxResult<()> {
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
                Err( MyError::new( String::from_utf8_lossy(&output.stderr).to_string() ).into() )
            }
        } else {
            Err(MyError::new( format!("Couldn't find a repo named {}", repo_name)).into())
        }
    } else {
        Err( "reponame is a required argument for the init command".into() )
    }
}

fn backup_all(conf: &Config, mail_on_success: bool) -> BoxResult<()> {
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

fn check_last_snapshots_for_all(conf: &Config, mail_on_success: bool) -> BoxResult<()> {
    if !mail_on_success {
        println!("Checking snapshots...");
    }

    let mut thread_pool = Pool::new(4);
    let mut restic_results = HashMap::new();
    let repos = conf.repos.values();
    if repos.len() < 1 {
        return Err("No repositories found to verify.".into());
    }
    for repo in repos {
        thread_pool.scoped(|_scope| {
            restic_results.insert(&repo.url, check_last_snapshot(repo));
        });
    }
    handle_thread_results(conf, mail_on_success, restic_results)
}

fn test_smtp(conf: &SmtpNotificationConfig) -> BoxResult<()> {
    match send_smtp(
        conf,
        "Resticmgr SMTP notification test",
        "This is a test email from resticmgr.",
    ) {
        Ok(_) => {
            println!("SMTP test sent");
            Ok(())
        }
        Err(err) => Err( MyError::new( format!("SMTP test failed: {}", err)).into() ),
    }
}

fn backup_to_single_repo(repo: &Repo, dirs: &[String]) -> Result<String, Box<dyn Error>> {
    let mut command = Command::new("restic");
    command.arg("backup").arg("--json").arg("-q");

    setup_restic_standard_options(repo, &mut command);

    // Set directories
    command.args(dirs.iter());

    // Run
    let output = command.output()?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else if output.status.code() == Some(3) {
        // This means some files didn't back up. Return success but show errors
        Ok(
            String::from_utf8_lossy(&output.stdout).to_string()
                .add(
                    String::from_utf8_lossy(&output.stderr).as_ref()
                )
        )
    }else{
        Err(MyError::new( String::from_utf8_lossy(&output.stderr).to_string() ).into() )
    }
}

fn check_last_snapshot(repo: &Repo) -> BoxResult<String> {
    let mut command = Command::new("restic");
    command.arg("snapshots").arg("--last").arg("--json");

    setup_restic_standard_options(repo, &mut command);

    // Run
    let output = command.output()?;
    if output.status.success() {
        let res = &*String::from_utf8_lossy(&output.stdout);
        match serde_json::from_str(res) as Result<Vec<SnapshotInfo>, serde_json::Error> {
            Ok(ref mut snapshots) => {
                if let Some(ref snapshot) = snapshots.pop() {
                    let ago = Utc::now().signed_duration_since(snapshot.time);
                    Ok(format!(
                        "Last backed up {} days, ({} hours), ({} mins) ago. Paths: {}",
                        ago.num_days(),
                        ago.num_hours(),
                        ago.num_minutes(),
                        snapshot.paths.join(", ")
                    ))
                } else {
                    Err("No snapshots found".into())
                }
            }
            Err(err) => Err(MyError::new( format!(
                "Couldn't parse response from restic: {}",
                err
            )).into()),
        }
    } else {
        Err(MyError::new( String::from_utf8_lossy(&output.stderr) ).into() )
    }
}

fn setup_restic_standard_options(repo: &Repo, command: &mut Command) {
    // Set repo
    command.arg("-r").arg(&repo.url);
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

fn handle_thread_results(
    conf: &Config,
    mail_on_success: bool,
    restic_results: ThreadResults<'_>,
) -> BoxResult<()> {
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
                Err(e) => errors.push(format!("Unable to send notification email for results: {}", e)),
            }
        } else {
            println!("{}", msgs.join("\n"));
        }
    }
    if !errors.is_empty() {
        eprintln!("{}", errors.join("\n"));
        match send_smtp(&conf.smtpnotify, "Restic results", &errors.join("\n")) {
            Ok(_) => {}
            Err(e) => eprintln!("Unable to send notification email for errors: {}", e),
        }
        Err(" ".into())
    } else {
        Ok(())
    }
}

fn send_smtp(conf: &SmtpNotificationConfig, subject: &str, msg: &str) -> BoxResult<()> {
    let email = Message::builder()
        .to(conf.to.clone().parse().expect("Couldn't parse TO address"))
        .from(conf.from.clone().parse().expect("Couldn't parse FROM address"))
        .subject(subject)
        .body(msg.to_string())?;

    // Connect to a remote server on a custom port
    let mailer = SmtpTransport::relay(&conf.server.clone())?
        .credentials(Credentials::new(
            conf.username.clone(),
            conf.password.clone(),
        ))
        //.smtp_utf8(true)
        .authentication(vec!(Mechanism::Login))
        .build();
    let res = mailer.send(&email);
    if res.is_ok(){
        Ok(())
    }else{
        Err( MyError::new( format!("Couldn't send email: {:?}", res  ) ).into() )
    }
}
