use rusqlite::Connection;

/// One migration file: its version, its name without `.sql`, and its SQL.
pub struct File {
    pub version: i64,
    #[allow(dead_code)] // Only fingerprint-history tests need the file name.
    pub name: String,
    pub sql: String,
}

/// Every migration file, in the order they apply.
pub fn migrations() -> Vec<File> {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let mut paths: Vec<_> = std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "sql"))
        .collect();
    paths.sort();
    let migrations: Vec<_> = paths
        .iter()
        .map(|path| {
            let name = path.file_stem().unwrap().to_str().unwrap().to_owned();
            File {
                version: name.split('_').next().unwrap().parse().unwrap(),
                name,
                sql: std::fs::read_to_string(path).unwrap(),
            }
        })
        .collect();
    let versions: Vec<_> = migrations.iter().map(|file| file.version).collect();
    assert_eq!(versions, (1..=versions.len() as i64).collect::<Vec<_>>());
    migrations
}

/// Replay the real historical files, one committed transaction per version.
pub fn apply_migrations(
    connection: &Connection,
    files: &[File],
    applied_at: &str,
    mut after_apply: impl FnMut(&Connection, i64),
) {
    connection
        .execute_batch(
            "CREATE TABLE schema_version (version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL)",
        )
        .unwrap();
    for file in files {
        let transaction = connection.unchecked_transaction().unwrap();
        transaction.execute_batch(&file.sql).unwrap();
        transaction
            .execute(
                "INSERT INTO schema_version(version, applied_at) VALUES (?1, ?2)",
                rusqlite::params![file.version, applied_at],
            )
            .unwrap();
        after_apply(&transaction, file.version);
        transaction.commit().unwrap();
    }
}
