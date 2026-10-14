use connect_connector_api::conformance;
use prism_fixture::{BrokenMode, Prism};
#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 || args[1] != "--contract-report" {
        eprintln!("usage: prism-fixture --contract-report <fresh-state-file> [--broken-mode context|authentication|uncertainty]");
        std::process::exit(2);
    }
    if std::path::Path::new(&args[2]).exists() {
        eprintln!("fresh state file required");
        std::process::exit(2);
    }
    let mode = match args.get(4).map(String::as_str) {
        None => BrokenMode::None,
        Some("context") => BrokenMode::Context,
        Some("authentication") => BrokenMode::Authentication,
        Some("uncertainty") => BrokenMode::Uncertainty,
        _ => {
            eprintln!("unknown broken mode");
            std::process::exit(2);
        }
    };
    let mut fixture = Prism::open_with_mode(&args[2], mode).expect("offline fixture construction");
    let report = conformance::run(&mut fixture).await;
    println!(
        "{}",
        serde_json::to_string(&report).expect("serialize report")
    );
    if !report.passed() {
        std::process::exit(1);
    }
}
