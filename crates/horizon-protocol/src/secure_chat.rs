//! Secure chat as vanilla 26.3's server keeps it: a player's profile key,
//! validated against the services' keys (ProfilePublicKey,
//! RemoteChatSession), the message chain each player signs
//! (SignedMessageChain), what a signature covers (PlayerChatMessage,
//! SignedMessageLink, SignedMessageBody), the window of messages a client
//! acknowledges (LastSeenMessagesValidator) and the cache a connection packs
//! signatures against (MessageSignatureCache).
use crate::rsa::{RsaPublicKey, SignatureHash};
use std::collections::{HashSet, VecDeque};

/// MessageSignature.BYTES.
pub const SIGNATURE_BYTES: usize = 256;
/// A message signature (MessageSignature).
pub type Signature = [u8; SIGNATURE_BYTES];
/// LastSeenMessages.LAST_SEEN_MESSAGES_MAX_LENGTH: the window a client acknowledges.
pub const LAST_SEEN: usize = 20;
/// PlayerChatMessage.MESSAGE_EXPIRES_AFTER_SERVER, in milliseconds.
pub const EXPIRES_AFTER_SERVER: i64 = 5 * 60 * 1000;

/// A signature from its packet bytes (always 256 on the wire).
pub fn signature(bytes: &[u8]) -> Option<Signature> {
    bytes.try_into().ok()
}

/// ProfilePublicKey.Data: the player's chat key (X.509 DER), when it
/// expires (epoch milliseconds), and the services' signature over both.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileKey {
    pub expires_at: i64,
    pub key: Vec<u8>,
    pub signature: Vec<u8>,
}

impl ProfileKey {
    /// Data.signedPayload: the profile id, the expiry and the key.
    fn signed_payload(&self, profile: u128) -> Vec<u8> {
        let mut out = Vec::with_capacity(24 + self.key.len());
        out.extend_from_slice(&profile.to_be_bytes());
        out.extend_from_slice(&self.expires_at.to_be_bytes());
        out.extend_from_slice(&self.key);
        out
    }
    /// Data.validateSignature with the services' profile-key keys
    /// (SignatureValidator.from(keySet, PROFILE_KEY): any of them, SHA1withRSA).
    pub fn signed_by(&self, services: &[RsaPublicKey], profile: u128) -> bool {
        let payload = self.signed_payload(profile);
        services
            .iter()
            .any(|k| k.verify(SignatureHash::Sha1, &payload, &self.signature))
    }
    /// Data.hasExpired.
    pub fn has_expired(&self, now: i64) -> bool {
        self.expires_at < now
    }
}

/// A validated RemoteChatSession: its id and the player's profile key.
#[derive(Clone, Debug)]
pub struct ChatSession {
    pub id: u128,
    pub profile_key: ProfileKey,
    key: RsaPublicKey,
}

impl ChatSession {
    /// RemoteChatSession.Data.validate (ProfilePublicKey.createValidated);
    /// the error is the disconnect message's translation key.
    pub fn validate(
        id: u128,
        profile_key: ProfileKey,
        services: &[RsaPublicKey],
        profile: u128,
    ) -> Result<Self, &'static str> {
        const INVALID: &str = "multiplayer.disconnect.invalid_public_key_signature";
        let key = RsaPublicKey::from_der(&profile_key.key).map_err(|_| INVALID)?;
        if !profile_key.signed_by(services, profile) {
            return Err(INVALID);
        }
        Ok(Self {
            id,
            profile_key,
            key,
        })
    }
    /// RemoteChatSession.Data, as ClientboundPlayerInfoUpdatePacket's
    /// INITIALIZE_CHAT carries it.
    pub fn data(&self) -> (u128, i64, Vec<u8>, Vec<u8>) {
        let k = &self.profile_key;
        (self.id, k.expires_at, k.key.clone(), k.signature.clone())
    }
}

/// SignedMessageBody: what a player signed besides the chain link.
#[derive(Clone, Copy, Debug)]
pub struct Body<'a> {
    pub content: &'a str,
    /// Epoch milliseconds.
    pub timestamp: i64,
    pub salt: i64,
    pub last_seen: &'a [Signature],
}

