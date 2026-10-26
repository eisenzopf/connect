#[path = "../conformance/fixture.rs"]
mod fixture;
#[tokio::main]
async fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args != ["--contract-report"] && args != ["--contract-report", "--broken-custody"] {
        eprintln!("usage: connect-media-connector-template --contract-report [--broken-custody]");
        std::process::exit(2);
    }
    let report = if args.len() == 2 {
        connect_media_conformance::run::<fixture::BrokenCustody>().await
    } else {
        connect_media_conformance::run::<fixture::Template>().await
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&report).expect("fixture report")
    );
    if !report.passed() {
        std::process::exit(1);
    }
}
