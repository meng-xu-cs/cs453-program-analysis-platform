use std::env;

use log::error;

use cs453_pap_worker::process;

fn main() {
    // setup logging
    stderrlog::new()
        .module(module_path!())
        .timestamp(stderrlog::Timestamp::Second)
        .verbosity(stderrlog::LogLevelNum::Info)
        .init()
        .expect("unable to setup logging");

    // check if we need to force provision
    let force = match env::var_os("FORCE_PROVISION") {
        None => false,
        Some(v) => v.to_str().map_or(false, |v| v == "1"),
    };

    // handle the command line
    match process::provision(force) {
        Ok(()) => (),
        Err(err) => {
            error!("failed to provision tools: {}", err);
        }
    }
}
