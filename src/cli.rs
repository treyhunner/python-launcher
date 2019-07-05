use std::{
    cmp, env,
    fmt::Write,
    io::{BufRead, BufReader, Read},
    iter::FromIterator,
    path::{Path, PathBuf},
    str::FromStr,
    string::ToString,
};

use crate::path;
use crate::version::RequestedVersion;

pub enum Action {
    Help(String, PathBuf),
    List(String),
    Execute {
        launcher_path: PathBuf,
        executable: PathBuf,
        args: Vec<String>,
    },
}

impl Action {
    pub fn from_main(argv: &[String]) -> Result<Self, String> {
        let mut args = argv.to_owned();
        let mut requested_version = RequestedVersion::Any;
        let launcher_path = PathBuf::from(args.remove(0)); // Strip the path to this executable.

        if !args.is_empty() {
            let flag = &args[0];

            if flag == "-h" || flag == "--help" {
                return match help(&launcher_path) {
                    Ok((message, executable_path)) => Ok(Action::Help(message, executable_path)),
                    Err(message) => Err(message),
                };
            } else if flag == "--list" {
                return match list_executables() {
                    Ok(list) => Ok(Action::List(list)),
                    Err(message) => Err(message),
                };
            } else if let Some(version) = version_from_flag(&flag) {
                args.remove(0);
                requested_version = version;
            }
        }

        let directories = path::path_entries();

        match path::find_executable(requested_version, directories.into_iter()) {
            Some(executable) => Ok(Action::Execute {
                launcher_path,
                executable,
                args,
            }),
            None => Err("no Python executable found".to_string()),
        }
    }
}

fn help(launcher_path: &Path) -> Result<(String, PathBuf), String> {
    let mut message = String::new();
    let directories = path::path_entries();

    if let Some(found_path) = path::find_executable(RequestedVersion::Any, directories.into_iter())
    {
        writeln!(
            message,
            include_str!("HELP.txt"),
            env!("CARGO_PKG_VERSION"),
            launcher_path.to_string_lossy(),
            found_path.to_string_lossy()
        )
        .unwrap();
        return Ok((message, found_path));
    } else {
        return Err("no Python executable found".to_string());
    }
}

/// Attempts to find a version specifier from a CLI argument.
///
/// It is assumed that the flag from the command-line is passed as-is
/// (i.e. the flag starts with `-`).
pub fn version_from_flag(arg: &str) -> Option<RequestedVersion> {
    if !arg.starts_with('-') {
        None
    } else {
        RequestedVersion::from_str(&arg[1..]).ok()
    }
}

pub fn list_executables() -> Result<String, String> {
    let paths = path::path_entries();
    let executables = path::all_executables(paths.into_iter());

    if executables.is_empty() {
        return Err("No Python executable found".to_string());
    }

    let mut executable_pairs = Vec::from_iter(executables);
    executable_pairs.sort_unstable();

    let max_version_length = executable_pairs.iter().fold(0, |max_so_far, pair| {
        cmp::max(max_so_far, pair.0.to_string().len())
    });

    let left_column_width = cmp::max(max_version_length, "Version".len());
    let mut help_string = String::new();
    // Including two spaces between columns for readability.
    writeln!(help_string, "{:<1$}  Path", "Version", left_column_width).unwrap();
    writeln!(help_string, "{:<1$}  ====", "=======", left_column_width).unwrap();

    for (version, path) in executable_pairs {
        writeln!(
            help_string,
            "{:<2$}  {}",
            version.to_string(),
            path.to_string_lossy(),
            left_column_width
        )
        .unwrap();
    }

    Ok(help_string)
}

/// Returns the path to the activated virtual environment.
///
/// A virtual environment is determined to be activated based on the existence of the `VIRTUAL_ENV`
/// environment variable.
pub fn activated_venv_executable() -> Option<PathBuf> {
    match env::var_os("VIRTUAL_ENV") {
        None => None,
        Some(venv_root) => {
            let mut path = PathBuf::new();
            path.push(venv_root);
            path.push("bin");
            path.push("python");
            // TODO: Do a is_file() check first?
            Some(path)
        }
    }
}

// https://en.m.wikipedia.org/wiki/Shebang_(Unix)
pub fn parse_python_shebang(reader: &mut impl Read) -> Option<RequestedVersion> {
    let mut shebang_buffer = [0; 2];
    if reader.read(&mut shebang_buffer).is_err() || shebang_buffer != [0x23, 0x21] {
        // Doesn't start w/ `#!` in ASCII/UTF-8.
        return None;
    }

    let mut buffered_reader = BufReader::new(reader);
    let mut first_line = String::new();

    if buffered_reader.read_line(&mut first_line).is_err() {
        return None;
    };

    // Whitespace between `#!` and the path is allowed.
    let line = first_line.trim();

    let accepted_paths = [
        "python",
        "/usr/bin/python",
        "/usr/local/bin/python",
        "/usr/bin/env python",
    ];

    for acceptable_path in &accepted_paths {
        if !line.starts_with(acceptable_path) {
            continue;
        }

        return match RequestedVersion::from_str(&acceptable_path[acceptable_path.len()..]) {
            Ok(version) => Some(version),
            Err(_) => None,
        };
    }

    None
}

