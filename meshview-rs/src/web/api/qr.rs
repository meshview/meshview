// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! A node as a contact the Meshtastic apps can import, for its QR code.
//!
//! The contact is built and encoded exactly as the Python meshview built it,
//! because the apps read it: a `SharedContact` with the node's number and a
//! `User` made of what the database knows of it, and its public key from its
//! latest node info packet.

use super::error::{ApiError, ErrorBody};
use super::params::parse_db_int_any_base;
use crate::state::AppState;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use meshview_proto::meshtastic::config::device_config::Role;
use meshview_proto::meshtastic::mesh_packet::PayloadVariant;
use meshview_proto::meshtastic::{HardwareModel, MeshPacket, SharedContact, User};
use meshview_proto::wire;
use prost::Message;
use serde::Serialize;
use std::collections::HashSet;
use utoipa::ToSchema;

/// The length of a Meshtastic public key.
const PUBLIC_KEY_LEN: usize = 32;

const CONTACT_URL: &str = "https://meshtastic.org/v/#";

#[derive(Debug, Serialize, ToSchema)]
pub struct Contact {
    pub node_id: i64,
    pub long_name: Option<String>,
    pub short_name: Option<String>,
    pub role: Option<String>,
    /// The public key, in base64, from the node's latest node info.
    pub public_key: Option<String>,
    /// Whether its node info packets disagree about its key. One of them may
    /// be forged, so then no contact is offered.
    pub public_key_conflict: bool,
    /// The contact, as the link the Meshtastic apps import; none without a key
    /// or with conflicting ones.
    pub meshtastic_url: Option<String>,
}

/// `!` and the number in eight hex digits at least, as Python's `!{n:08x}`
/// writes it, the sign included for a negative one.
fn user_id(node_id: i64) -> String {
    if node_id < 0 {
        format!("!-{:07x}", node_id.unsigned_abs())
    } else {
        format!("!{node_id:08x}")
    }
}

/// Node contact
///
/// The node as a contact the Meshtastic apps can import: a link to make a QR
/// code of. There is none when the node has sent no public key, which current
/// apps refuse a contact without and older firmware would erase the key it has
/// for, nor when its packets disagree about its key.
#[utoipa::path(
    get,
    path = "/node/{node_id}/qr",
    tag = "nodes",
    params(("node_id" = i64, Path, description = "The node's number, in decimal or 0x hex.")),
    responses(
        (status = 200, description = "The contact", body = Contact),
        (status = 400, description = "`node_id` is not a number", body = ErrorBody),
        (status = 404, description = "There is no such node", body = ErrorBody),
        (status = 500, description = "It could not be made", body = ErrorBody),
    )
)]
pub async fn qr(
    State(state): State<AppState>,
    Path(node_id): Path<String>,
) -> Result<Json<Contact>, ApiError> {
    let node_id =
        parse_db_int_any_base(&node_id).ok_or_else(|| ApiError::bad_request("Invalid node_id"))?;
    let db = state.db()?;
    let failed = |e: anyhow::Error| ApiError::internal(format!("Failed to generate URL: {e}"), &e);
    let node = db
        .node(node_id)
        .await
        .map_err(failed)?
        .ok_or_else(|| ApiError::not_found("Node not found"))?;

    let mut user = User {
        id: user_id(node_id),
        long_name: node.long_name.clone().unwrap_or_default(),
        short_name: node.short_name.clone().unwrap_or_default(),
        ..Default::default()
    };
    if let Some(model) = node
        .hw_model
        .as_deref()
        .and_then(|name| HardwareModel::from_str_name(&name.to_uppercase()))
    {
        user.hw_model = model as i32;
    }
    if let Some(role) = node
        .role
        .as_deref()
        .and_then(|name| Role::from_str_name(&name.to_uppercase()))
    {
        user.role = role as i32;
    }

    // The database keeps no keys: they are in the node info packets the node
    // sent. Any packet counts, read as node info whatever it says it carries,
    // as in the Python version.
    let mut latest: Option<User> = None;
    let mut keys = HashSet::new();
    for payload in db.nodeinfo_payloads(node_id).await.map_err(failed)? {
        let Some(payload) = payload.filter(|payload| !payload.is_empty()) else {
            continue;
        };
        let Some(packet) = wire::decode::<MeshPacket>("meshtastic.MeshPacket", &payload) else {
            continue;
        };
        let inner = match &packet.payload_variant {
            Some(PayloadVariant::Decoded(data)) => data.payload.as_slice(),
            _ => &[],
        };
        let Some(info) = wire::decode::<User>("meshtastic.User", inner) else {
            continue;
        };
        if info.public_key.len() == PUBLIC_KEY_LEN {
            keys.insert(info.public_key.clone());
        }
        latest.get_or_insert(info);
    }
    if let Some(latest) = &latest {
        if latest.public_key.len() == PUBLIC_KEY_LEN {
            user.public_key = latest.public_key.clone();
        }
        user.is_licensed = latest.is_licensed;
        user.is_unmessagable = latest.is_unmessagable;
    }

    // Importing a contact replaces a key the radio has pinned, so when the
    // packets disagree on the key, neither is offered.
    let public_key_conflict = keys.len() > 1;
    let meshtastic_url = if !user.public_key.is_empty() && !public_key_conflict {
        let node_num = u32::try_from(node_id).map_err(|_| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to generate URL: Value out of range: {node_id}"),
            )
        })?;
        let contact = SharedContact {
            node_num,
            user: Some(user.clone()),
            should_ignore: false,
            manually_verified: false,
        };
        Some(format!(
            "{CONTACT_URL}{}",
            URL_SAFE_NO_PAD.encode(contact.encode_to_vec())
        ))
    } else {
        None
    };

    Ok(Json(Contact {
        node_id,
        long_name: node.long_name,
        short_name: node.short_name,
        role: node.role,
        public_key: (!user.public_key.is_empty()).then(|| STANDARD.encode(&user.public_key)),
        public_key_conflict,
        meshtastic_url,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_ids_are_written_as_python_writes_them() {
        assert_eq!(user_id(0xabcd), "!0000abcd");
        assert_eq!(user_id(0xdead_beef), "!deadbeef");
        assert_eq!(user_id(-5), "!-0000005");
    }
}
