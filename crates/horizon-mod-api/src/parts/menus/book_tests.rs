use super::*;
fn screen(book: &crate::RecipeBookTypeKey) -> crate::Screen {
    crate::Screen {
        width: 176.0,
        height: 166.0,
        slots: vec![Some([8.0, 8.0]); 3],
        recipe_book: Some(book.clone()),
        ..Default::default()
    }
}
fn roles() -> crate::RecipeBookLayout {
    crate::RecipeBookLayout::Furnace {
        input: 0,
        fuel: 1,
        result: 2,
    }
}
#[test]
fn registration_refuses_unknown_books_duplicate_menus_and_overlapping_roles() {
    let mut registrar = PartRegistrar::new("demo");
    let book = registrar.recipe_book_type("demo:book").unwrap();
    assert!(
        registrar
            .menu_recipe_book("demo:pot", &book, "demo:cooking", roles(), screen(&book))
            .is_ok()
    );
    assert!(
        registrar
            .menu_recipe_book("demo:pot", &book, "demo:cooking", roles(), screen(&book))
            .is_err()
    );
    assert!(
        registrar
            .menu_recipe_book("other:pot", &book, "demo:cooking", roles(), screen(&book))
            .is_err()
    );
    assert!(
        registrar
            .menu_recipe_book(
                "demo:bad",
                &book,
                "demo:cooking",
                crate::RecipeBookLayout::Stonecutter {
                    input: 0,
                    result: 0
                },
                screen(&book)
            )
            .is_err()
    );
    let unknown = mod_vocab::__private::parts_installation::recipe_book_type("demo:unknown".into());
    assert!(
        registrar
            .menu_recipe_book(
                "demo:unknown",
                &unknown,
                "demo:cooking",
                roles(),
                screen(&book)
            )
            .is_err()
    );
    assert_eq!(registrar.menu_recipe_books.len(), 1);
    assert_eq!(registrar.refused.len(), 4);
    assert_eq!(
        registrar.menu_recipe_books["demo:pot"].screen,
        screen(&book)
    );
}
#[test]
fn missing_nonfinite_and_wrong_book_geometry_refuse_registration() {
    let mut registrar = PartRegistrar::new("demo");
    let book = registrar.recipe_book_type("demo:book").unwrap();
    let mut bad = screen(&book);
    bad.slots[1] = None;
    let error = registrar
        .menu_recipe_book("demo:missing", &book, "demo:cooking", roles(), bad)
        .unwrap_err();
    assert!(error.contains("mod demo menu \"demo:missing\"") && error.contains("slot 1"));
    let mut bad = screen(&book);
    bad.slots[1] = Some([f32::NAN, 0.0]);
    assert!(
        registrar
            .menu_recipe_book("demo:nan", &book, "demo:cooking", roles(), bad)
            .unwrap_err()
            .contains("finite")
    );
    let mut bad = screen(&book);
    bad.recipe_book = None;
    assert!(
        registrar
            .menu_recipe_book("demo:wrong", &book, "demo:cooking", roles(), bad)
            .unwrap_err()
            .contains("screen book differs")
    );
    assert!(registrar.menu_recipe_books.is_empty());
    assert_eq!(registrar.refused.len(), 3);
}