/// PlayerChatMessage.updateSignature: the version, the link (sender,
/// session, index) and the body (salt, time in seconds, content, last seen).
pub fn signed_payload(sender: u128, session: u128, index: i32, body: &Body) -> Vec<u8> {
    let content = body.content.as_bytes();
    let mut out = Vec::with_capacity(64 + content.len() + body.last_seen.len() * SIGNATURE_BYTES);
    out.extend_from_slice(&1i32.to_be_bytes());
    out.extend_from_slice(&sender.to_be_bytes());
    out.extend_from_slice(&session.to_be_bytes());
    out.extend_from_slice(&index.to_be_bytes());
    out.extend_from_slice(&body.salt.to_be_bytes());
    out.extend_from_slice(&body.timestamp.div_euclid(1000).to_be_bytes());
    out.extend_from_slice(&(content.len() as i32).to_be_bytes());
    out.extend_from_slice(content);
    out.extend_from_slice(&(body.last_seen.len() as i32).to_be_bytes());
    for s in body.last_seen {
        out.extend_from_slice(s);
    }
    out
}

/// SignedMessageChain.DecodeException's messages, and the signed command
/// mismatch (ServerGamePacketListenerImpl.INVALID_COMMAND_SIGNATURE).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeError {
    MissingProfileKey,
    ChainBroken,
    ExpiredProfileKey,
    InvalidSignature,
    OutOfOrderChat,
    InvalidCommandSignature,
}

impl DecodeError {
    pub fn key(self) -> &'static str {
        match self {
            Self::MissingProfileKey => "chat.disabled.missingProfileKey",
            Self::ChainBroken => "chat.disabled.chain_broken",
            Self::ExpiredProfileKey => "chat.disabled.expiredProfileKey",
            Self::InvalidSignature => "chat.disabled.invalid_signature",
            Self::OutOfOrderChat => "chat.disabled.out_of_order_chat",
            Self::InvalidCommandSignature => "chat.disabled.invalid_command_signature",
        }
    }
}

/// A message the chain let through: unsigned (PlayerChatMessage.unsigned)
/// or signed at a link of the sender's chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unpacked {
    Unsigned,
    Signed { index: i32, signature: Signature },
}

/// A player's SignedMessageChain.Decoder: unsigned until the player sends a
/// chat session, then keyed to it.
#[derive(Clone, Debug, Default)]
pub struct Chain {
    session: Option<ChatSession>,
    /// The next link's index; None once the chain is broken.
    next: Option<i32>,
    /// The last message's time (lastTimeStamp, from Instant.EPOCH).
    last_timestamp: i64,
}

impl Chain {
    /// RemoteChatSession.createMessageDecoder: a new chain from link 0.
    pub fn keyed(session: ChatSession) -> Self {
        Self {
            session: Some(session),
            next: Some(0),
            last_timestamp: 0,
        }
    }
    pub fn session(&self) -> Option<&ChatSession> {
        self.session.as_ref()
    }
    /// Decoder.unpack for `sender`'s message. Without a session the message
    /// is unsigned unless the server enforces secure chat; with one it must
    /// be the next link, in order, signed by the player's key.
    pub fn unpack(
        &mut self,
        sender: u128,
        signature: Option<&Signature>,
        body: &Body,
        enforce: bool,
        now: i64,
    ) -> Result<Unpacked, DecodeError> {
        let Some(session) = &self.session else {
            return if enforce {
                Err(DecodeError::MissingProfileKey)
            } else {
                Ok(Unpacked::Unsigned)
            };
        };
        let signature = signature.ok_or(DecodeError::MissingProfileKey)?;
        if session.profile_key.has_expired(now) {
            return Err(DecodeError::ExpiredProfileKey);
        }
        let index = self.next.ok_or(DecodeError::ChainBroken)?;
        if body.timestamp < self.last_timestamp {
            self.next = None;
            return Err(DecodeError::OutOfOrderChat);
        }
        self.last_timestamp = body.timestamp;
        let payload = signed_payload(sender, session.id, index, body);
        if !session
            .key
            .verify(SignatureHash::Sha256, &payload, signature)
        {
            self.next = None;
            return Err(DecodeError::InvalidSignature);
        }
        self.next = index.checked_add(1);
        Ok(Unpacked::Signed {
            index,
            signature: *signature,
        })
    }
    /// Decoder.setChainBroken (nothing for the unsigned decoder).
    pub fn set_broken(&mut self) {
        if self.session.is_some() {
            self.next = None;
        }
    }
}

/// Arrays.hashCode(byte[]).
fn java_hash(bytes: &[u8]) -> i32 {
    bytes.iter().fold(1i32, |h, &b| {
        h.wrapping_mul(31).wrapping_add(i32::from(b as i8))
    })
}

