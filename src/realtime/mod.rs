//! Realtime voice: SDP offer/answer signaling with ChatGPT's private voice
//! service and the `POST /v1/realtime/sessions` route. Audio and events flow
//! directly between the caller's WebRTC peer and the service.
mod client;
#[cfg(feature = "server")]
pub(crate) mod routes;

pub use client::{
    RealtimeAnswer, RealtimeClient, RealtimeClientConfig, RealtimeError, RealtimeMode,
    RealtimeOffer, DEFAULT_UPSTREAM, DEFAULT_VOICE, MAX_SDP_BYTES,
};
