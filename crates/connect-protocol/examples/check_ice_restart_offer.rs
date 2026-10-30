//! Local comparison helper for retained private SDP; never prints its contents.
use std::{fs::File, io::Read};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let paths: Vec<_> = std::env::args_os().skip(1).collect();
    if paths.len() != 2 {
        return Err("Expected original and proposed private SDP paths".into());
    }
    let mut offers = Vec::new();
    for path in paths {
        let mut text = String::new();
        File::open(path)?
            .take(connect_protocol::webrtc_ice_restart::MAX_SDP_BYTES as u64 + 1)
            .read_to_string(&mut text)?;
        offers.push(text);
    }
    connect_protocol::webrtc_ice_restart::validate_offer_change(&offers[0], &offers[1])?;
    println!("Restart-only offer comparison passed; ICE readiness is unqualified");
    Ok(())
}
