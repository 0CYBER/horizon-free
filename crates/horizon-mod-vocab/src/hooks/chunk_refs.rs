pub trait Notes {
    #[notes = r#"## Loaded chunks

`LevelRef::loaded_chunks(host)` lists the chunks the level holds now, the same ones `chunk`
finds by position, ordered by x then z so two calls over an unchanged level agree. It loads no
chunk and sorts a copy of the level's loaded set, so call it when a mod needs the whole set, not
for a position it already knows."#]
    fn chunk_refs_note0();
}
