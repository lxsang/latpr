//! # //! Echo publisher example of the tunnel API
//!
//! **Author**: "Dany LE"
//!
use std::env;
use std::panic;
use latpr::utils::*;
use latpr::tunnel::*;
use latpr::utils::{LogLevel, LOG};
use latpr::{ERROR, INFO, WARN, EXIT};


/// Callback: clean up function
///
/// This function remove the unix socket file if
/// exist before quiting the program
///
/// # Arguments
///
/// * `n` - system exit code
fn clean_up(n: i32) {
    if n != 0 {
        panic!("{}", format!("efcgi is terminated by system signal: {}", n));
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // init the system log
    // Create an empty log object and keep it alive in the scope
    // of `main`. When this object is dropped, the syslog will
    // be closed automatically
    let _log = LOG::init_log();
    on_exit(clean_up);

    // read all the arguments
    let args: Vec<String> = env::args().collect();
    // there must be minimum 3 arguments:
    // - the program
    // - the socket file
    // - the topic name
    if args.len() != 3 
    {
        EXIT!("Invalid arguments: {}",  format!("{:?}", args));
    }
    let mut topic = Topic::create(&args[2], &args[1]);
    topic.open()?;
    topic.close()?;
    println!("GOOD");
    Ok(())
}