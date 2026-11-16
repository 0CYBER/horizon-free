#[test]
#[ignore = "daily"]
fn profession_mod_missing_refused_before_pairing() {
    const CHILD: &str = "HORIZON_PROFESSION_JOIN_TEST_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--include-ignored",
                "--exact",
                "lan::conn::tests::profession_mod_missing_refused_before_pairing",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(String::from_utf8_lossy(&result.stdout).contains("1 passed"));
        return;
    }
    use horizon_protocol::mod_list::{self, Entry, Side};
    assert!(mod_list::set_local(vec![Entry {
        id: "test".into(),
        version: "1.0".into(),
        side: Side::Both
    }]));
    let config = crate::lan::NetConfig {
        announce: false,
        listen: false,
        ..crate::lan::NetConfig::lan(0, "Profession join".into(), 8)
    };
    let lan = crate::lan::Lan::start_with(Arc::new(SyncData::empty()), config).unwrap();
    let mut c = Connection::local(lan.connect_local().unwrap());
    c.reader
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    login_profile(&mut c, "Owner");
    c.send(&login::Serverbound::LoginAcknowledged(horizon_protocol::packets::Empty).encode())
        .unwrap();
    let mut brand = horizon_protocol::buf::Writer::new();
    brand.string("horizon");
    for (channel, data) in [
        ("minecraft:brand", brand.data),
        (mod_list::CHANNEL, mod_list::encode(&[])),
    ] {
        c.send(
            &cfg::Serverbound::CustomPayload(cfg::CustomPayload {
                channel: channel.into(),
                data,
            })
            .encode(),
        )
        .unwrap();
    }
    loop {
        let (id, payload) = c.recv().unwrap();
        match cfg::Clientbound::decode(id, &payload).unwrap() {
            cfg::Clientbound::CustomPayload(p)
                if p.channel == horizon_protocol::payload_negotiation::CHANNEL =>
            {
                use horizon_protocol::payload_negotiation::{CHANNEL, Control, Manifest};
                if Control::decode(&p.data).unwrap() == Control::Query {
                    c.send(
                        &cfg::Serverbound::CustomPayload(cfg::CustomPayload {
                            channel: CHANNEL.into(),
                            data: Control::Offer(Manifest::default()).encode().unwrap(),
                        })
                        .encode(),
                    )
                    .unwrap();
                }
            }
            cfg::Clientbound::SelectKnownPacks(p) => c
                .send(&cfg::Serverbound::SelectKnownPacks(p).encode())
                .unwrap(),
            cfg::Clientbound::Disconnect(d) => {
                let reason = horizon_protocol::text::plain(&d.reason, &Default::default());
                assert!(reason.contains("test 1.0"), "{reason}");
                // Configuration never activated play, so no entity could pair or send metadata.
                assert!(c.recv().is_err(), "packets followed the refusal");
                break;
            }
            cfg::Clientbound::FinishConfiguration(_) => {
                panic!("missing profession mod reached play admission")
            }
            _ => {}
        }
    }
}
