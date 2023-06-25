use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::io;
use std::io::{Read, Seek, Write};
use std::path::Path;
use std::time::{Duration, SystemTime};

use anyhow::{bail, Result};
use bollard::container::{
    Config, CreateContainerOptions, ListContainersOptions, LogOutput, LogsOptions,
    RemoveContainerOptions,
};
use bollard::errors::Error::{DockerContainerWaitError, IOError};
use bollard::image::{BuildImageOptions, CommitContainerOptions, RemoveImageOptions};
use bollard::models::{HostConfig, ResourcesUlimits};
use bollard::Docker;
use futures_util::StreamExt;
use log::{debug, error, info};
use memfile::MemFile;
use tar::Builder;
use tokio::runtime;

/// Default timeout for sandboxed execution
const DEFAULT_SANDBOX_TIMEOUT: Duration = Duration::from_secs(60);

struct ImageID(String);
struct ContainerID(String);

/// Exit status of the execution
pub enum ExitStatus {
    Success,
    Failure,
    Timeout,
}

/// Utility for waiting for async actions
fn wait_for<F: Future>(future: F) -> F::Output {
    let rt = runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(future)
}

/// An encapsulation of the Docker command line
pub struct Dock {
    name: String,
    docker: Docker,
}

impl Dock {
    /// Create a new Docker manager
    pub fn new(name: String) -> Result<Self> {
        Ok(Self {
            name,
            docker: Docker::connect_with_socket_defaults()?,
        })
    }

    /// Create a duplicate
    pub fn duplicate(&self) -> Result<Self> {
        Self::new(format!("{}-sideline", self.name))
    }

    /// Query an image by its tag
    fn get_image(&self, tag: &str) -> Result<Option<ImageID>> {
        let tag_latest = format!("{}:latest", tag);

        let mut candidates = BTreeSet::new();
        for image in wait_for(self.docker.list_images::<String>(None))? {
            if image.repo_tags.contains(&tag_latest) {
                candidates.insert(image.id);
            }
        }

        if candidates.len() > 1 {
            bail!("more than one image with tag {}", tag);
        }
        match candidates.into_iter().next() {
            None => Ok(None),
            Some(id) => {
                debug!("[docker] found image \"{}\" for tag \"{}\"", id, tag);
                Ok(Some(ImageID(id)))
            }
        }
    }

    /// Delete an image together with its associated containers
    fn del_image(&self, id: &ImageID) -> Result<()> {
        // delete associated containers first
        let mut candidates = vec![];
        let opts = ListContainersOptions::<String> {
            all: true,
            ..Default::default()
        };
        for container in wait_for(self.docker.list_containers(Some(opts)))? {
            if let Some(cid) = container.id {
                match (container.image, container.image_id) {
                    (None, None) => (),
                    (Some(id0), None) | (None, Some(id0)) => {
                        if id0 == id.0 {
                            candidates.push(cid);
                        }
                    }
                    (Some(id1), Some(id2)) => {
                        if id1 == id.0 || id2 == id.0 {
                            candidates.push(cid);
                        }
                    }
                }
            }
        }
        for cid in candidates {
            self.del_container(&ContainerID(cid))?;
        }

        // delete the image
        let opts = RemoveImageOptions {
            force: true,
            ..Default::default()
        };
        wait_for(self.docker.remove_image(&id.0, Some(opts), None))?;
        debug!("[docker] image \"{}\" deleted", id.0);
        Ok(())
    }

    /// Query a container by its name
    fn get_container(&self, name: &str) -> Result<Option<ContainerID>> {
        let mut candidates = BTreeSet::new();
        let opts = ListContainersOptions::<String> {
            all: true,
            ..Default::default()
        };
        for container in wait_for(self.docker.list_containers(Some(opts)))? {
            match container.id {
                None => (),
                Some(id) => {
                    if container
                        .names
                        .map_or(false, |names| names.into_iter().any(|n| n == name))
                    {
                        candidates.insert(id);
                    }
                }
            }
        }

        if candidates.len() > 1 {
            bail!("more than one container with name {}", name);
        }
        match candidates.into_iter().next() {
            None => Ok(None),
            Some(id) => {
                debug!("[docker] found container \"{}\" with name \"{}\"", id, name);
                Ok(Some(ContainerID(id)))
            }
        }
    }

    /// Delete a container, stop it first if still running
    fn del_container(&self, id: &ContainerID) -> Result<()> {
        let opts = RemoveContainerOptions {
            force: true,
            v: true,
            ..Default::default()
        };
        wait_for(self.docker.remove_container(&id.0, Some(opts)))?;
        debug!("[docker] container \"{}\" deleted", id.0);
        Ok(())
    }

