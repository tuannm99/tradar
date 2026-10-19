use super::*;

#[test]
fn loading_from_a_missing_file_returns_no_connections() {
    let dir = tempfile::tempdir().unwrap();
    let store = ConnectionStore::at(dir.path().join("connections.toml"));

    let connections = store.load().unwrap();

    assert!(connections.is_empty());
}

#[test]
fn default_connections_path_ends_with_connections_toml() {
    let path = default_connections_path().unwrap();

    assert_eq!(path.file_name().unwrap(), "connections.toml");
}

#[test]
fn saving_then_loading_round_trips_a_connection() {
    let dir = tempfile::tempdir().unwrap();
    let store = ConnectionStore::at(dir.path().join("connections.toml"));
    let connection = SavedConnection {
        name: "local".to_string(),
        driver: "sqlite".to_string(),
        target: "test.db".to_string(),
    };

    store.save(std::slice::from_ref(&connection)).unwrap();
    let loaded = store.load().unwrap();

    assert_eq!(loaded, vec![connection]);
}

#[test]
fn default_snippets_path_ends_with_snippets_toml() {
    let path = default_snippets_path().unwrap();

    assert_eq!(path.file_name().unwrap(), "snippets.toml");
}

#[test]
fn default_migrations_dir_is_scoped_to_the_connection_name() {
    let path = default_migrations_dir("prod-db").unwrap();

    assert_eq!(path.file_name().unwrap(), "prod-db");
    assert_eq!(path.parent().unwrap().file_name().unwrap(), "migrations");
}

#[test]
fn default_migrations_dir_sanitizes_an_unsafe_connection_name() {
    let path = default_migrations_dir("a/../../etc").unwrap();

    assert_eq!(path.file_name().unwrap(), "a_.._.._etc");
}

#[test]
fn move_dir_if_safe_moves_an_existing_directory_to_a_free_destination() {
    let root = tempfile::tempdir().unwrap();
    let old = root.path().join("old");
    let new = root.path().join("new");
    std::fs::create_dir(&old).unwrap();
    std::fs::write(old.join("0001_init.sql"), "SELECT 1;").unwrap();

    move_dir_if_safe(&old, &new);

    assert!(!old.exists(), "the old directory must be gone");
    assert!(
        new.join("0001_init.sql").exists(),
        "its contents must have moved"
    );
}

#[test]
fn move_dir_if_safe_is_a_no_op_when_the_old_directory_never_existed() {
    let root = tempfile::tempdir().unwrap();
    let old = root.path().join("old"); // never created -- no migrations run yet
    let new = root.path().join("new");

    move_dir_if_safe(&old, &new);

    assert!(!new.exists());
}

#[test]
fn move_dir_if_safe_leaves_the_old_directory_in_place_when_the_new_one_is_taken() {
    let root = tempfile::tempdir().unwrap();
    let old = root.path().join("old");
    let new = root.path().join("new");
    std::fs::create_dir(&old).unwrap();
    std::fs::write(old.join("0001_init.sql"), "SELECT 1;").unwrap();
    std::fs::create_dir(&new).unwrap();
    std::fs::write(new.join("unrelated.sql"), "SELECT 2;").unwrap();

    move_dir_if_safe(&old, &new);

    assert!(
        old.join("0001_init.sql").exists(),
        "nothing already at the destination must be overwritten or merged"
    );
    assert!(new.join("unrelated.sql").exists());
}

#[test]
fn loading_a_missing_snippets_file_returns_an_empty_list() {
    let dir = tempfile::tempdir().unwrap();

    let snippets = SnippetStore::at(dir.path().join("nope.toml"))
        .load()
        .unwrap();

    assert!(snippets.is_empty());
}

#[test]
fn saving_a_snippet_then_loading_round_trips_it() {
    let dir = tempfile::tempdir().unwrap();
    let store = SnippetStore::at(dir.path().join("snippets.toml"));
    let snippet = SavedSnippet {
        name: "active-users".to_string(),
        driver: "postgres".to_string(),
        text: "SELECT * FROM users WHERE active;".to_string(),
    };

    store.save(std::slice::from_ref(&snippet)).unwrap();
    let loaded = store.load().unwrap();

    assert_eq!(loaded, vec![snippet]);
}

/// A `Snippets` backed by a temp-dir store, bypassing the process-global
/// singleton (`init_snippets`/`snippets()`) entirely -- same reasoning
/// as testing `ConnectionStore`/`RecentStore` directly rather than
/// through `QueryFiles`.
fn test_snippets(dir: &std::path::Path) -> Snippets {
    Snippets {
        store: SnippetStore::at(dir.join("snippets.toml")),
        list: std::sync::RwLock::new(Vec::new()),
    }
}