/// Finds the shebang line from `reader`.
///
/// If a shebang line is found, then the `#!` is removed and the line is stripped of leading and trailing whitespace.
pub fn find_shebang(reader: impl Read) -> Option<String> {
    let mut buffered_reader = BufReader::new(reader);

    let mut line = String::new();
    if buffered_reader.read_line(&mut line).is_err() {
        return None;
    };

    if !line.starts_with("#!") {
        None
    } else {
        Some(line[2..].trim().to_string())
    }
}

/// Split the shebang into the Python version specified and the arguments to pass to the executable.
///
/// `Some` is only returned if the specified executable is one of:
/// - `/usr/bin/python`
/// - `/usr/local/bin/python`
/// - `/usr/bin/env python`
/// - `python`
pub fn split_shebang(shebang_line: &str) -> Option<(RequestedVersion, Vec<String>)> {
    let accepted_paths = [
        "/usr/bin/python",
        "/usr/local/bin/python",
        "/usr/bin/env python",
        "python",
    ];

    for exec_path in &accepted_paths {
        if !shebang_line.starts_with(exec_path) {
            continue;
        }

        let trimmed_shebang = shebang_line[exec_path.len()..].to_string();
        let version_string: String = trimmed_shebang
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        let specified_version = if version_string.is_empty() {
            Ok(RequestedVersion::MajorOnly(2))
        } else {
            RequestedVersion::from_str(&version_string)
        };

        return specified_version
            .map(|version| {
                let args = trimmed_shebang[version_string.len()..].trim();
                (
                    version,
                    args.split_whitespace().map(|s| s.to_string()).collect(),
                )
            })
            .ok();
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_from_flag() {
        assert!(version_from_flag(&"-S".to_string()).is_none());
        assert!(version_from_flag(&"--something".to_string()).is_none());
        assert_eq!(
            version_from_flag(&"-3".to_string()),
            Some(RequestedVersion::MajorOnly(3))
        );
        assert_eq!(
            version_from_flag(&"-3.6".to_string()),
            Some(RequestedVersion::Exact(3, 6))
        );
        assert_eq!(
            version_from_flag(&"-42.13".to_string()),
            Some(RequestedVersion::Exact(42, 13))
        );
        assert!(version_from_flag(&"-3.6.4".to_string()).is_none());
    }

    #[test]
    fn test_virtual_env() {
        let original_venv = env::var_os("VIRTUAL_ENV");

        env::remove_var("VIRTUAL_ENV");
        assert_eq!(activated_venv_executable(), None);

        env::set_var("VIRTUAL_ENV", "/some/path");
        assert_eq!(
            activated_venv_executable(),
            Some(PathBuf::from("/some/path/bin/python"))
        );

        match original_venv {
            None => env::remove_var("VIRTUAL_ENV"),
            Some(venv_value) => env::set_var("VIRTUAL_ENV", venv_value),
        }
    }

    #[test]
    fn test_find_shebang() {
        // Common case.
        assert_eq!(
            find_shebang("#! /usr/bin/cat\nprint('Hello!')\n".as_bytes()),
            Some("/usr/bin/cat".to_string())
        );

        // No shebang.
        assert_eq!(find_shebang("print('Hello!')".as_bytes()), None);

        // No whitespace between `#!` and command.
        assert_eq!(
            find_shebang("#!/usr/bin/cat\nHello".as_bytes()),
            Some("/usr/bin/cat".to_string())
        );

        // Command wtih arguments.
        assert_eq!(
            find_shebang("#! /usr/bin/env python -S".as_bytes()),
            Some("/usr/bin/env python -S".to_string())
        );

        // Strip trailing whitespace.
        assert_eq!(
            find_shebang("#! /usr/bin/python \n# Hello".as_bytes()),
            Some("/usr/bin/python".to_string())
        );

        // Nothing but a shebang.
        assert_eq!(
            find_shebang("#! /usr/bin/python".as_bytes()),
            Some("/usr/bin/python".to_string())
        );
    }

    #[test]
    fn test_split_shebang() {
        assert_eq!(split_shebang(&"/usr/bin/rustup".to_string()), None);
        assert_eq!(
            split_shebang(&"/usr/bin/rustup self update".to_string()),
            None
        );
        assert_eq!(
            split_shebang(&"/usr/bin/env python".to_string()),
            Some((RequestedVersion::MajorOnly(2), Vec::new()))
        );
        assert_eq!(
            split_shebang(&"/usr/bin/python42.13".to_string()),
            Some((RequestedVersion::Exact(42, 13), Vec::new()))
        );
        assert_eq!(
            split_shebang(&"python -S -v".to_string()),
            Some((
                RequestedVersion::MajorOnly(2),
                vec!["-S".to_string(), "-v".to_string()]
            ))
        );
        assert_eq!(
            split_shebang(&"/usr/local/bin/python3.7 -S".to_string()),
            Some((RequestedVersion::Exact(3, 7), vec!["-S".to_string()]))
        );
    }
}
