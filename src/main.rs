// #![allow(dead_code)]
#![allow(unused_imports)]

extern crate argparse;
extern crate rustc_serialize;
extern crate ansi_term;

use std::path::{PathBuf};
use std::fs::{ReadDir, DirEntry, Metadata, File};
use std::fs::metadata as fs_metadata;
use std::io;
use std::io::{Read, Stdin};
use std::process::{Command, Output};
use std::fmt;

use argparse::{ArgumentParser, Store};

use rustc_serialize::json::{Json, Object};

use ansi_term::Colour::{Black, Yellow};
use ansi_term::Style;

// ------------------------------------------------------------------------------------------------

trait CollectResultExt<T, E> {
    fn collect_result_vec(self) -> Result<Vec<T>, E>;
    
    fn collect_result(self) -> Result<std::vec::IntoIter<T>, E>;
}

impl<T, E, I> CollectResultExt<T, E> for I 
where I: Iterator<Item = Result<T, E>> {
    fn collect_result_vec(self) -> Result<Vec<T>, E> {
        self.collect()
    }
    
    fn collect_result(self) -> Result<std::vec::IntoIter<T>, E> {
        self.collect_result_vec().map(|v| v.into_iter())
    }
}

trait CollectOptionExt<T> {
    fn collect_option_vec(self) -> Option<Vec<T>>;
}

impl<T, I> CollectOptionExt<T> for I
where I: Iterator<Item = Option<T>> {
    fn collect_option_vec(self) -> Option<Vec<T>> {
        self.collect()
    }
}

// ------------------------------------------------------------------------------------------------

fn extract_object(json: Json) -> Option<Object> {
    match json {
        Json::Object(o) => Some(o),
        _ => None
    }
}

fn extract_string(json: Json) -> Option<String> {
    match json {
        Json::String(s) => Some(s),
        _ => None
    }
}

// ------------------------------------------------------------------------------------------------

struct Package {
    attribute: Attribute,
    name: String,
    description: String
}

type Attribute = String;

impl fmt::Debug for Package {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Package({}, {}, {})", self.attribute, self.name, self.description)
    }
}

impl Package {
    fn query(&self, query: &str) -> bool {
        self.attribute.contains(query) ||
        self.name.contains(query) ||
        self.description.contains(query)
    }
}

enum NoxError {
    NoHomeDir,
    Io(io::Error),
    MissingVersionIndicator(PathBuf),
    NixEnvFailed(String, String),
    NixEnvParseError,
    BadInstallRequestFormat(InstallRequestFormatError)
}

enum InstallRequestFormatError {
    NotANumber(String),
    InvalidIndex(usize)
}

impl From<io::Error> for NoxError {
    fn from(io_error: io::Error) -> NoxError {
        NoxError::Io(io_error)
    }
}

impl From<InstallRequestFormatError> for NoxError {
    fn from(error: InstallRequestFormatError) -> NoxError {
        NoxError::BadInstallRequestFormat(error)
    }
}

type NoxResult<T> = Result<T, NoxError>;
type NoxInstallRequest = Result<Option<Vec<i32>>, String>;

// ------------------------------------------------------------------------------------------------

trait Cache<T> {
    fn get_or_create<E>(&mut self, _key: String, create_fn: fn() -> Result<T, E>) -> Result<T, E>;
}

struct NullCache;

impl<T> Cache<T> for NullCache {
    fn get_or_create<E>(&mut self, _key: String, create_fn: fn() -> Result<T, E>) -> Result<T, E> {
        create_fn()
    }
}

// ------------------------------------------------------------------------------------------------

fn nix_packages_json() -> NoxResult<Json> {
    println!("Refreshing cache.");

    Command::new("nix-env")
    .arg("-q")
    .arg("-a")
    .arg("--json")
    .output()
    .map_err(NoxError::Io)
    .and_then(|output: Output| -> NoxResult<String> {
        if !(&output).stderr.is_empty() {
            let stderr = String::from_utf8(output.stderr).ok().expect("nix-env's stderr is utf-8");
            Err(NoxError::NixEnvFailed("nix-env posted to stderr.".to_string(), stderr))
        } else {
            let stdout = String::from_utf8(output.stdout).ok().expect("nix-env's stdout is utf-8");
            Ok(stdout)
        }
    })
    .and_then(|output: String| Json::from_str(&output).map_err(|_e| {
        NoxError::NixEnvFailed("Failed to parse stdout as JSON.".to_string(), output)
    }))
}

