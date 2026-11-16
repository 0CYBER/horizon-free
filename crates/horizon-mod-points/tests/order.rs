// Generated fixtures exercise every shape; TODO(arismoko/horizon#2273): make the generator emit clippy-clean code.
#![allow(clippy::all, unused_comparisons, unreachable_patterns, dead_code)]
use mod_api::__private::points::register;
use mod_points::{
    __private::{ApiKind, Forward, PointHost, SlotListener, TABLE, seal},
    PointId,
};

#[test]
fn point_priority_preserves_dependency_order_and_orders_injects_within_a_mod() {
    mod_api::__private::initialize().expect("the point fixture initializes its platform runtime");
    struct Row {
        place: PointId,
        priorities: [i32; 4],
        one_owner: bool,
        expected: [&'static str; 4],
    }
    let rows = [
        Row {
            place: PointId::Tick,
            priorities: [0; 4],
            one_owner: false,
            expected: ["z_native", "a_wasm", "m_native", "b_wasm"],
        },
        Row {
            place: PointId::TickReturn,
            priorities: [0, 10, 0, 0],
            one_owner: false,
            expected: ["a_wasm", "z_native", "m_native", "b_wasm"],
        },
        Row {
            place: PointId::TickWorld,
            priorities: [0; 4],
            one_owner: true,
            expected: ["wasm", "wasm", "native", "native"],
        },
    ];
    fn native() {}
    struct Endpoint;
    impl Forward for Endpoint {
        fn forward(&self, _: u32, _: PointHost<'_>, _: &mut dyn mod_api::wire::Record) {}
    }
    static ENDPOINT: Endpoint = Endpoint;
    for row in &rows {
        for (index, (owner, inject)) in [
            ("z_native", "z"),
            ("a_wasm", "a"),
            ("m_native", "m"),
            ("b_wasm", "b"),
        ]
        .into_iter()
        .enumerate()
        {
            let owner = if row.one_owner { "same_mod" } else { owner };
            // SAFETY: these synthetic slots inspect kind and identity; they never invoke the pointers.
            let listener = unsafe {
                if index % 2 == 0 {
                    SlotListener::api(owner, row.place.place(), native as *const ())
                } else {
                    SlotListener::forwarded(owner, row.place.place(), &ENDPOINT, 0)
                }
            };
            register(&listener.ordered(row.priorities[index], inject)).unwrap();
        }
    }
    for owner in 0..64 {
        let id = Box::leak(format!("owner_{owner}").into_boxed_str());
        // SAFETY: this fixture inspects identity without invoking the pointer.
        let listener =
            unsafe { SlotListener::api(id, PointId::TickWorldReturn.place(), native as *const ()) };
        register(&listener.ordered(owner % 3, "large")).unwrap();
    }
    seal();
    let mut owners = Vec::new();
    TABLE.fire_api(PointId::TickWorldReturn as usize, &mut |_| {
        owners.push(
            mod_api::content::crash::current_native_mod()
                .unwrap()
                .to_owned(),
        );
    });
    let expected: Vec<_> = (0..3)
        .rev()
        .flat_map(|priority| {
            (0..64)
                .filter(move |owner| owner % 3 == priority)
                .map(|owner| format!("owner_{owner}"))
        })
        .collect();
    assert_eq!(
        owners, expected,
        "equal priorities retain dependency order at scale"
    );
    for row in rows {
        let mut heard = Vec::new();
        TABLE.fire_api(row.place as usize, &mut |kind| {
            heard.push(if row.one_owner {
                match kind {
                    ApiKind::Typed(_) => "native",
                    ApiKind::Forwarded(..) => "wasm",
                }
            } else {
                mod_api::content::crash::current_native_mod().unwrap()
            });
        });
        assert_eq!(heard, row.expected);
    }
}