#[test]
fn saving_under_an_existing_name_and_driver_overwrites_it() {
    let dir = tempfile::tempdir().unwrap();
    let snippets = test_snippets(dir.path());
    snippets.save(
        "q".to_string(),
        "postgres".to_string(),
        "SELECT 1".to_string(),
    );

    snippets.save(
        "q".to_string(),
        "postgres".to_string(),
        "SELECT 2".to_string(),
    );

    assert_eq!(
        snippets.for_driver("postgres"),
        vec![SavedSnippet {
            name: "q".to_string(),
            driver: "postgres".to_string(),
            text: "SELECT 2".to_string(),
        }],
        "saving under a name already in use must update it, not duplicate it"
    );
}

#[test]
fn the_same_name_on_two_different_drivers_does_not_conflict() {
    let dir = tempfile::tempdir().unwrap();
    let snippets = test_snippets(dir.path());

    snippets.save(
        "q".to_string(),
        "postgres".to_string(),
        "SELECT 1".to_string(),
    );
    snippets.save("q".to_string(), "redis".to_string(), "GET k".to_string());

    assert_eq!(snippets.for_driver("postgres").len(), 1);
    assert_eq!(snippets.for_driver("redis").len(), 1);
}

#[test]
fn for_driver_only_returns_that_driver_s_snippets() {
    let dir = tempfile::tempdir().unwrap();
    let snippets = test_snippets(dir.path());
    snippets.save(
        "a".to_string(),
        "postgres".to_string(),
        "SELECT 1".to_string(),
    );
    snippets.save("b".to_string(), "redis".to_string(), "GET k".to_string());

    let entries = snippets.for_driver("postgres");
    let names: Vec<&str> = entries.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, vec!["a"]);
}

#[test]
fn deleting_removes_only_the_matching_driver_s_entry() {
    let dir = tempfile::tempdir().unwrap();
    let snippets = test_snippets(dir.path());
    snippets.save(
        "q".to_string(),
        "postgres".to_string(),
        "SELECT 1".to_string(),
    );
    snippets.save("q".to_string(), "redis".to_string(), "GET k".to_string());

    snippets.delete("postgres", "q");

    assert!(snippets.for_driver("postgres").is_empty());
    assert_eq!(snippets.for_driver("redis").len(), 1);
}

#[test]
fn renaming_updates_the_name_and_persists_to_disk() {
    let dir = tempfile::tempdir().unwrap();
    let snippets = test_snippets(dir.path());
    snippets.save(
        "old".to_string(),
        "postgres".to_string(),
        "SELECT 1".to_string(),
    );

    snippets.rename("postgres", "old", "new".to_string());

    assert_eq!(
        snippets.for_driver("postgres"),
        vec![SavedSnippet {
            name: "new".to_string(),
            driver: "postgres".to_string(),
            text: "SELECT 1".to_string(),
        }]
    );
    let reloaded = SnippetStore::at(dir.path().join("snippets.toml"))
        .load()
        .unwrap();
    assert_eq!(reloaded[0].name, "new");
}

#[test]
fn renaming_a_snippet_that_does_not_exist_is_a_harmless_no_op() {
    let dir = tempfile::tempdir().unwrap();
    let snippets = test_snippets(dir.path());

    snippets.rename("postgres", "ghost", "new".to_string());

    assert!(snippets.for_driver("postgres").is_empty());
}

#[test]
fn a_session_file_from_before_queries_were_saved_is_ignored_not_fatal() {
    // The old format stored tabs as plain strings. Rather than
    // migrating, loading fails and the caller falls back to a fresh
    // session -- losing a restore is a smaller cost than the migration
    // code for a file that is rewritten on every quit anyway.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("session.toml");
    std::fs::write(&path, "active_tab = 0\ntabs = [\"local sqlite\"]\n").unwrap();

    assert!(SessionStore::at(path).load().is_err());
}

#[test]
fn a_bare_name_is_saved_into_the_queries_directory_as_sql() {
    let dir = std::path::Path::new("/home/u/.config/tradar/queries");

    assert_eq!(
        resolve_query_path("report", dir),
        dir.join("report.sql"),
        "a bare name gets the extension so files are readable elsewhere"
    );
    assert_eq!(
        resolve_query_path("report.txt", dir),
        dir.join("report.txt"),
        "an explicit extension is respected"
    );
}

#[test]
fn an_explicit_escape_is_used_as_typed() {
    let dir = std::path::Path::new("/home/u/.config/tradar/queries");

    assert_eq!(
        resolve_query_path("/tmp/one.sql", dir),
        PathBuf::from("/tmp/one.sql")
    );
    assert_eq!(
        resolve_query_path("./one.sql", dir),
        PathBuf::from("./one.sql")
    );
    assert_eq!(
        resolve_query_path("../one.sql", dir),
        PathBuf::from("../one.sql")
    );
    assert_eq!(
        resolve_query_path("~/one.sql", dir),
        PathBuf::from("~/one.sql")
    );
}

#[test]
fn a_relative_subfolder_is_joined_into_the_queries_directory() {
    let dir = std::path::Path::new("/home/u/.config/tradar/queries");

    assert_eq!(
        resolve_query_path("reports/first", dir),
        dir.join("reports/first.sql"),
        "no leading escape marker, so this is a subfolder inside the queries dir, not a cwd-relative path"
    );
    assert_eq!(
        resolve_query_path("reports/first.sql", dir),
        dir.join("reports/first.sql")
    );
}

