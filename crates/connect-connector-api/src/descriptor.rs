//! Declarative route and context contracts, registered by trusted composition.
/// Target validation independent of any provider name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddressKind {
    Opaque,
    Uuid,
    E164Pair,
}

/// Durable dispatch semantics. Changing a registered contract requires restart
/// and explicit qualification; adapters cannot bypass conversation authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Descriptor {
    pub name: &'static str,
    pub address: AddressKind,
    /// Supply an authorized context projection and serialize dependent turns.
    pub conversational: bool,
    /// Apply reviewed enrollment, freshness and STOP policy before dispatch.
    pub sms: bool,
}
impl Descriptor {
    /// An adapter without context or SMS semantics receives only the current body.
    pub const fn one_way(name: &'static str) -> Self {
        Self {
            name,
            address: AddressKind::Opaque,
            conversational: false,
            sms: false,
        }
    }
}
/// Compatibility descriptors for existing durable routes. These are declarative
/// defaults, not an adapter registry or evidence that credentials are configured.
pub const TELNYX: Descriptor = Descriptor {
    name: "telnyx",
    address: AddressKind::E164Pair,
    conversational: false,
    sms: true,
};
/// Existing Vapi chat route contract; media composition remains a separate API.
pub const VAPI: Descriptor = Descriptor {
    name: "vapi",
    address: AddressKind::Uuid,
    conversational: true,
    sms: false,
};
