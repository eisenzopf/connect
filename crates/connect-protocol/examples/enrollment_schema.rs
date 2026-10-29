//! Reference-only aggregate of the actual enrollment DTOs, never a wire body.
#[cfg(feature = "schema")]
fn main() {
    use connect_protocol::enrollment::*;
    use schemars::{generate::SchemaSettings, JsonSchema};
    #[allow(dead_code)]
    #[derive(JsonSchema)]
    struct EnrollmentContract {
        request: EnrollmentRequest,
        claim: EnrollmentClaim,
        revoke: EnrollmentRevokeRequest,
        offer_response: EnrollmentResponse<EnrollmentOffer>,
        credential_response: EnrollmentResponse<EnrollmentCredential>,
        revoke_response: EnrollmentResponse<EnrollmentRevocation>,
        hosting_rejection: connect_protocol::canonical_http::Rejection,
    }
    let schema = SchemaSettings::draft2020_12()
        .for_deserialize()
        .into_generator()
        .into_root_schema_for::<EnrollmentContract>();
    println!("{}", serde_json::to_string_pretty(&schema).expect("schema"));
}
#[cfg(not(feature = "schema"))]
fn main() {
    panic!("Enrollment schema generation requires --features schema");
}
