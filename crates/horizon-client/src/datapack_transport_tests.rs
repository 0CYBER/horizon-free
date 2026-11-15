use super::*;
use horizon_protocol::{
    frame,
    packets::{configuration as cfg, play},
};
use mod_api::*;

fn declaration() -> DatapackRegistryDeclaration {
    DatapackRegistryDeclaration {
        mod_id: "large".into(),
        key: "large:blob".into(),
        fields: vec![DatapackField::required(
            "text",
            DatapackFieldKind::Text { max_len: 4 << 20 },
        )],
        sync: DatapackRegistrySync::Required,
    }
}
fn snapshot(generation: u64, text: String) -> DatapackSnapshot {
    DatapackSnapshot {
        session: 7,
        generation,
        registries: vec![DatapackSnapshotRegistry {
            key: "large:blob".into(),
            fields: declaration().fields,
            tags: Default::default(),
            entries: vec![DatapackSnapshotEntry {
                id: "large:entry".into(),
                values: vec![DatapackValue::Text(text)],
            }],
        }],
    }
}

#[test]
#[ignore = "daily"]
fn incompressible_large_registry_snapshot_fits_each_frame_and_activates_only_when_complete() {
    // The real receiver is process-wide, as is a real client's connection. Keep this
    // fixture away from parallel Session tests that legitimately clear that receiver.
    if std::env::var_os("HORIZON_REGISTRY_TRANSPORT_CHILD").is_none() {
        let full_name = std::any::type_name_of_val(&incompressible_large_registry_snapshot_fits_each_frame_and_activates_only_when_complete);
        let name = full_name.split_once("::").unwrap().1;
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--include-ignored", "--exact", name, "--nocapture"])
            .env("HORIZON_REGISTRY_TRANSPORT_CHILD", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "test={name} exit_code={:?} category=fixture-child-failed",
            output.status.code()
        );
        return;
    }
    struct Large;
    #[mod_api::__subscriptions]
    impl Hooks for Large {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let d = declaration();
            r.declare_datapack_registry("blob", d.sync, d.fields)
                .unwrap();
        }
    }
    let mods = vec![ModEntry::new("large", Box::new(Large))];
    mod_platform::__private::part_tests::install_parts_fixture(&mods).unwrap();
    mod_platform::testing::with_mods(mods.into(), || {
        clear_received();
        accept(&snapshot(1, "old".into()).encode().unwrap()).unwrap();
        let mut seed = 0x1a2b3c4d5e6f_u64;
        let text: String = (0..(3 << 20))
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                char::from(b'!' + (seed & 63) as u8)
            })
            .collect();
        let bytes = snapshot(2, text.clone()).encode().unwrap();
        assert!(bytes.len() > frame::MAX_FRAME);
        assert!(
            frame::encode(&bytes, Some(256)).is_err(),
            "the unsplit incompressible snapshot must exceed the real frame limit"
        );
        let packets: Vec<_> = chunks(&bytes).unwrap().collect();
        assert!(packets.len() > 1);
        for (i, data) in packets.iter().enumerate() {
            assert!(data.len() < 1 << 20, "vanilla unknown custom payload bound");
            let body = play::Clientbound::CustomPayload(cfg::CustomPayload {
                channel: CHANNEL.into(),
                data: data.clone(),
            })
            .encode();
            assert!(body.len() < frame::MAX_UNCOMPRESSED);
            frame::encode(&body, None).unwrap();
            frame::encode(&body, Some(256)).unwrap();
            accept_chunk(data).unwrap();
            assert_eq!(finish_transfer().is_ok(), i + 1 == packets.len());
            assert_eq!(
                received().unwrap().generation(),
                if i + 1 == packets.len() { 2 } else { 1 }
            );
        }
        let set = received().unwrap();
        let handle = set.handle("large:blob", "large:entry").unwrap();
        assert_eq!(
            set.get(&handle).unwrap().get("text"),
            Some(&DatapackValue::Text(text))
        );
        finish_transfer().unwrap();
        clear_received();
    });
}

#[test]
fn registry_fragments_refuse_oversize_reordered_duplicate_and_truncated_transfers() {
    let data = vec![1; CHUNK_DATA + 3];
    let parts: Vec<_> = chunks(&data).unwrap().collect();
    let mut assembly = Assembly::default();
    assert!(assembly.push(&parts[1]).is_err());
    assert!(assembly.push(&parts[0]).unwrap().is_none());
    assert!(assembly.push(&parts[0]).is_err());
    assert!(assembly.pending.is_none());
    let mut oversized = parts[0].clone();
    oversized[4..8].copy_from_slice(&((MAX_SNAPSHOT_BYTES + 1) as u32).to_le_bytes());
    assert!(assembly.push(&oversized).unwrap_err().contains("64 MiB"));
    let mut truncated = parts[0].clone();
    truncated.pop();
    assert!(assembly.push(&truncated).is_err());
    assert!(assembly.pending.is_none());
    assert!(assembly.push(&parts[0]).unwrap().is_none());
    assert_eq!(assembly.push(&parts[1]).unwrap(), Some(data));
    assert!(
        assembly.push(&parts[1]).is_err(),
        "a completed transfer cannot be replayed from its tail"
    );
    let larger = vec![2; 2 * CHUNK_DATA + 3];
    let mut changed: Vec<_> = chunks(&larger).unwrap().collect();
    assert!(assembly.push(&changed[0]).unwrap().is_none());
    changed[1][4..8].copy_from_slice(&((larger.len() + 1) as u32).to_le_bytes());
    assert!(
        assembly.push(&changed[1]).is_err(),
        "the declared total cannot change mid-transfer"
    );
}
