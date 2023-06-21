use std::collections::BTreeSet;
use std::env;
use std::future::Future;
use std::path::Path;

use anyhow::{anyhow, bail, Result};
use docker_api::models::ImageBuildChunk;
use docker_api::opts::{ContainerRemoveOpts, ImageBuildOpts, ImageRemoveOpts};
use docker_api::{Container, Docker, Image};
use futures_util::StreamExt;
use log::{debug, error, info};
use tokio::runtime;

const UNIX_SOCKET: &str = "/var/run/docker.sock";
const DEFAULT_PLATFORM: &str = "linux/amd64";

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
    docker: Docker,
}

impl Dock {
    /// Create a new Docker manager
    pub fn new() -> Self {
        Self {
            docker: Docker::unix(UNIX_SOCKET),
        }
    }

    /// Query an image by its tag
    pub fn get_image(&self, tag: &str) -> Result<Option<Image>> {
        let tag_latest = format!("{}:latest", tag);

        let mut candidates = BTreeSet::new();
        for image in wait_for(self.docker.images().list(&Default::default()))? {
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
                let image = self.docker.images().get(id);
                let name = image.name();
                debug!("[docker] found image \"{}\" for tag \"{}\"", name, tag);
                Ok(Some(image))
            }
        }
    }

    /// Delete an image together with its associated containers
    pub fn del_image(&mut self, image: &Image) -> Result<()> {
        let name = image.name().to_string();

        // delete associated containers first
        let mut candidates = vec![];
        for container in wait_for(self.docker.containers().list(&Default::default()))? {
            if let Some(cid) = container.id {
                match (container.image, container.image_id) {
                    (None, None) => (),
                    (Some(id), None) | (None, Some(id)) => {
                        if id == name {
                            candidates.push(cid);
                        }
                    }
                    (Some(id1), Some(id2)) => {
                        if id1 == name || id2 == name {
                            candidates.push(cid);
                        }
                    }
                }
            }
        }
        for cid in candidates {
            self.del_container(&self.docker.containers().get(cid))?;
        }

        // delete the image
        let opts = ImageRemoveOpts::builder().force(true).build();
        wait_for(image.remove(&opts))?;
        debug!("[docker] image \"{}\" deleted", name);
        Ok(())
    }

    /// Delete a container, stop it first if still running
    pub fn del_container(&mut self, container: &Container) -> Result<()> {
        let id = container.id();
        let opts = ContainerRemoveOpts::builder()
            .force(true)
            .volumes(true)
            .link(true)
            .build();
        wait_for(container.remove(&opts))?;
        debug!("[docker] container \"{}\" deleted", id);
        Ok(())
    }

    /// Build an image from a Dockerfile
    async fn _build_async(&mut self, path: &Path, tag: &str) -> Result<()> {
        // build options
        let opts = ImageBuildOpts::builder(path)
            .tag(tag)
            .nocahe(true)
            .platform(DEFAULT_PLATFORM)
            .build();

        // run the build
        let images = self.docker.images();
        let mut stream = images.build(&opts);
        while let Some(frame) = stream.next().await {
            let frame = frame?;
            match frame {
                ImageBuildChunk::Update { stream } => {
                    print!("{}", stream);
                }
                ImageBuildChunk::Error {
                    error,
                    error_detail,
                } => {
                    error!("[docker] {}: {}", error, error_detail.message);
                }
                ImageBuildChunk::Digest { aux } => {
                    debug!("[docker] digest {}", aux.id);
                }
                ImageBuildChunk::PullStatus { status, .. } => {
                    debug!("[docker] status {}", status);
                }
            }
        }

        // image successfully built
        Ok(())
    }

    /// Build an image from a Dockerfile, delete or reuse previous image depending on flag
    pub fn build(&mut self, path: &Path, tag: &str, force: bool) -> Result<Image> {
        // preparation
        match self.get_image(tag)? {
            None => (),
            Some(image) => {
                if force {
                    info!("[docker] deleting image \"{}\" before building", tag);
                    self.del_image(&image)?;
                } else {
                    info!("[docker] image \"{}\" already exists", tag);
                    return Ok(image);
                }
            }
        }

        // actual image building
        wait_for(self._build_async(path, tag))?;

        // confirm that we actually have the image
        self.get_image(tag)?
            .ok_or_else(|| anyhow!("unable to locate image \"{}\"", tag))
    }
}

impl Default for Dock {
    fn default() -> Self {
        Self::new()
    }
}
