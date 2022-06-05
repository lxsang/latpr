//! # //! Common utilities and system specific functions and structures
//!
//! **Author**: "Dany LE"
//!
use libc;
use std::collections::HashMap;
use std::error::Error;
use std::ffi::{CStr, CString};
use std::fmt::Arguments;
use std::fs::File;
use std::io::prelude::*;
use std::io::BufReader;
use std::mem;
use std::os::unix::io::RawFd;
use std::path::Path;
use std::ptr;

/// app version
pub const API_VERSION: &str = "0.1.0";

/// Application name
const DAEMON_NAME: &str = "antd-tunnel";

/// Return an Error Result object from error string
///
#[macro_export]
macro_rules! ERR {
    ($x:expr) => {
        Box::new(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("({}:{}): {}", file!(), line!(), $x),
        ))
    };
}

/// Macro for error log helper
///
#[macro_export]
macro_rules! INFO {
    ($($args:tt)*) => ({
        if std::env::var("debug").is_ok()
        {
            let prefix = format!(":info@[{}:{}]: ",file!(), line!());
            let _ = LOG::log(&prefix[..], &LogLevel::INFO, format_args!($($args)*));
        }
    })
}

/// Macro for warning log helper
///
#[macro_export]
macro_rules! WARN {
    ($($args:tt)*) => ({
        let prefix = format!(":warning@[{}:{}]: ",file!(), line!());
        let _ = LOG::log(&prefix[..], &LogLevel::WARN, format_args!($($args)*));
    })
}

/// Macro for info log helper
///
#[macro_export]
macro_rules! ERROR {
    ($($args:tt)*) => ({
        let prefix = format!(":error@[{}:{}]: ",file!(), line!());
        let _ = LOG::log(&prefix[..], &LogLevel::ERROR, format_args!($($args)*));
    })
}

/// Log and quit
///
#[macro_export]
macro_rules! EXIT {
    ($($args:tt)*) => ({
        ERROR!($($args)*);
        panic!("{}",  format_args!($($args)*));
    })
}

/// Different Logging levels for `LOG`
pub enum LogLevel {
    /// Error conditions
    ERROR,
    /// Normal, but significant, condition
    INFO,
    /// Warning conditions
    WARN,
}

/// Log struct wrapper
///
pub struct LOG {}

impl LOG {
    /// Init the system log
    ///
    /// This should be called only once in the entire lifetime
    /// of the program, the returned LOG instance should
    /// be keep alive during the lifetime of the program (the main function).
    /// When it is dropped, the connection to the system log will be
    /// closed automatically
    #[must_use]
    pub fn init_log() -> Self {
        // connect to the system log
        unsafe {
            libc::openlog(
                std::ptr::null(),
                libc::LOG_CONS | libc::LOG_PID | libc::LOG_NDELAY,
                libc::LOG_DAEMON,
            );
        }
        Self {}
    }

    /// Wrapper function that log error or info message to the
    /// connected syslog server
    ///
    /// # Arguments
    ///
    /// * `prefix` - Prefix of the log message
    /// * `level` - Log level
    /// * `args` - Arguments object representing a format string and its arguments
    ///
    /// # Errors
    ///
    /// * `error` - All errors related to formated and C string manipulation
    pub fn log(prefix: &str, level: &LogLevel, args: Arguments<'_>) -> Result<(), Box<dyn Error>> {
        use std::fmt::Write;
        let mut output = String::new();
        if output.write_fmt(args).is_err() {
            return Err(ERR!("Unable to create format string from arguments"));
        }
        let log_fmt = format!("{}(v{}){}%s\n", DAEMON_NAME, API_VERSION, prefix);
        let fmt = CString::new(log_fmt.as_bytes())?;
        let c_msg = CString::new(output.as_bytes())?;
        let sysloglevel = match level {
            LogLevel::ERROR => libc::LOG_ERR,
            LogLevel::WARN => libc::LOG_WARNING,
            _ => libc::LOG_NOTICE,
        };
        unsafe {
            libc::syslog(sysloglevel, fmt.as_ptr(), c_msg.as_ptr());
        }
        Ok(())
    }
}

