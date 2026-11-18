use super::super::{TryIntoApi, TryIntoEngine};
use horizon_core::arm_pose::ArmPose as Engine;
use mod_api::ArmPose as Api;

macro_rules! poses {
    ($($pose:ident),* $(,)?) => {
        impl TryIntoApi for Engine {
            type Api = Api;
            type Error = std::convert::Infallible;
            fn try_into_api(&self) -> Result<Api, Self::Error> {
                Ok(match self { $(Engine::$pose => Api::$pose,)* Engine::Custom { slot, .. } => mod_api::CustomArmPose::from_slot(*slot).map_or(Api::Empty, Api::Custom), })
            }
        }
        impl TryIntoEngine<()> for Api {
            type Engine = Engine;
            type Error = String;
            fn try_into_engine(&self, _: &()) -> Result<Engine, Self::Error> {
                Ok(match self { $(Api::$pose => Engine::$pose,)* Api::Custom(pose) => {
                    let (both, offhand) = pose.flags().ok_or("unknown custom arm pose")?;
                    Engine::Custom { slot: pose.slot(), both, offhand, angles: [[f32::NAN.to_bits(); 3]; 2] }
                } })
            }
        }
    };
}
poses!(
    Empty,
    Item,
    Block,
    Bow,
    CrossbowHold,
    CrossbowCharge,
    ThrowTrident,
    TootHorn,
    Spyglass,
    Spear,
    Brush
);

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn forged_pose_slots_are_refused_at_the_engine_boundary() {
        let mut data = 11u32.to_le_bytes().to_vec();
        data.extend(u32::MAX.to_le_bytes());
        data.extend([0u8, 0u8]);
        let pose = mod_api::wire::Reader::new(&data).take::<Api>().unwrap();
        assert!(pose.try_into_engine(&()).is_err());
    }
}
