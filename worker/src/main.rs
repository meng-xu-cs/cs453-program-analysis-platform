use log::error;

use cs453_pap_worker::tools;

fn main() {
    // setup logging
    stderrlog::new()
        .module(module_path!())
        .timestamp(stderrlog::Timestamp::Second)
        .verbosity(stderrlog::LogLevelNum::Info)
        .init()
        .expect("unable to setup logging");

    // handle the command line
    match tools::provision(false) {
        Ok(()) => (),
        Err(err) => {
            error!("failed to provision tools: {}", err);
        }
    }
}