impl Drop for LOG {
    /// The connection to the syslog will be closed
    /// automatically when the log object is drop
    fn drop(&mut self) {
        // Close the current connection to the system logger
        unsafe {
            libc::closelog();
        }
    }
}

/// Utility function to catch common signal that
/// cause the program to exit
///
/// Signals catched: SIGABRT, SIGINT, SIGTERM, SIGQUIT
///
/// # Arguments
///
/// * `f` - callback function that will be called when a signal is trapped
pub fn on_exit(f: fn(n: i32) -> ()) {
    unsafe {
        let _ = libc::signal(libc::SIGPIPE, libc::SIG_IGN);
        let _ = libc::signal(libc::SIGABRT, (f as *const std::ffi::c_void) as usize);
        let _ = libc::signal(libc::SIGINT, (f as *const std::ffi::c_void) as usize);
        let _ = libc::signal(libc::SIGTERM, (f as *const std::ffi::c_void) as usize);
        let _ = libc::signal(libc::SIGQUIT, (f as *const std::ffi::c_void) as usize);
    };
}

/// Utility function to get current UNIX username
///
/// This function relies on some low level libc function
/// to get the username from user uid
///
/// # Errors
///
/// * `std error` - All error related to lib ffi calls
pub fn get_username() -> Result<String, Box<dyn Error>> {
    let mut passwd_ptr = unsafe { mem::zeroed::<libc::passwd>() };
    let mut buf = vec![0; 1024];
    let mut result = ptr::null_mut::<libc::passwd>();

    unsafe {
        let _ = libc::getpwuid_r(
            libc::geteuid(),
            &mut passwd_ptr,
            buf.as_mut_ptr(),
            buf.len(),
            &mut result,
        );
    }

    if result.is_null() {
        // There is no such user, or an error has occurred.
        // errno gets set if there’s an error.
        return Err(ERR!("get_username: Result of getpwuid_r is NULL"));
    }

    if result != &mut passwd_ptr {
        // The result of getpwuid_r should be its input passwd.
        return Err(ERR!(
            "get_username: result pointer of getpwuid_r does not match input passwd pointer"
        ));
    }

    if let Ok(username) = unsafe { CStr::from_ptr(passwd_ptr.pw_name) }.to_str() {
        Ok(String::from(username))
    } else {
        Err(ERR!(
            "get_username: Unable to extract username from passwd struct"
        ))
    }
}

/// Drop user privileges
///
/// This function drop the privileges of the current user
/// to another inferior privileges user.
/// e.g. drop from root->maint
///
/// # Arguments
///
/// * `optuser` - Option object that contains system user name
/// * `optgroup` - Option object that contains system group name
///
/// # Errors
///
/// * `Invalid user/group name` - The input user/group name is None
/// * `getgrnam` - Error when calling the libc `getgrnam` function
/// * `setgid` - Error when calling the libc `setgid` function
/// * `getpwnam` - Error when calling the libc `getpwnam` function
/// * `setuid` - Error when calling the libc `setuid` function
/// * `CString from String` - Error creating `CString` from Rust `String`
pub fn privdrop(optuser: Option<&String>, optgroup: Option<&String>) -> Result<(), Box<dyn Error>> {
    if optuser.is_none() && optgroup.is_none() {
        return Err(ERR!("No user or group found!"));
    }
    // the group id need to be set first, otherwise,
    // when the user privileges drop, it is unnable to
    // set the group id
    if let Some(group) = optgroup {
        // get the uid from username
        if let Ok(cstr) = CString::new(group.as_bytes()) {
            let p = unsafe { libc::getgrnam(cstr.as_ptr()) };
            if p.is_null() {
                return Err(ERR!(format!(
                    "privdrop: Unable to getgrnam of group `{}`: {}",
                    group,
                    std::io::Error::last_os_error()
                )));
            }
            if unsafe { libc::setgid((*p).gr_gid) } != 0 {
                return Err(ERR!(format!(
                    "privdrop: Unable to setgid of group `{}`: {}",
                    group,
                    std::io::Error::last_os_error()
                )));
            }
        } else {
            return Err(ERR!("Cannot create CString from String (group)!"));
        }
    }
    // drop the user privileges
    if let Some(user) = optuser {
        // get the uid from username
        if let Ok(cstr) = CString::new(user.as_bytes()) {
            let p = unsafe { libc::getpwnam(cstr.as_ptr()) };
            if p.is_null() {
                return Err(ERR!(format!(
                    "privdrop: Unable to getpwnam of user `{}`: {}",
                    user,
                    std::io::Error::last_os_error()
                )));
            }
            if unsafe { libc::setuid((*p).pw_uid) } != 0 {
                return Err(ERR!(format!(
                    "privdrop: Unable to setuid of user ``{}`: {}",
                    user,
                    std::io::Error::last_os_error()
                )));
            }
        } else {
            return Err(ERR!("Cannot create CString from String (user)!"));
        }
    }
    Ok(())
}