// ------------------------------------------------------------------------------------------------

fn make_key_part_nox(channel_dir_path: PathBuf) -> NoxResult<Option<String>> {
    make_key_part(channel_dir_path).map_err(NoxError::Io)
}

fn make_key_part(channel_dir_path: PathBuf) -> io::Result<Option<String>> {
    fn check_manifest(mut channel_dir_path: PathBuf) -> io::Result<Option<String>> {
        channel_dir_path.push("manifest.nix");
        let manifest = channel_dir_path;

        // TODO(Havvy): except (FileNotFoundError, NotADirectoryError):
        let metadata = try!(fs_metadata(&manifest));
        if !metadata.is_file() {
            return Ok(None);
        }

        let mut contents = String::new();
        let mut manifest = try!(File::open(manifest));
        try!((&mut manifest).read_to_string(&mut contents));

        Ok(Some(contents))
    }

    fn check_git(mut channel_dir_path: PathBuf) -> io::Result<Option<String>> {
        channel_dir_path.push(".git");
        let git_dir = channel_dir_path;

        let metadata = try!(fs_metadata(git_dir));
        if !metadata.is_dir() {
            return Ok(None);
        }

        Command::new("git")
        .arg("rev-parse")
        .arg("--verify HEAD")
        .output()
        .map(|output| {
            Some(String::from_utf8(output.stdout).ok().expect("git's stdout is utf-8"))
        })
    }

    {
        let metadata = try!(fs_metadata(&channel_dir_path));
        if !metadata.is_dir() {
            return Ok(None);
        }
    }

    let channel = { (&channel_dir_path).file_name().unwrap().to_str().unwrap().to_string() };

    let mut version = try!(check_manifest(channel_dir_path.clone()));
    if version == None {
        version = try!(check_git(channel_dir_path))
    }

    Ok(version.map(|version| format!("\"{}\": {}", channel, version)))
}

fn make_key() -> Result<String, NoxError> {
    std::env::home_dir()
    .ok_or(NoxError::NoHomeDir)
    .map(|mut home_dir| {
        (&mut home_dir).push(".nix-defexpr");
        home_dir
    })
    .and_then(|defexpr_dir| std::fs::read_dir(defexpr_dir).map_err(NoxError::Io))
    .and_then(|dir_entries| dir_entries.collect_result_vec().map_err(NoxError::Io))
    .and_then(|dir_entries| {
        dir_entries.iter()
        .map(|e: &DirEntry| e.path())
        .map(make_key_part_nox)
        .collect_result()
    })
    .map(|key_parts| {
        let mut middle: Vec<String> = key_parts.filter_map(|p| p).collect();
        middle.sort();
        format!("{{{}}}", middle.connect(", "))
    })
}

// ------------------------------------------------------------------------------------------------

fn parse_package((attribute, attribute_value): (Attribute, Json)) -> Option<Package> {
    extract_object(attribute_value)
    .and_then(|mut attribute_data| {
        let name = attribute_data.remove("name")
        .and_then(extract_string);

        let description = attribute_data.remove("meta")
        .and_then(extract_object)
        .and_then(|mut meta| meta.remove("description"))
        .and_then(extract_string)
        .unwrap_or(String::new());

        name.map(|name| Package {
            attribute: attribute,
            name: name,
            description: description
        })
    })
}

fn parse_packages(nix_packages_json: Json) -> NoxResult<Vec<Package>> {
    extract_object(nix_packages_json)
    .and_then(|o| {
        o.into_iter()
        .map(parse_package)
        .collect_option_vec()
    })
    .ok_or(NoxError::NixEnvParseError)
}

fn all_packages(mut cache: NullCache) -> NoxResult<Vec<Package>> {
    let key = try!(make_key());
    let packages = try!(cache.get_or_create(key, nix_packages_json));
    parse_packages(packages)
}