/// LastSeenMessages.computeChecksum: never 0, which means "not checked".
pub fn checksum(entries: &[Signature]) -> i8 {
    let c = entries
        .iter()
        .fold(1i32, |c, e| c.wrapping_mul(31).wrapping_add(java_hash(e)));
    match c as i8 {
        0 => 1,
        b => b,
    }
}

/// LastSeenMessagesValidator: the messages sent to a player that its next
/// acknowledgement may name, each pending until it does.
#[derive(Clone, Debug)]
pub struct LastSeenValidator {
    tracked: VecDeque<Option<(Signature, bool)>>,
    last_pending: Option<Signature>,
}

impl Default for LastSeenValidator {
    fn default() -> Self {
        Self {
            tracked: std::iter::repeat_n(None, LAST_SEEN).collect(),
            last_pending: None,
        }
    }
}

impl LastSeenValidator {
    pub fn add_pending(&mut self, signature: &Signature) {
        if self.last_pending.as_ref() != Some(signature) {
            self.tracked.push_back(Some((*signature, true)));
            self.last_pending = Some(*signature);
        }
    }
    pub fn tracked_count(&self) -> usize {
        self.tracked.len()
    }
    /// applyOffset: drop the `offset` oldest messages.
    pub fn apply_offset(&mut self, offset: i32) -> Result<(), String> {
        let max = self.tracked.len() as i64 - LAST_SEEN as i64;
        if offset >= 0 && i64::from(offset) <= max {
            self.tracked.drain(..offset as usize);
            Ok(())
        } else {
            Err(format!(
                "Advanced last seen window by {offset} messages, but expected at most {max}"
            ))
        }
    }
    /// applyUpdate: the offset, then the acknowledged messages of the
    /// window (a 20-bit set, little-endian bytes), checked against the
    /// client's checksum unless it sent 0.
    pub fn apply_update(
        &mut self,
        offset: i32,
        acknowledged: [u8; 3],
        checksum: i8,
    ) -> Result<Vec<Signature>, String> {
        self.apply_offset(offset)?;
        let bit = |i: usize| acknowledged[i / 8] >> (i % 8) & 1 != 0;
        let length = (0..24).rev().find(|&i| bit(i)).map_or(0, |i| i + 1);
        if length > LAST_SEEN {
            return Err(format!(
                "Last seen update contained {length} messages, but maximum window size is {LAST_SEEN}"
            ));
        }
        let mut seen = Vec::with_capacity(LAST_SEEN);
        for i in 0..LAST_SEEN {
            let entry = self.tracked[i];
            if bit(i) {
                let Some((s, _)) = entry else {
                    return Err(format!(
                        "Last seen update acknowledged unknown or previously ignored message at index {i}"
                    ));
                };
                self.tracked[i] = Some((s, false));
                seen.push(s);
            } else {
                if let Some((_, false)) = entry {
                    return Err(format!(
                        "Last seen update ignored previously acknowledged message at index {i}"
                    ));
                }
                self.tracked[i] = None;
            }
        }
        if checksum != 0 && checksum != self::checksum(&seen) {
            return Err(
                "Checksum mismatch on last seen update: the client and server must have desynced"
                    .to_owned(),
            );
        }
        Ok(seen)
    }
}

/// MessageSignatureCache: the 128 signatures a connection's client also
/// holds, so a packed signature can name one by index.
#[derive(Clone, Debug)]
pub struct SignatureCache {
    entries: Vec<Option<Signature>>,
}

impl Default for SignatureCache {
    fn default() -> Self {
        Self {
            entries: vec![None; 128],
        }
    }
}