    /// Build an image from a Dockerfile
    async fn _build_async(&self, path: &Path, tag: &str) -> Result<()> {
        // context tarball
        let tx = MemFile::create_default(tag)?;

        let mut tarball = Builder::new(tx);
        tarball.follow_symlinks(false);
        tarball.append_dir_all(".", path)?;
        tarball.finish()?;
        let mut tx = tarball.into_inner()?;

        tx.rewind()?;
        let mut data = vec![];
        tx.read_to_end(&mut data)?;
        drop(tx);

        // build options
        let opts = BuildImageOptions {
            t: tag,
            nocache: true,
            ..Default::default()
        };

        // run the build
        let mut stream = self.docker.build_image(opts, None, Some(data.into()));
        while let Some(frame) = stream.next().await {
            let frame = frame?;
            if let Some(msg) = frame.stream {
                print!("{}", msg);
            }
            if let Some(msg) = frame.status {
                info!("[docker] {}", msg);
            }
            if let Some(msg) = frame.error {
                error!("[docker] {}", msg);
            }
            if let Some(msg) = frame.error_detail {
                error!(
                    "[docker] {} - {}",
                    msg.code.unwrap_or(0),
                    msg.message.unwrap_or_default()
                );
            }
            if let Some(msg) = frame.progress {
                debug!("[docker] {}", msg);
            }
        }

        // image successfully built
        Ok(())
    }

    /// Build an image from a Dockerfile, delete or reuse previous image depending on flag
    pub fn build(&self, path: &Path, tag: &str, force: bool) -> Result<()> {
        // preparation
        match self.get_image(tag)? {
            None => (),
            Some(id) => {
                if force {
                    info!("[docker] deleting image \"{}\" before building", tag);
                    self.del_image(&id)?;
                } else {
                    info!("[docker] image \"{}\" already exists", tag);
                    return Ok(());
                }
            }
        }

        // actual image building
        wait_for(self._build_async(path, tag))?;

        // confirm that we actually have the image
        match self.get_image(tag)? {
            None => {
                bail!("unable to locate image \"{}\"", tag);
            }
            Some(id) => {
                info!("[docker] image \"{}\" built successfully: {}", tag, id.0);
            }
        }
        Ok(())
    }

    /// Run a container
    async fn _exec_async(
        &self,
        id: &ContainerID,
        console: bool,
        timeout: Option<Duration>,
        start_time: SystemTime,
    ) -> Result<ExitStatus> {
        // follow output
        let opts = LogsOptions {
            follow: true,
            stdout: true,
            stderr: true,
            ..Default::default()
        };
        let mut stream = self.docker.logs::<String>(&id.0, Some(opts));
        while let Some(frame) = stream.next().await {
            // TODO: this is a hack to workaround bytes remaining problem
            let frame = match frame {
                Ok(frame) => frame,
                Err(IOError { err }) if err.to_string() == "bytes remaining on stream" => {
                    continue;
                }
                Err(e) => bail!(e),
            };
            // handle the frame
            match frame {
                LogOutput::StdIn { message } => {
                    bail!(
                        "unexpected message to stdin: {}",
                        String::from_utf8(message.to_vec())
                            .unwrap_or_else(|_| "<not-utf8-string>".to_string())
                    );
                }
                LogOutput::StdOut { message } => {
                    if console {
                        io::stdout().write_all(&message)?;
                    }
                }
                LogOutput::StdErr { message } => {
                    if console {
                        io::stderr().write_all(&message)?;
                    }
                }
                LogOutput::Console { message } => {
                    if console {
                        io::stdout().write_all(&message)?;
                    }
                }
            }

            // check timeout
            match timeout.as_ref() {
                None => (),
                Some(duration) => {
                    let elapsed = SystemTime::now().duration_since(start_time)?;
                    if &elapsed > duration {
                        return Ok(ExitStatus::Timeout);
                    }
                }
            }
        }

        // wait for termination
        let mut status = None;
        let mut stream = self.docker.wait_container::<String>(&id.0, None);
        while let Some(frame) = stream.next().await {
            let exit_code = match frame {
                Ok(resp) => {
                    if let Some(msg) = resp.error {
                        error!(
                            "[docker] {}",
                            msg.message.unwrap_or_else(|| "<none>".to_string())
                        );
                    }
                    resp.status_code
                }
                Err(DockerContainerWaitError { error, code }) => {
                    if !error.is_empty() {
                        bail!("unexpected wait error: {}", error);
                    }
                    code
                }
                Err(err) => {
                    bail!(err);
                }
            };

            if status.is_none() {
                status = Some(exit_code);
            } else {
                bail!("conflicting status code");
            }
        }

        // simplify the exit status
        let exit_status = match status {
            None => {
                bail!("not receiving a status code");
            }
            Some(0) => ExitStatus::Success,
            Some(_) => ExitStatus::Failure,
        };
        Ok(exit_status)
    }

