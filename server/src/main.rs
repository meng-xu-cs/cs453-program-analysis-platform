use std::fmt::{Display, Formatter};
use std::io::Cursor;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::{fs, thread};

use log::{error, info};
use once_cell::sync::Lazy;
use tempdir::TempDir;
use tiny_http::{Method, Request, Response};
use zip::ZipArchive;

use cs453_pap_worker::packet::Registry;
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

/// Port number for the server
const PORT: u16 = 8000;

/// Number of server instances
const NUMBER_OF_SERVERS: usize = 4;

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

/// Allowed actions
enum Action {
    Trial,
    Submit,
}

impl Display for Action {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Trial => write!(f, "trial"),
            Self::Submit => write!(f, "submit"),
        }
    }
}

/// Entrypoint of the execution
fn entrypoint(req: &mut Request) -> Response<Cursor<Vec<u8>>> {
    // shortcut for help
    match req.method() {
        Method::Post => (),
        _ => {
            return make_sanity_error("invalid method");
        }
    }

    // parse command
    let url = req.url();
    let action = match url {
        "/trial" => Action::Trial,
        "/submit" => Action::Submit,
        _ => {
            return make_sanity_error("invalid URI path");
        }
    };

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
    info!("processing request: {}", action);
    match analyze(&REGISTRY, dir.path()) {
        Ok(_) => (),
        Err(err) => {
            info!("invalid packet: {}", err);
            return make_sanity_error(format!("failed to schedule analysis: {}", err));
        }
    }

    // clean-up
    match dir.close() {
        Ok(_) => (),
        Err(err) => {
            return make_server_error(format!("unable to clear the temporary directory: {}", err));
        }
    }

    make_ok("everything is good")
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

    // initialize everything
    info!("number of packets found: {}", REGISTRY.count());

    // bind address
    let addr = SocketAddr::from(([127, 0, 0, 1], PORT));
    let server = tiny_http::Server::http(addr).expect("server binding");
    info!("socket bounded");

    let pointer = Arc::new(server);
    let mut handles = Vec::with_capacity(NUMBER_OF_SERVERS);
    for i in 0..NUMBER_OF_SERVERS {
        let instance = Arc::clone(&pointer);
        let handle = thread::spawn(move || loop {
            // wait for request
            let mut request = match instance.recv() {
                Ok(req) => req,
                Err(err) => {
                    error!(
                        "[instance {}] unexpected error when receiving requests: {}",
                        i, err
                    );
                    continue;
                }
            };

            // process it
            let response = entrypoint(&mut request);

            // send back response
            match request.respond(response) {
                Ok(_) => (),
                Err(err) => {
                    error!(
                        "[instance {}] unexpected error when sending response: {}",
                        i, err
                    );
                }
            }
        });
        handles.push(handle);
    }

    // wait for termination
    for (i, h) in handles.into_iter().enumerate() {
        match h.join() {
            Ok(_) => {
                info!("[instance {}] terminated", i);
            }
            Err(err) => {
                error!("[instance {}] unexpected error on join: {:?}", i, err);
            }
        }
    }
}
