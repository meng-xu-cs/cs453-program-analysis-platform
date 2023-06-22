use std::collections::BTreeSet;
use std::fs::File;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::{fs, io};

use anyhow::{bail, Result};
use sha3::{Digest, Sha3_256};

/// Uniquely identifies a packet
#[derive(Ord, PartialOrd, Eq, PartialEq)]
pub struct Packet {
    hash: String,
}

/// Registry of packets
pub struct Registry {
    root: RwLock<PathBuf>,
}

impl Registry {
    /// Create a new registry
    pub fn new(root: PathBuf) -> Self {
        Self {
            root: RwLock::new(root),
        }
    }

    /// Register a packet from a filesystem path
    pub fn register<P: AsRef<Path>>(&self, src: P) -> Result<(Packet, bool)> {
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

        // check for duplication atomically
        let locked = self.root.write().expect("lock");
        let root = locked.join(&hash);
        let existed = root.exists();
        let err_opt = if existed {
            None
        } else {
            fs::create_dir_all(&root).err()
        };
        drop(locked);
        if let Some(err) = err_opt {
            bail!(err);
        }

        // prepare the packet in registry
        if !existed {
            // copy to destination
            copy_dir_recursive(base, &root)?;

            // overwrite the interface file
            let content = include_bytes!("../asset/interface.h");
            fs::write(root.join("interface.h"), content)?;

            // create an output directory
            let output = root.join("output");
            fs::create_dir_all(output)?;
        }

        // complete the return package
        Ok((Packet { hash }, existed))
    }

    /// Prepare the workspace
    pub fn mk_dockerized_packet(
        &self,
        pkt: &Packet,
        name: &str,
        mnt: &str,
    ) -> Result<DockedPacket> {
        let locked = self.root.read().expect("lock");
        let host_base = locked.join(&pkt.hash);
        drop(locked);

        // prepare the host workspace path
        let host_output = host_base.join("output").join(name);
        if host_output.exists() {
            if host_output.is_file() {
                fs::remove_file(&host_output)?;
            } else {
                fs::remove_dir_all(&host_output)?;
            }
        }
        fs::create_dir(&host_output)?;

        // prepare the dockerized packet
        let dock_base = Path::new(mnt);

        let host_input = host_base.join("input");
        let dock_input = dock_base.join("input");
        let mut dock_input_cases = BTreeSet::new();
        for item in fs::read_dir(host_input)? {
            let item = item?;
            dock_input_cases.insert(path_to_str(dock_input.join(item.file_name())));
        }

        let host_crash = host_base.join("crash");
        let dock_crash = dock_base.join("crash");
        let mut dock_crash_cases = BTreeSet::new();
        for item in fs::read_dir(host_crash)? {
            let item = item?;
            dock_crash_cases.insert(path_to_str(dock_input.join(item.file_name())));
        }

        let dock_packet = DockedPacket {
            host_base,
            host_output,
            path_base: mnt.to_string(),
            path_program: path_to_str(dock_base.join("main.c")),
            path_input: path_to_str(dock_input),
            path_input_cases: dock_input_cases,
            path_crash: path_to_str(dock_crash),
            path_crash_cases: dock_crash_cases,
            path_output: path_to_str(dock_base.join("output").join(name)),
        };

        // done with the construction
        Ok(dock_packet)
    }
}

/// Dockerized packet
pub struct DockedPacket {
    pub host_base: PathBuf,
    pub host_output: PathBuf,
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
        path_to_str(Path::new(&self.path_output).join(seg))
    }
}

// Utilities functions

fn path_to_str(path: PathBuf) -> String {
    path.into_os_string().into_string().expect("ascii path")
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