/// Utility function to read config file
///
/// The configuration in the following format:
///
/// ```ini
/// configuration_name = configuration value
/// # this is the comment in the configuration file
/// ```
///
/// # Arguments
///
/// * `file` - The path to the configuration file
///
/// # Errors
///
/// * `read_config` - Unable to read config file
pub fn read_config(file: &str) -> Result<HashMap<String, String>, Box<dyn Error>> {
    if let Ok(f) = File::open(file) {
        let mut map = HashMap::new();
        let buf = BufReader::new(f);
        buf.lines()
            .filter_map(std::result::Result::ok)
            .filter(|s| {
                if let Some(ch) = s.trim_start().chars().next() {
                    ch != '#'
                } else {
                    true
                }
            })
            .for_each(|s: String| {
                if let Some(i) = s.find('=') {
                    let _ = map.insert(
                        String::from(s[..i - 1].trim()),
                        String::from(s[i + 1..].trim().trim_matches('"')),
                    );
                }
            });
        Ok(map)
    } else {
        Err(ERR!(format!("Unable to open config file {}", file)))
    }
}

/// Urldecode
///
/// This function decode percent encoded url to
/// original url
///
/// # Arguments
/// * `url` - url  string to be decoded
#[must_use]
pub fn urldecode(url: &str) -> String {
    let mut decoded = String::from("");
    let mut skip = 0;
    for i in 0..url.len() {
        if skip != 0 {
            skip -= 1;
            continue;
        }
        let c: char = url.chars().nth(i).unwrap();
        if c == '%' {
            let left = url.chars().nth(i + 1).unwrap();
            let right = url.chars().nth(i + 2).unwrap();
            let byte = u8::from_str_radix(&format!("{}{}", left, right), 16).unwrap();
            decoded += &(byte as char).to_string();
            skip = 2;
        } else {
            decoded += &c.to_string();
        }
    }
    decoded
}

/// Get file basename from a Path
///
/// This function return the string basename of a file
///
/// # Arguments
///
/// * `path` - a `Path` object
#[must_use]
pub fn get_basename_str(path: &Path) -> Option<&str> {
    if path.exists() {
        if let Some(osbasename) = path.file_name() {
            if let Some(basename) = osbasename.to_str() {
                return Some(basename);
            }
        }
    }
    None
}

/// Extract error message from `Any` error object
///
/// # Arguments
///
/// * `error` - any error object
#[must_use]
pub fn error_string(error: Box<dyn std::any::Any + Send>) -> String {
    match error.downcast::<String>() {
        Ok(panic_msg) => panic_msg.to_string(),
        Err(_) => String::from("Unknown error type"),
    }
}

/// Convert u8 array to str string
///
/// # Arguments
///
/// * `data` - u8 vector
///
/// # Errors
///
/// * `std io error` - conversion error
pub fn string_from_u8(data: &[u8]) -> Result<String, Box<dyn Error>> {
    match std::str::from_utf8(data) {
        Ok(s) => Ok(String::from(s)),
        Err(error) => Err(ERR!(format!(
            "str_from_u8: {}",
            error_string(Box::new(error))
        ))),
    }
}

/// Return the number of bytes available to read from a raw fd.
/// Can be used with unix socket file descriptor to determine if it is readable
///
/// # Arguments
///
/// * `fd` - Unix raw fd
#[must_use]
pub fn fd_available(fd: RawFd) -> i32 {
    let mut num_available: libc::c_int = 0;
    let ret = unsafe { libc::ioctl(fd, libc::FIONREAD, &mut num_available) };
    if ret == -1 {
        return -1;
    }
    num_available
}
