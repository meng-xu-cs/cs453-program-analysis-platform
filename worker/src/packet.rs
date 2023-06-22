use std::collections::BTreeSet;
use std::fs::File;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::{fs, io};

use anyhow::{bail, Result};
use sha3::{Digest, Sha3_256};

/// Represents a packet
pub struct Packet {
    hash: String,
    pub root: PathBuf,
    pub num_tests: usize,
    pub num_crash: usize,
    output: PathBuf,
}

impl Packet {
    /// Build a packet from a filesystem path
    pub fn new<SRC: AsRef<Path>, DST: AsRef<Path>>(src: SRC, dst: DST) -> Result<Self> {
        let tmp = src.as_ref().canonicalize()?;
        if !tmp.is_dir() {
            bail!("not a directory");
        }

        // probe packet base
        let mut probe = None;
        let mut count = 0;
        for item in fs::read_dir(&tmp)? {
            let item = item?;
            count += 1;
            if count == 2 {
                break;
            }

            let ty = item.file_type()?;
            if ty.is_dir() {
                probe = Some(item.path());
            }
        }
        let base = match (probe, count) {
            (Some(p), 1) => p,
            _ => tmp,
        };

        // scan for directory content
        for item in fs::read_dir(&base)? {
            let item = item?;
            let name = item.file_name();
            let ty = item.file_type()?;
            match name.to_str() {
                None => bail!("unrecognized item: {:?}", name),
                Some(n) => match n {
                    "main.c" | "interface.h" | "input" | "crash" => (),
                    _ => {
                        if n.starts_with("README") && ty.is_file() {
                            continue;
                        }
                        if n.starts_with("output") && ty.is_dir() {
                            continue;
                        }
                        bail!("unrecognized item: {}", n);
                    }
                },
            }
        }

        // hasher
        let mut hasher = Sha3_256::new();

        // program
        let program = base.join("main.c");
        if !(program.exists() && program.is_file()) {
            bail!("main.c is missing");
        }
        let size = program.metadata()?.size();
        if size > 256 * 1024 {
            bail!("main.c is too big");
        }

        hasher.update(b"program");
        let mut file = File::open(&program)?;
        io::copy(&mut file, &mut hasher)?;

        // input tests
        let mut input_tests = vec![];
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
            input_tests.push(item_path);
        }

        let num_tests = input_tests.len();
        for (i, item_path) in input_tests.into_iter().enumerate() {
            // hash the input
            hasher.update(b"input");
            hasher.update(i.to_ne_bytes());
            let mut file = File::open(&item_path)?;
            io::copy(&mut file, &mut hasher)?;
            drop(file);
            // rename the test file
            fs::rename(&item_path, item_path.with_file_name(i.to_string()))?;
        }

        // crash tests
        let mut input_crash = vec![];
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
            input_crash.push(item_path);
        }

        let num_crash = input_crash.len();
        for (i, item_path) in input_crash.into_iter().enumerate() {
            // hash the input
            hasher.update(b"crash");
            hasher.update(i.to_ne_bytes());
            let mut file = File::open(&item_path)?;
            io::copy(&mut file, &mut hasher)?;
            drop(file);
            // rename the test file
            fs::rename(&item_path, item_path.with_file_name(i.to_string()))?;
        }

        // derive the hash
        let digest = hasher.finalize();
        let hash = hex::encode(digest);

        // copy to destination
        let root = dst.as_ref().join(&hash);
        copy_dir_recursive(base, &root)?;

        // overwrite the interface file
        let content = include_bytes!("../asset/interface.h");
        fs::write(root.join("interface.h"), content)?;

        // create output directory
        let output = root.join("output");
        fs::create_dir_all(&output)?;

        // done with basic sanity checking
        Ok(Self {
            hash,
            root,
            num_tests,
            num_crash,
            output,
        })
    }

    /// Prepare the workspace
    pub fn mk_docked_wks(&mut self, name: &str, mnt: &str) -> Result<(PathBuf, DockedPacket)> {
        // prepare the host workspace path
        let host_wks = self.output.join(name);
        if host_wks.exists() {
            if host_wks.is_file() {
                fs::remove_file(&host_wks)?;
            } else {
                fs::remove_dir_all(&host_wks)?;
            }
        }
        fs::create_dir(&host_wks)?;

        // prepare the dockerized packet
        let dock_base = Path::new(mnt);
        let dock_packet = DockedPacket {
            path_base: mnt.to_string(),
            path_program: path_to_str1(dock_base, "main.c"),
            path_input: path_to_str1(dock_base, "input"),
            path_input_cases: (0..self.num_tests)
                .map(|i| path_to_str2(dock_base, "input", &i.to_string()))
                .collect(),
            path_crash: path_to_str1(dock_base, "crash"),
            path_crash_cases: (0..self.num_crash)
                .map(|i| path_to_str2(dock_base, "crash", &i.to_string()))
                .collect(),
            path_output: path_to_str2(dock_base, "output", name),
        };

        // done with the construction
        Ok((host_wks, dock_packet))
    }
}

/// Dockerized packet
pub struct DockedPacket {
    pub path_base: String,
    pub path_program: String,
    pub path_input: String,
    pub path_input_cases: BTreeSet<String>,
    pub path_crash: String,
    pub path_crash_cases: BTreeSet<String>,
    pub path_output: String,
}

impl DockedPacket {
    /// Derive a workspace path
    pub fn wks_path(&self, seg: &str) -> String {
        path_to_str1(Path::new(&self.path_output), seg)
    }
}

// Utilities functions

fn path_to_str1(base: &Path, seg: &str) -> String {
    base.join(seg).into_os_string().into_string().unwrap()
}

fn path_to_str2(base: &Path, seg1: &str, seg2: &str) -> String {
    base.join(seg1)
        .join(seg2)
        .into_os_string()
        .into_string()
        .unwrap()
}

fn copy_dir_recursive(src: impl AsRef<Path>, dst: impl AsRef<Path>) -> io::Result<()> {
    fs::create_dir_all(&dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        if ty.is_dir() {
            copy_dir_recursive(entry.path(), dst.as_ref().join(entry.file_name()))?;
        } else {
            fs::copy(entry.path(), dst.as_ref().join(entry.file_name()))?;
        }
    }
    Ok(())
}
