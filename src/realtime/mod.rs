//! Voice signaling and subscription control. Legacy modes use JSON SDP on
//! `POST /v1/realtime/sessions`; GPT-Live uses `/v1/realtime/live` with a trusted
//! server-held sideband. The caller owns WebRTC audio and its lifecycle.
mod client;
#[cfg(feature = "server")]
mod live;
#[cfg(feature = "server")]
pub(crate) mod live_routes;
#[cfg(feature = "server")]
pub use live::LiveOffer;
#[cfg(feature = "server")]
pub(crate) mod routes;

pub use client::{
    RealtimeAnswer, RealtimeClient, RealtimeClientConfig, RealtimeError, RealtimeMode,
    RealtimeOffer, DEFAULT_UPSTREAM, DEFAULT_VOICE, MAX_SDP_BYTES,
};
