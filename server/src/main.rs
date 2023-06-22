use std::convert::Infallible;
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::{fs, io};

use http_body_util::{BodyExt, Full};
use hyper::body::{Bytes, Incoming};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use log::{error, info};
use once_cell::sync::Lazy;
use tempdir::TempDir;
use tokio::net::TcpListener;
use zip::ZipArchive;

use cs453_pap_worker::packet::Registry;
use cs453_pap_worker::process::analyze;

/// Absolute path to the `data` directory
static REGISTRY: Lazy<Registry> = Lazy::new(|| {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    assert!(path.pop());
    path.push("data");
    fs::create_dir_all(&path).expect("unable to initialize data directory");

    // construct the registry
    Registry::new(path)
});

/// Port number for the server
pub const PORT: u16 = 8000;

/// Produce an error response related to user making a bad request
fn make_sanity_error(reason: &str) -> Response<Full<Bytes>> {
    let mut r = Response::new(Full::new(Bytes::from(format!("[error] {}", reason))));
    *r.status_mut() = StatusCode::BAD_REQUEST;
    r
}

/// Produce an error response related to server internal status
fn make_server_error(reason: &str) -> Response<Full<Bytes>> {
    let mut r = Response::new(Full::new(Bytes::from(format!(
        "[internal error] {}",
        reason
    ))));
    *r.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
    r
}

/// Produce a normal reply
fn make_ok(message: &str) -> Response<Full<Bytes>> {
    let mut r = Response::new(Full::new(Bytes::from(format!("{}\n", message))));
    *r.status_mut() = StatusCode::OK;
    r
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
async fn entrypoint(req: Request<Incoming>) -> Result<Response<Full<Bytes>>, Infallible> {
    // shortcut for help
    if req.method() != "POST" {
        return Ok(make_sanity_error("only POST request allowed"));
    }

    // parse command
    let uri = req.uri();
    if uri.query().is_some() {
        return Ok(make_sanity_error("queries not allowed in URI path"));
    }
    let action = match uri.path() {
        "/trial" => Action::Trial,
        "/submit" => Action::Submit,
        _ => {
            return Ok(make_sanity_error("invalid URI path"));
        }
    };

    // parse body
    let body = match req.into_body().collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(err) => {
            return Ok(make_sanity_error(&format!(
                "unable to read POST body: {}",
                err
            )));
        }
    };

    let mut reader = io::Cursor::new(body);
    let mut zip = match ZipArchive::new(&mut reader) {
        Ok(ar) => ar,
        Err(err) => {
            return Ok(make_sanity_error(&format!(
                "unable to parse POST body into a ZIP archive: {}",
                err
            )));
        }
    };

    // process the packet
    let dir = match TempDir::new("pap") {
        Ok(d) => d,
        Err(err) => {
            return Ok(make_server_error(&format!(
                "unable to create temporary directory: {}",
                err
            )));
        }
    };
    match zip.extract(dir.path()) {
        Ok(_) => (),
        Err(err) => {
            return Ok(make_server_error(&format!(
                "unable to extract the ZIP archive into the temporary directory: {}",
                err
            )));
        }
    }

    // act on the request
    info!("processing request: {}", action);
    match analyze(&REGISTRY, dir.path()) {
        Ok(_) => (),
        Err(err) => {
            return Ok(make_sanity_error(&format!(
                "failed to analyze package: {}",
                err
            )));
        }
    }

    // clean-up
    match dir.close() {
        Ok(_) => (),
        Err(err) => {
            return Ok(make_server_error(&format!(
                "unable to clean-up the temporary directory: {}",
                err
            )));
        }
    }

    let response = make_ok("everything is good");
    Ok(response)
}

/// Start server
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    // setup logging
    stderrlog::new()
        .module(module_path!())
        .timestamp(stderrlog::Timestamp::Second)
        .verbosity(stderrlog::LogLevelNum::Info)
        .init()
        .expect("unable to setup logging");

    // initialize everything

    // bind address
    let addr = SocketAddr::from(([127, 0, 0, 1], PORT));
    let listener = TcpListener::bind(addr).await?;
    info!("server started");

    // start server
    loop {
        let (stream, _) = listener.accept().await?;
        // Spawn a tokio task to serve multiple connections concurrently
        tokio::task::spawn(async move {
            if let Err(err) = http1::Builder::new()
                .serve_connection(stream, service_fn(entrypoint))
                .await
            {
                error!("Error serving connection: {:?}", err);
            }
        });
    }
}