// ------------------------------------------------------------------------------------------------

fn display_package(ix: &usize, package: &Package) {
    let number = Black.on(Yellow).paint(&ix.to_string()).to_string();
    let name = Style::default().bold().paint(&package.name).to_string();
    let attribute = Style::default().dimmed().paint(&package.attribute).to_string();
    let description = &package.description;

    println!("{} {} ({})\n    {}", number, name, attribute, description);
}

fn request_package_indices_to_install(max_index: usize) -> NoxResult<Option<Vec<usize>>> {
    let mut input = String::new();

    print!("Packages to install: ");
    try!(io::stdin().read_line(&mut input));

    let input = input.trim();

    if input.is_empty() {
        return Ok(None);
    }

    let indices: Result<Vec<usize>, _> = input.split(" ").map(|i| i.parse()).collect();
    let indices = try!(indices.map_err(|_| {
        InstallRequestFormatError::NotANumber(input.to_string())
    }));

    indices.into_iter()
    .map(|index| {
        if index > max_index {
            Err(InstallRequestFormatError::InvalidIndex(index))
        } else {
            Ok(index)
        }
    })
    .collect::<Result<Vec<usize>, InstallRequestFormatError>>()
    .map(Some)
    .map_err(NoxError::BadInstallRequestFormat)
}

// ------------------------------------------------------------------------------------------------

fn install_attributes(attributes: Vec<Attribute>) -> NoxResult<()> {
    let mut install_command = Command::new("nix-env");

    install_command.arg("-iA");

    for attribute in attributes {
        install_command.arg(attribute);
    }

    install_command.output();

    // TODO: Stuff with output.
    Ok(())
}

// ------------------------------------------------------------------------------------------------

fn main() {
    let mut query = String::new();

    {
        let mut cli = ArgumentParser::new();
        cli.set_description("Search and install Nix packages.");
        cli.refer(&mut query)
            .add_argument("query", Store, "Package to search for.")
            .required();
        cli.parse_args_or_exit();
    }

    all_packages(NullCache)
    .map(|packages| {
        packages.into_iter()
        .filter(|package| package.query(&query))
        .enumerate()
        .inspect(|&(ref ix, ref package)| display_package(ix, package))
        .collect::<Vec<(usize, Package)>>()
    })
    .map(|options: Vec<(usize, Package)>| {
        if options.is_empty() {
            println!("Zero packages match that query.");
            std::process::exit(0);
        }

        options
    })
    .and_then(|options| {
        let to_install_indices = try!(request_package_indices_to_install(options.len()));

        let to_install_indices = to_install_indices.unwrap_or_else(|| {
            println!("User requested not to install any packages.");
            std::process::exit(0);
        });

        Ok(
            to_install_indices
            .into_iter()
            .map(|ix| options[ix].1.attribute.clone())
            .collect::<Vec<Attribute>>()
        )
    })
    .and_then(install_attributes)
    .unwrap_or_else(|nox_error| {
        match nox_error {
            NoxError::NoHomeDir => { println!("Error: Cannot find home directory."); },
            NoxError::Io(io_error) => { println!("IO Error: {:?}", io_error); },
            NoxError::NixEnvFailed(_, _) => { println!("Error: Calling nix-env failed."); },
            NoxError::NixEnvParseError => { println!("nix-env result oddly formated."); }
            _ => { println!("Unknown error occured!"); }
        }

        std::process::exit(1);
    });

    print!("Packages to install: ");


}

//     if results:
//         def parse_input(inp):
//             if inp[0] == 's':
//                 action = 'shell'
//                 inp = inp[1:]
//             else:
//                 action = 'install'
//             packages = [results[int(i) - 1] for i in inp.split()]
//             return action, packages

//         action, packages = click.prompt('Packages to install',
//                                         value_proc=parse_input)
//         attributes = [p.attribute for p in packages]
//         if action == 'install':
//             subprocess.check_call(['nix-env', '-iA'] + attributes)
//         elif action == 'shell':
//             attributes = [a[len('nixpkgs.'):] for a in attributes]
//             subprocess.check_call(['nix-shell', '-p'] + attributes)