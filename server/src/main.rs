use std::io::Cursor;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::{fs, thread};

use crossbeam_channel::Sender;
use log::{error, info};
use once_cell::sync::Lazy;
use tempdir::TempDir;
use tiny_http::{Method, Request, Response};
use zip::ZipArchive;

use cs453_pap_worker::packet::{Packet, Registry};
use cs453_pap_worker::process::analyze;

/// Absolute path to the `data` directory
static REGISTRY: Lazy<Registry> = Lazy::new(|| {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    assert!(path.pop());
    path.push("data");
    fs::create_dir_all(&path).expect("unable to initialize the data directory");

    // construct the registry
    Registry::new(path).expect("unable to initialize the registry")
});

/// Hostname for the server
const HOST: &str = "localhost";

/// Port number for the server
const PORT: u16 = 8000;

/// Number of server instances
const NUMBER_OF_SERVERS: usize = 2;

/// Number of worker instances
const NUMBER_OF_WORKERS: usize = 8;

/// Produce an error response related to user making a bad request
fn make_sanity_error<S: AsRef<str>>(reason: S) -> Response<Cursor<Vec<u8>>> {
    Response::from_string(format!("[error] {}", reason.as_ref())).with_status_code(400)
}

/// Produce an error response related to server internal status
fn make_server_error<S: AsRef<str>>(reason: S) -> Response<Cursor<Vec<u8>>> {
    Response::from_string(format!("[internal error] {}", reason.as_ref())).with_status_code(500)
}

/// Produce a normal reply
fn make_ok<S: AsRef<str>>(reason: S) -> Response<Cursor<Vec<u8>>> {
    Response::from_string(format!("{}\n", reason.as_ref())).with_status_code(200)
}

/// Entrypoint of the execution
fn entrypoint(req: &mut Request, channel: &Sender<Packet>) -> Response<Cursor<Vec<u8>>> {
    // shortcut for help
    match req.method() {
        Method::Post => (),
        _ => {
            return make_sanity_error("invalid method");
        }
    }

    // parse command
    if req.url() != "/submit" {
        return make_sanity_error("invalid URI path");
    }

    // parse body
    let mut body = vec![];
    match req.as_reader().read_to_end(&mut body) {
        Ok(_) => (),
        Err(err) => {
            return make_sanity_error(format!("unable to read POST body: {}", err));
        }
    }

    let mut reader = Cursor::new(body);
    let mut zip = match ZipArchive::new(&mut reader) {
        Ok(ar) => ar,
        Err(err) => {
            return make_sanity_error(format!(
                "unable to parse POST body into a ZIP archive: {}",
                err
            ));
        }
    };

    // process the packet
    let dir = match TempDir::new("pap") {
        Ok(d) => d,
        Err(err) => {
            return make_server_error(format!("unable to create temporary directory: {}", err));
        }
    };
    match zip.extract(dir.path()) {
        Ok(_) => (),
        Err(err) => {
            return make_server_error(format!(
                "unable to extract the ZIP archive into the temporary directory: {}",
                err
            ));
        }
    }

    // act on the request
    info!("processing request");
    let response = match REGISTRY.register(dir.path()) {
        Ok((packet, existed)) => {
            // prepare the message first
            let head = if existed {
                "has been submitted before"
            } else {
                "is scheduled for analysis"
            };
            let msg = format!(
                "the package {}, you can check its status or result at https://{}:{}/{}",
                head,
                HOST,
                PORT,
                packet.id()
            );
            info!("packet {}: {}", head, packet.id());

            // send the packet to channel
            match channel.send(packet) {
                Ok(_) => make_ok(msg),
                Err(err) => make_server_error(format!("failed to schedule analysis: {}", err)),
            }
        }
        Err(err) => {
            info!("invalid packet: {}", err);
            make_sanity_error(format!("package does not seem to be well-formed: {}", err))
        }
    };

    // clean-up
    match dir.close() {
        Ok(_) => (),
        Err(err) => {
            return make_server_error(format!("unable to clear the temporary directory: {}", err));
        }
    }

    response
}

/// Start server
fn main() {
    // setup logging
    stderrlog::new()
        .module(module_path!())
        .timestamp(stderrlog::Timestamp::Second)
        .verbosity(stderrlog::LogLevelNum::Info)
        .init()
        .expect("unable to setup logging");

    // initialize
    info!("number of packets found: {}", REGISTRY.count());

    // setup channel
    let (channel_send, channel_recv) = crossbeam_channel::unbounded::<Packet>();

    // spawn workers
    let mut worker_handles = Vec::with_capacity(NUMBER_OF_WORKERS);
    for i in 0..NUMBER_OF_WORKERS {
        let c_recv = channel_recv.clone();
        let handle = thread::spawn(move || loop {
            // wait for packet
            let packet = match c_recv.recv() {
                Ok(pkt) => pkt,
                Err(err) => {
                    error!(
                        "[worker {}] unexpected error when receiving packets: {}",
                        i, err
                    );
                    continue;
                }
            };
            info!("[worker {}] received packet: {}", i, packet.id());

            // process the packet
            match analyze(&REGISTRY, &packet) {
                Ok(_) => {
                    info!("[worker {}] packet analyzed: {}", i, packet.id());
                }
                Err(err) => {
                    error!(
                        "[worker {}] unexpected error when analyzing packet: {}",
                        i, err
                    );
                }
            }
        });
        worker_handles.push(handle);
    }

    // bind address
    let addr = SocketAddr::from(([127, 0, 0, 1], PORT));
    let server = tiny_http::Server::http(addr).expect("server binding");
    info!("socket bounded");

    // spawn servers
    let pointer = Arc::new(server);
    let mut server_handles = Vec::with_capacity(NUMBER_OF_SERVERS);
    for i in 0..NUMBER_OF_SERVERS {
        let instance = Arc::clone(&pointer);
        let c_send = channel_send.clone();
        let handle = thread::spawn(move || loop {
            // wait for request
            let mut request = match instance.recv() {
                Ok(req) => req,
                Err(err) => {
                    error!(
                        "[server {}] unexpected error when receiving requests: {}",
                        i, err
                    );
                    continue;
                }
            };

            // process it
            let response = entrypoint(&mut request, &c_send);

            // send back response
            match request.respond(response) {
                Ok(_) => (),
                Err(err) => {
                    error!(
                        "[server {}] unexpected error when sending response: {}",
                        i, err
                    );
                }
            }
        });
        server_handles.push(handle);
    }

    // wait for termination
    for (i, h) in server_handles.into_iter().enumerate() {
        match h.join() {
            Ok(_) => {
                info!("[server {}] terminated", i);
            }
            Err(err) => {
                error!("[server {}] unexpected error on join: {:?}", i, err);
            }
        }
    }
    for (i, h) in worker_handles.into_iter().enumerate() {
        match h.join() {
            Ok(_) => {
                info!("[worker {}] terminated", i);
            }
            Err(err) => {
                error!("[worker {}] unexpected error on join: {:?}", i, err);
            }
        }
    }
}
