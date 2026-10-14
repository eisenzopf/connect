use connect_connector_api::conformance;
use connect_connector_template::LocalProvider;
#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some("--contract-report") {
        eprintln!("usage: connect-connector-template --contract-report <isolated-state-file>");
        std::process::exit(2);
    }
    let path = args.next().expect("isolated fixture state file required");
    let mut provider = LocalProvider::open_fixture(path.into()).expect("open isolated fixture");
    let report = conformance::run(&mut provider).await;
    println!(
        "{}",
        serde_json::to_string_pretty(&report).expect("fixture report")
    );
    if !report.passed() {
        std::process::exit(1);
    }
}
