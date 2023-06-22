use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs::File;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::{fs, io};

use anyhow::{bail, Result};
use sha3::{Digest, Sha3_256};

/// Represents a packet
pub struct Packet {
    hash: String,
    pub base: PathBuf,
    pub program: PathBuf,
    pub input_tests: BTreeMap<OsString, PathBuf>,
    pub input_crash: BTreeMap<OsString, PathBuf>,
    output: PathBuf,
}

impl Packet {
    /// Build a packet from a filesystem path
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self> {
        let base = path.as_ref().canonicalize()?;
        if !base.is_dir() {
            bail!("not a directory");
        }

        // scan for directory content
        for item in fs::read_dir(&base)? {
            let name = item?.file_name();
            match name.to_str() {
                Some("main.c") | Some("interface.h") | Some("input") | Some("crash")
                | Some("README") | Some("README.md") | Some("README.txt") | Some("README.pdf") => {}
                Some(n) => bail!("unrecognized item: {}", n),
                None => bail!("unrecognized item: {:?}", name),
            }
        }

        // hasher
        let mut hasher = Sha3_256::new();

        // code
        let program = base.join("main.c");
        if !(program.exists() && program.is_file()) {
            bail!("main.c is missing");
        }
        let size = program.metadata()?.size();
        if size > 256 * 1024 {
            bail!("main.c is too big");
        }

        hasher.update(b"code");
        let mut file = File::open(&program)?;
        io::copy(&mut file, &mut hasher)?;

        // input tests
        let mut input_tests = BTreeMap::new();
        let path_tests = base.join("input");
        if !(path_tests.exists() && path_tests.is_dir()) {
            bail!("input/ is missing");
        }
        for item in fs::read_dir(&path_tests)? {
            let item = item?;
            let item_name = item.file_name();
            if !item.file_type()?.is_file() {
                bail!("input/{:?} is invalid", item_name);
            }
            let item_path = item.path();
            let size = item_path.metadata()?.size();
            if size > 1024 {
                bail!("input/{:?} is too big", item_name);
            }
            input_tests
                .insert(item_name, item_path)
                .expect("unique file names");
        }
        for (item_name, item_path) in &input_tests {
            hasher.update(b"input");
            hasher.update(item_name.as_bytes());
            let mut file = File::open(item_path)?;
            io::copy(&mut file, &mut hasher)?;
        }

        // crash tests
        let mut input_crash = BTreeMap::new();
        let path_tests = base.join("crash");
        if !(path_tests.exists() && path_tests.is_dir()) {
            bail!("crash/ is missing");
        }
        for item in fs::read_dir(&path_tests)? {
            let item = item?;
            let item_name = item.file_name();
            if !item.file_type()?.is_file() {
                bail!("crash/{:?} is invalid", item_name);
            }
            let item_path = item.path();
            let size = item_path.metadata()?.size();
            if size > 1024 {
                bail!("crash/{:?} is too big", item_name);
            }
            input_crash
                .insert(item_name, item_path)
                .expect("unique file names");
        }
        for (item_name, item_path) in &input_crash {
            hasher.update(b"crash");
            hasher.update(item_name.as_bytes());
            let mut file = File::open(item_path)?;
            io::copy(&mut file, &mut hasher)?;
        }

        // derive the hash
        let digest = hasher.finalize();
        let hash = hex::encode(digest);

        // overwrite the interface file
        let content = include_bytes!("../asset/interface.h");
        fs::write(base.join("interface.h"), content)?;

        // create output directory
        let output = base.join("output");
        fs::create_dir_all(&output)?;

        // done with basic sanity checking
        Ok(Self {
            hash,
            base,
            program,
            input_tests,
            input_crash,
            output,
        })
    }

    /// Prepare the workspace
    pub fn mk_wks(&mut self, name: &str) -> Result<PathBuf> {
        let path = self.output.join(name);
        if path.exists() {
            if path.is_file() {
                fs::remove_file(&path)?;
            } else {
                fs::remove_dir_all(&path)?;
            }
        }
        fs::create_dir(&path)?;
        Ok(path)
    }
}
