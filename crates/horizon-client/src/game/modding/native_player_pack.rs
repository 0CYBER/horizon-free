//! A resource pack pushed to one connected player in play, and taken off again.
use super::{Game, api_values::TryIntoEngine};
use horizon_protocol::packets::{
    configuration::{ResourcePackPop, ResourcePackPush},
    play::Clientbound as C,
};
use mod_api::{PlayerPackRefusal as Refusal, PlayerRef, ResourcePackOffer};

/// The most UTF-16 code units of a protocol string (`FriendlyByteBuf.writeUtf` without a limit).
const URL_LIMIT: usize = 32767;
/// `ClientboundResourcePackPushPacket` refuses a longer hash.
const HASH_LIMIT: usize = 40;

impl Game<'_> {
    /// The connected player's uuid, or why nothing can be sent to them.
    fn pack_target(&self, player: PlayerRef) -> Result<u128, Refusal> {
        self.player_connection(player)
            .map(|_| player.uuid())
            .map_err(|refusal| match refusal {
                mod_api::PlayerWorldRefusal::Client => Refusal::Client,
                mod_api::PlayerWorldRefusal::NoConnection => Refusal::NoConnection,
                _ => Refusal::Stale,
            })
    }
    /// `ClientboundResourcePackPushPacket` to that player, in play.
    pub(super) fn player_push_resource_pack(
        &mut self,
        player: PlayerRef,
        pack: ResourcePackOffer,
    ) -> Result<(), Refusal> {
        let uuid = self.pack_target(player)?;
        if pack.url.encode_utf16().count() > URL_LIMIT {
            return Err(Refusal::UrlTooLong);
        }
        if pack.sha1.encode_utf16().count() > HASH_LIMIT {
            return Err(Refusal::HashTooLong);
        }
        let prompt = pack
            .prompt
            .map(|text| text.try_into_engine(self))
            .transpose()
            .map_err(Refusal::BadPrompt)?
            .map(|text| horizon_client::lan::data::component_nbt(&text.component()));
        self.native_send(
            uuid,
            C::ResourcePackPush(ResourcePackPush {
                id: pack.id,
                url: pack.url,
                hash: pack.sha1,
                required: pack.required,
                prompt,
            }),
        );
        Ok(())
    }
    /// `ClientboundResourcePackPopPacket` to that player: one pack, or all of them.
    pub(super) fn player_pop_resource_pack(
        &mut self,
        player: PlayerRef,
        id: Option<u128>,
    ) -> Result<(), Refusal> {
        let uuid = self.pack_target(player)?;
        self.native_send(uuid, C::ResourcePackPop(ResourcePackPop { id }));
        Ok(())
    }
}
