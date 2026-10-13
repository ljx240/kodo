mod _helpers;

use _helpers::fresh_store;

#[test]
fn migrate_twice_is_idempotent() {
    let (_dir, store) = fresh_store("migrate-twice");
    store.ensure_schema().unwrap();
    store.ensure_schema().unwrap();
    let counts = store.row_counts().unwrap();
    assert_eq!(counts.get("memory_entries").copied().unwrap(), 0);
    // Both migrations are recorded.
    let applied = store
        .with_conn(|c| {
            let mut stmt = c.prepare("SELECT version FROM schema_migrations ORDER BY version")?;
            let v: Vec<i64> = stmt
                .query_map([], |r| r.get(0))?
                .collect::<Result<_, _>>()?;
            Ok::<_, kodo_memory::MemoryError>(v)
        })
        .unwrap();
    assert_eq!(applied, vec![1, 2]);
}

#[test]
fn foreign_keys_are_on() {
    let (_dir, store) = fresh_store("fk");
    let fk: i64 = store
        .with_conn(|c| Ok(c.query_row("PRAGMA foreign_keys", [], |r| r.get(0))?))
        .unwrap();
    assert_eq!(fk, 1);
}