#[test]
fn recording_a_file_moves_it_to_the_front_without_duplicating() {
    let mut recent = RecentFiles::default();

    recent.record(std::path::Path::new("/a.sql"));
    recent.record(std::path::Path::new("/b.sql"));
    recent.record(std::path::Path::new("/a.sql"));

    assert_eq!(recent.paths, vec!["/a.sql", "/b.sql"]);
}

#[test]
fn the_recent_list_is_capped() {
    let mut recent = RecentFiles::default();

    for i in 0..(MAX_RECENT + 5) {
        recent.record(std::path::Path::new(&format!("/{i}.sql")));
    }

    assert_eq!(recent.paths.len(), MAX_RECENT);
    assert_eq!(recent.paths[0], format!("/{}.sql", MAX_RECENT + 4));
}

#[test]
fn saving_then_loading_round_trips_the_recent_list() {
    let dir = tempfile::tempdir().unwrap();
    let store = RecentStore::at(dir.path().join("recent.toml"));
    let mut recent = RecentFiles::default();
    recent.record(std::path::Path::new("/a.sql"));

    store.save(&recent).unwrap();

    assert_eq!(store.load().unwrap(), recent);
}

#[test]
fn loading_a_missing_recent_file_returns_an_empty_list() {
    let dir = tempfile::tempdir().unwrap();

    let recent = RecentStore::at(dir.path().join("nope.toml"))
        .load()
        .unwrap();

    assert!(recent.paths.is_empty());
}

#[test]
fn loading_a_missing_session_file_returns_the_default_state() {
    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::at(dir.path().join("session.toml"));

    let state = store.load().unwrap();

    assert_eq!(state, SessionState::default());
}

#[test]
fn default_session_path_ends_with_session_toml() {
    let path = default_session_path().unwrap();

    assert_eq!(path.file_name().unwrap(), "session.toml");
}

#[test]
fn saving_then_loading_round_trips_a_session_state() {
    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::at(dir.path().join("session.toml"));
    let state = SessionState {
        active_tab: 1,
        tabs: vec![
            TabState {
                connection: "local sqlite".to_string(),
                query: "select 1;\nselect 2;".to_string(),
            },
            TabState::new("local postgres"),
        ],
    };

    store.save(&state).unwrap();
    let loaded = store.load().unwrap();

    assert_eq!(loaded, state);
}

#[test]
fn default_http_requests_path_ends_with_http_requests_toml() {
    let path = default_http_requests_path().unwrap();

    assert_eq!(path.file_name().unwrap(), "http_requests.toml");
}

#[test]
fn loading_a_missing_http_requests_file_returns_an_empty_list() {
    let dir = tempfile::tempdir().unwrap();

    let requests = HttpRequestStore::at(dir.path().join("nope.toml"))
        .load()
        .unwrap();

    assert!(requests.is_empty());
}

#[test]
fn saving_an_http_request_then_loading_round_trips_it() {
    let dir = tempfile::tempdir().unwrap();
    let store = HttpRequestStore::at(dir.path().join("http_requests.toml"));
    let request = SavedHttpRequest {
        name: "list users".to_string(),
        method: "GET".to_string(),
        url: "https://api.example.com/users".to_string(),
        headers: "Accept: application/json".to_string(),
        body: String::new(),
    };

    store.save(std::slice::from_ref(&request)).unwrap();
    let loaded = store.load().unwrap();

    assert_eq!(loaded, vec![request]);
}

fn test_http_requests(dir: &std::path::Path) -> HttpRequests {
    HttpRequests {
        store: HttpRequestStore::at(dir.join("http_requests.toml")),
        list: std::sync::RwLock::new(Vec::new()),
    }
}

fn sample_request(name: &str) -> SavedHttpRequest {
    SavedHttpRequest {
        name: name.to_string(),
        method: "GET".to_string(),
        url: "https://api.example.com".to_string(),
        headers: String::new(),
        body: String::new(),
    }
}

#[test]
fn saving_under_an_existing_name_overwrites_it() {
    let dir = tempfile::tempdir().unwrap();
    let requests = test_http_requests(dir.path());
    requests.save(sample_request("q"));

    let mut updated = sample_request("q");
    updated.method = "POST".to_string();
    requests.save(updated);

    let all = requests.all();
    assert_eq!(
        all.len(),
        1,
        "saving under a used name must update, not duplicate"
    );
    assert_eq!(all[0].method, "POST");
}

#[test]
fn deleting_removes_the_matching_entry() {
    let dir = tempfile::tempdir().unwrap();
    let requests = test_http_requests(dir.path());
    requests.save(sample_request("a"));
    requests.save(sample_request("b"));

    requests.delete("a");

    let names: Vec<String> = requests.all().into_iter().map(|r| r.name).collect();
    assert_eq!(names, vec!["b".to_string()]);
}