    /// Run a container based on an image file
    #[allow(clippy::too_many_arguments)]
    fn _run(
        &self,
        tag: &str,
        name: Option<String>,
        cmd: Vec<String>,
        net: bool,
        tty: bool,
        console: bool,
        timeout: Option<Duration>,
        binding: BTreeMap<&Path, String>,
        workdir: Option<String>,
    ) -> Result<ExitStatus> {
        // check container existence
        let ephemeral_name = format!("{}-ephemeral-{}", tag, self.name);
        if let Some(id) = self.get_container(&ephemeral_name)? {
            bail!(
                "docker container \"{}\" already exists with name \"{}\"",
                id.0,
                ephemeral_name
            );
        }

        // check image existence
        let image_id = match self.get_image(tag)? {
            None => {
                bail!("docker image tagged \"{}\" does not exist", tag);
            }
            Some(id) => id,
        };

        // build the configs
        let opts = CreateContainerOptions {
            name: ephemeral_name,
            ..Default::default()
        };
        let cfgs = Config {
            attach_stdin: Some(false),
            attach_stdout: Some(true),
            attach_stderr: Some(true),
            tty: Some(tty),
            network_disabled: Some(!net),
            image: Some(image_id.0),
            working_dir: workdir,
            cmd: Some(cmd),
            host_config: Some(HostConfig {
                ulimits: Some(vec![ResourcesUlimits {
                    name: Some("stack".to_string()),
                    soft: Some(-1),
                    hard: Some(-1),
                }]),
                binds: Some(
                    binding
                        .into_iter()
                        .map(|(h, c)| format!("{}:{}", h.to_str().unwrap(), c))
                        .collect(),
                ),
                ..Default::default()
            }),
            ..Default::default()
        };

        // create the container
        let result = wait_for(self.docker.create_container(Some(opts), cfgs))?;
        if !result.warnings.is_empty() {
            for msg in result.warnings {
                error!("{}", msg);
            }
            self.del_container(&ContainerID(result.id))?;
            bail!("unexpected warning in docker container creation");
        }
        let container_id = ContainerID(result.id);

        // start the container
        match wait_for(self.docker.start_container::<String>(&container_id.0, None)) {
            Ok(()) => (),
            Err(err) => {
                self.del_container(&container_id)?;
                bail!(err);
            }
        }
        let timestamp = SystemTime::now();

        // wait for the termination of the container
        let exit_status =
            match wait_for(self._exec_async(&container_id, console, timeout, timestamp)) {
                Ok(r) => r,
                Err(err) => {
                    self.del_container(&container_id)?;
                    bail!(err);
                }
            };

        // decide if we need to commit the container
        if let Some(commit) = name {
            match exit_status {
                ExitStatus::Success => {
                    // commit the container
                    match wait_for(self.docker.commit_container(
                        CommitContainerOptions {
                            container: container_id.0.clone(),
                            repo: commit,
                            ..Default::default()
                        },
                        Config::<String>::default(),
                    )) {
                        Ok(_) => (),
                        Err(err) => {
                            self.del_container(&container_id)?;
                            bail!(err);
                        }
                    }
                }
                _ => {
                    self.del_container(&container_id)?;
                    bail!("aborting commit due to execution failure");
                }
            }
        }

        // remove the container
        self.del_container(&container_id)?;

        // return the exit status of this execution
        Ok(exit_status)
    }

    /// Run a container based on an image file and commit it back
    #[allow(clippy::too_many_arguments)]
    pub fn commit(
        &self,
        tag: &str,
        name: &str,
        cmd: Vec<String>,
        net: bool,
        tty: bool,
        binding: BTreeMap<&Path, String>,
        workdir: Option<String>,
        force: bool,
    ) -> Result<()> {
        // preparation
        match self.get_image(name)? {
            None => (),
            Some(id) => {
                if force {
                    info!("[docker] deleting image \"{}\" before building", tag);
                    self.del_image(&id)?;
                } else {
                    info!("[docker] image \"{}\" already exists", tag);
                    return Ok(());
                }
            }
        }

        // incremental build
        self._run(
            tag,
            Some(name.to_string()),
            cmd,
            net,
            tty,
            true,
            None,
            binding,
            workdir,
        )?;

        // done
        Ok(())
    }

    /// Invoke a simple command on a container and discard it
    #[allow(clippy::too_many_arguments)]
    fn invoke(
        &self,
        tag: &str,
        cmd: Vec<String>,
        net: bool,
        tty: bool,
        console: bool,
        timeout: Option<Duration>,
        binding: BTreeMap<&Path, String>,
        workdir: Option<String>,
    ) -> Result<ExitStatus> {
        self._run(tag, None, cmd, net, tty, console, timeout, binding, workdir)
    }

    /// Invoke a simple command on a container in sandboxed environment and discard it
    pub fn sandbox(
        &self,
        tag: &str,
        cmd: Vec<String>,
        timeout: Option<Duration>,
        binding: BTreeMap<&Path, String>,
        workdir: Option<String>,
    ) -> Result<ExitStatus> {
        self.invoke(
            tag,
            cmd,
            false,
            true,
            false,
            Some(timeout.unwrap_or(DEFAULT_SANDBOX_TIMEOUT)),
            binding,
            workdir,
        )
    }
}
