#![allow(clippy::unwrap_used)]
mod authority;
mod fixture;
mod lifecycle;
mod local_access;
mod lock_expiry;
mod transactions;

mod grants;

mod cancellation_budget;
mod claims;
mod delegated_cancel;
#[cfg(feature = "test-support")]
mod effect_crashes;
mod effect_helpers;
mod effect_migration;
mod effect_provider_boundaries;
mod effect_providers;
mod effect_questions;
mod effect_recovery;
mod effects;
mod grant_boundaries;
mod grant_operations;
mod media_authority;
mod media_context;
mod media_fixture;
mod media_migration;
mod media_recovery;
mod media_setup;
mod media_setup_binding;
mod native_media_migration;
mod native_reply;
mod native_resource_boundaries;
mod native_resource_fixture;
mod native_resources;
mod provider_setup;
mod recall;
mod recall_authority;
mod recall_helpers;
mod recall_sources;
mod replies;
mod reply_races;
mod retained_inspection;
mod sms_fixture;
mod sms_policy;
