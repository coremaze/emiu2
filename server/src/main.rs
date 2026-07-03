use emiu2_relay::RelayServer;

const DEFAULT_PORT: u16 = 5885;

fn main() {
    let mut port = DEFAULT_PORT;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--port" | "-p" => {
                let value = args.next().unwrap_or_default();
                port = match value.parse() {
                    Ok(port) => port,
                    Err(_) => {
                        eprintln!("Invalid port: {value:?}");
                        std::process::exit(2);
                    }
                };
            }
            "--help" | "-h" => {
                println!("Usage: emiu2-relay [--port <port>]  (default port {DEFAULT_PORT})");
                return;
            }
            other => {
                eprintln!("Unknown argument: {other:?} (try --help)");
                std::process::exit(2);
            }
        }
    }

    let server = match RelayServer::bind(("0.0.0.0", port)) {
        Ok(server) => server,
        Err(why) => {
            eprintln!("Could not bind port {port}: {why}");
            std::process::exit(1);
        }
    };
    server.run();
}