impl SignatureCache {
    /// MessageSignature.pack: the cache index, or the full signature.
    pub fn pack(&self, signature: &Signature) -> (i32, Option<Vec<u8>>) {
        match self
            .entries
            .iter()
            .position(|e| e.as_ref() == Some(signature))
        {
            Some(i) => (i as i32, None),
            None => (-1, Some(signature.to_vec())),
        }
    }
    /// push(body, signature): the message's last seen and its signature go
    /// to the front, most recent first; what they displace moves back.
    pub fn push(&mut self, last_seen: &[Signature], signature: Option<&Signature>) {
        let mut queue: VecDeque<Signature> = last_seen.iter().chain(signature).copied().collect();
        let new: HashSet<Signature> = queue.iter().copied().collect();
        for slot in &mut self.entries {
            let Some(next) = queue.pop_back() else { break };
            if let Some(old) = slot.replace(next)
                && !new.contains(&old)
            {
                queue.push_front(old);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sig(b: u8) -> Signature {
        [b; SIGNATURE_BYTES]
    }

    #[test]
    fn payload_layout_matches_update_signature() {
        let body = Body {
            content: "hé",
            timestamp: 1_700_000_123_456,
            salt: -2,
            last_seen: &[sig(7)],
        };
        let p = signed_payload(1, 2, 3, &body);
        assert_eq!(&p[..4], &[0, 0, 0, 1]);
        assert_eq!(&p[4..20], &1u128.to_be_bytes());
        assert_eq!(&p[20..36], &2u128.to_be_bytes());
        assert_eq!(&p[36..40], &3i32.to_be_bytes());
        assert_eq!(&p[40..48], &(-2i64).to_be_bytes());
        assert_eq!(&p[48..56], &1_700_000_123i64.to_be_bytes());
        assert_eq!(&p[56..60], &3i32.to_be_bytes());
        assert_eq!(&p[60..63], "hé".as_bytes());
        assert_eq!(&p[63..67], &1i32.to_be_bytes());
        assert_eq!(p.len(), 67 + 256);
        // Instant.getEpochSecond floors.
        let before = Body {
            timestamp: -1,
            ..body
        };
        assert_eq!(
            &signed_payload(1, 2, 3, &before)[48..56],
            &(-1i64).to_be_bytes()
        );
    }

    #[test]
    fn unsigned_chain_follows_enforcement() {
        let mut chain = Chain::default();
        let body = Body {
            content: "hi",
            timestamp: 5,
            salt: 0,
            last_seen: &[],
        };
        assert_eq!(
            chain.unpack(1, None, &body, false, 0),
            Ok(Unpacked::Unsigned)
        );
        assert_eq!(
            chain.unpack(1, Some(&sig(1)), &body, false, 0),
            Ok(Unpacked::Unsigned)
        );
        assert_eq!(
            chain.unpack(1, None, &body, true, 0),
            Err(DecodeError::MissingProfileKey)
        );
    }

    #[test]
    fn checksums_follow_java_hash_codes() {
        // Arrays.hashCode(new byte[]{-1}) = 31 + (-1) = 30.
        assert_eq!(java_hash(&[0xff]), 30);
        assert_eq!(checksum(&[]), 1);
        let one = [sig(0)];
        let expected = 31i32.wrapping_add(java_hash(&sig(0))) as i8;
        assert_eq!(checksum(&one), if expected == 0 { 1 } else { expected });
    }

    #[test]
    fn last_seen_window() {
        let mut v = LastSeenValidator::default();
        assert_eq!(v.tracked_count(), 20);
        v.add_pending(&sig(1));
        v.add_pending(&sig(1));
        v.add_pending(&sig(2));
        assert_eq!(v.tracked_count(), 22);
        assert!(v.apply_offset(3).is_err());
        // Slide by two: the two messages are the window's last entries.
        let mut ack = [0u8; 3];
        ack[18 / 8] |= 1 << (18 % 8);
        ack[19 / 8] |= 1 << (19 % 8);
        let seen = v.apply_update(2, ack, checksum(&[sig(1), sig(2)])).unwrap();
        assert_eq!(seen, vec![sig(1), sig(2)]);
        // Once acknowledged, a message can't be ignored.
        assert!(v.apply_update(0, [0; 3], 0).is_err());
        let mut v = LastSeenValidator::default();
        v.add_pending(&sig(1));
        assert!(v.apply_update(1, [0, 0, 0x10], 0).is_err(), "bit 20");
        let mut v = LastSeenValidator::default();
        assert!(v.apply_update(0, [1, 0, 0], 0).is_err(), "unknown");
        let mut v = LastSeenValidator::default();
        v.add_pending(&sig(1));
        let mut ack = [0u8; 3];
        ack[2] |= 1 << 3;
        assert!(v.apply_update(1, ack, 99).is_err(), "checksum");
    }

    #[test]
    fn cache_moves_recent_signatures_first() {
        let mut c = SignatureCache::default();
        assert_eq!(c.pack(&sig(1)), (-1, Some(sig(1).to_vec())));
        c.push(&[], Some(&sig(1)));
        assert_eq!(c.pack(&sig(1)), (0, None));
        c.push(&[sig(1)], Some(&sig(2)));
        assert_eq!(c.pack(&sig(2)), (0, None));
        assert_eq!(c.pack(&sig(1)), (1, None));
        c.push(&[], Some(&sig(3)));
        assert_eq!(
            [c.pack(&sig(3)).0, c.pack(&sig(2)).0, c.pack(&sig(1)).0],
            [0, 1, 2]
        );
    }
}
