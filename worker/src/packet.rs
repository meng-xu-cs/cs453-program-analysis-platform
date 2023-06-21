use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use anyhow::{bail, Result};

/// Represents a packet
pub struct Packet {
    code: PathBuf,
    input_tests: PathBuf,
    input_crash: PathBuf,
    output: PathBuf,
}

impl Packet {
    /// Build a packet from a filesystem path
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self> {
        let base = path.as_ref();
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

        // code
        let code = base.join("main.c");
        if !(code.exists() && code.is_file()) {
            bail!("main.c is missing");
        }
        let size = code.metadata()?.size();
        if size > 256 * 1024 {
            bail!("main.c is too big");
        }

        // input tests
        let input_tests = base.join("input");
        if !(input_tests.exists() && input_tests.is_dir()) {
            bail!("input/ is missing");
        }
        for item in fs::read_dir(&input_tests)? {
            let item = item?;
            if !item.file_type()?.is_file() {
                bail!("input/{:?} is invalid", item.file_name());
            }
            let item_path = item.path();
            let size = item_path.metadata()?.size();
            if size > 1024 {
                bail!("input/{:?} is too big", item.file_name());
            }
        }

        // crash tests
        let input_crash = base.join("crash");
        if !(input_crash.exists() && input_crash.is_dir()) {
            bail!("crash/ is missing");
        }
        for item in fs::read_dir(&input_crash)? {
            let item = item?;
            if !item.file_type()?.is_file() {
                bail!("crash/{:?} is invalid", item.file_name());
            }
            let item_path = item.path();
            let size = item_path.metadata()?.size();
            if size > 1024 {
                bail!("crash/{:?} is too big", item.file_name());
            }
        }

        // overwrite the interface file
        let content = include_bytes!("../asset/interface.h");
        fs::write(base.join("interface.h"), content)?;

        // create output directory
        let output = base.join("output");
        fs::create_dir_all(&output)?;

        // done with basic sanity checking
        Ok(Self {
            code,
            input_tests,
            input_crash,
            output,
        })
    }
}
