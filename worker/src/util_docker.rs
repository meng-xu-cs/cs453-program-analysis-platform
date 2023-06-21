use std::collections::BTreeSet;
use std::future::Future;
use std::io::{Read, Seek};
use std::path::Path;

use anyhow::{bail, Result};
use bollard::container::{ListContainersOptions, RemoveContainerOptions};
use bollard::image::{BuildImageOptions, ListImagesOptions, RemoveImageOptions};
use bollard::Docker;
use futures_util::StreamExt;
use log::{debug, error, info};
use memfile::MemFile;
use tar::Builder;
use tokio::runtime;

struct ImageID(String);
struct ContainerID(String);

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
    pub fn new() -> Result<Self> {
        Ok(Self {
            docker: Docker::connect_with_socket_defaults()?,
        })
    }

    /// Query an image by its tag
    fn get_image(&self, tag: &str) -> Result<Option<ImageID>> {
        let tag_latest = format!("{}:latest", tag);

        let mut candidates = BTreeSet::new();
        let opts = ListImagesOptions::<String>::default();
        for image in wait_for(self.docker.list_images(Some(opts)))? {
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
    fn del_image(&mut self, id: &ImageID) -> Result<()> {
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

    /// Delete a container, stop it first if still running
    fn del_container(&mut self, id: &ContainerID) -> Result<()> {
        let opts = RemoveContainerOptions {
            force: true,
            v: true,
            link: true,
        };
        wait_for(self.docker.remove_container(&id.0, Some(opts)))?;
        debug!("[docker] container \"{}\" deleted", id.0);
        Ok(())
    }

    /// Build an image from a Dockerfile
    async fn _build_async(&mut self, path: &Path, tag: &str) -> Result<()> {
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
    pub fn build(&mut self, path: &Path, tag: &str, force: bool) -> Result<()> {
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
}
