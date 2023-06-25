use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::{fs, io};

use anyhow::{anyhow, bail, Result};
use sha3::{Digest, Sha3_256};

use crate::process::AnalysisResult;

/// Marker for unexpected internal error
const MARKER_ERROR: &str = "error";

/// Marker for completed analysis
const MARKER_RESULT: &str = "result.json";

/// Uniquely identifies a packet
#[derive(Ord, PartialOrd, Eq, PartialEq, Clone)]
pub struct Packet {
    hash: String,
}

impl Packet {
    /// Get the unique ID for this packet
    pub fn id(&self) -> &str {
        &self.hash
    }
}

/// Packet analysis status
#[derive(Copy, Clone)]
pub enum Status {
    Received,
    Error,
    Completed,
}

/// Registry of packets
pub struct Registry {
    root: RwLock<PathBuf>,
    queue: RwLock<Vec<Packet>>,
    packets: RwLock<BTreeMap<Packet, Status>>,
}

impl Registry {
    /// Create a new registry
    pub fn new(root: PathBuf) -> Result<Self> {
        // scan across the root directory
        if !root.exists() || !root.is_dir() {
            bail!("invalid root path for registry");
        }

        let mut packets = BTreeMap::new();
        for item in fs::read_dir(&root)? {
            let item = item?;
            let hash = item
                .file_name()
                .into_string()
                .map_err(|_| anyhow!("invalid package hash in registry"))?;

            // check packet status
            let path = item.path();
            let packet = Packet { hash };

            // on completed
            let path_result = path.join(MARKER_RESULT);
            if path_result.exists() {
                packets.insert(packet, Status::Completed);
                continue;
            }

            // on error (re-queue the packet for analysis)
            let path_error = path.join(MARKER_ERROR);
            if path_error.exists() {
                fs::remove_file(&path_error)?;
            }

            // on received with error cleared
            packets.insert(packet, Status::Received);
        }

        Ok(Self {
            root: RwLock::new(root),
            queue: RwLock::new(vec![]),
            packets: RwLock::new(packets),
        })
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
                            fs::remove_file(item.path())?;
                            continue;
                        }
                        if n.starts_with("output") && ty.is_dir() {
                            fs::remove_dir_all(item.path())?;
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

    /// Report a snapshot of all packets the registry accumulates
    pub fn snapshot(&self) -> BTreeMap<Packet, Status> {
        let locked = self.packets.read().expect("lock");
        locked.clone()
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
            dock_crash_cases.insert(path_to_str(dock_crash.join(item.file_name())));
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

    /// Add the packet to queue
    pub fn queue(&self, packet: Packet) {
        let mut locked = self.queue.write().expect("lock");
        locked.push(packet.clone());
        drop(locked);

        let mut locked = self.packets.write().expect("lock");
        locked.insert(packet, Status::Received);
        drop(locked);
    }

    /// Save analysis result
    pub fn save_result(&self, packet: Packet, result: AnalysisResult) -> Result<()> {
        // save to filesystem
        let locked = self.root.read().expect("lock");
        let path = locked.join(&packet.hash).join(MARKER_RESULT);
        drop(locked);
        serde_json::to_writer_pretty(File::create(path)?, &result)?;

        // mark availability
        let mut locked = self.packets.write().expect("lock");
        locked.insert(packet.clone(), Status::Completed);
        drop(locked);

        // remove it from queue
        let mut locked = self.queue.write().expect("lock");
        locked.retain(|p| p != &packet);
        drop(locked);

        // done
        Ok(())
    }

    /// Save analysis error
    pub fn save_error(&self, packet: Packet, error: String) -> Result<()> {
        // save to filesystem
        let locked = self.root.read().expect("lock");
        let path = locked.join(&packet.hash).join(MARKER_ERROR);
        drop(locked);
        fs::write(path, error)?;

        // mark availability
        let mut locked = self.packets.write().expect("lock");
        locked.insert(packet.clone(), Status::Error);
        drop(locked);

        // remove it from queue
        let mut locked = self.queue.write().expect("lock");
        locked.retain(|p| p != &packet);
        drop(locked);

        // done
        Ok(())
    }

    /// Load analysis result or error
    pub fn load_packet_status(&self, hash: String) -> Result<Option<String>> {
        let packet = Packet { hash };

        // check availability
        let locked = self.packets.read().expect("lock");
        let status = locked.get(&packet).cloned();
        drop(locked);

        let message = match status {
            None => None,
            Some(Status::Received) => {
                let locked = self.queue.read().expect("lock");
                let index = locked.iter().position(|p| p == &packet);
                drop(locked);
                match index {
                    None => {
                        bail!("unable to find packet in queue");
                    }
                    Some(pos) => Some(format!("queued at position {}", pos)),
                }
            }
            Some(Status::Completed) => {
                let locked = self.root.read().expect("lock");
                let path = locked.join(&packet.hash).join(MARKER_RESULT);
                drop(locked);
                if !path.exists() {
                    bail!("unable to find analysis result file");
                }
                let result: AnalysisResult = serde_json::from_reader(File::open(path)?)?;
                Some(serde_json::to_string(&result)?)
            }
            Some(Status::Error) => {
                let locked = self.root.read().expect("lock");
                let path = locked.join(&packet.hash).join(MARKER_ERROR);
                drop(locked);
                if !path.exists() {
                    bail!("unable to find analysis error file");
                }
                Some(fs::read_to_string(&path)?)
            }
        };

        Ok(message)
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
    pub fn wks_path(&self, seg: &str) -> (PathBuf, String) {
        (
            self.host_output.join(seg),
            path_to_str(Path::new(&self.path_output).join(seg)),
        )
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
