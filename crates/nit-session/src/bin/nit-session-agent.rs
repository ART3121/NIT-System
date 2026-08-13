fn main() {
    if let Err(error) = nit_session::SessionAgent::serve(&nit_session::default_endpoint()) {
        eprintln!("NIT Session Agent failed: {error:#}");
        std::process::exit(1);
    }
}
