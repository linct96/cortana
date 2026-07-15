use super::{codex::*, db::*, *};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AgentsProfile {
    id: String,
    name: String,
    content: String,
    is_active: bool,
    created_at: i64,
    updated_at: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AgentsStatus {
    profiles: Vec<AgentsProfile>,
    path: String,
    file_state: String,
}

fn agents_path(state: &AppState) -> Result<PathBuf, String> {
    Ok(auth_path(state)?.with_file_name("AGENTS.md"))
}

fn current_agents_profile_id(
    state: &AppState,
    connection: &Connection,
) -> Result<Option<String>, String> {
    let content = read_optional_file(&agents_path(state)?)?;
    content
        .as_deref()
        .filter(|content| !content.trim().is_empty())
        .map(|content| unique_profile_id_for_content(connection, content))
        .transpose()
        .map(Option::flatten)
}

#[tauri::command]
pub(super) fn get_agents_status(state: State<'_, AppState>) -> Result<AgentsStatus, String> {
    get_agents_status_internal(&state)
}

#[tauri::command]
pub(super) fn create_agents_profile(
    state: State<'_, AppState>,
    name: String,
    content: String,
) -> Result<AgentsProfile, String> {
    create_agents_profile_internal(&state, &name, &content)
}

#[tauri::command]
pub(super) fn update_agents_profile(
    state: State<'_, AppState>,
    profile_id: String,
    name: String,
    content: String,
) -> Result<AgentsProfile, String> {
    update_agents_profile_internal(&state, &profile_id, &name, &content)
}

#[tauri::command]
pub(super) fn activate_agents_profile(
    state: State<'_, AppState>,
    profile_id: String,
    force: bool,
) -> Result<AgentsProfile, String> {
    activate_agents_profile_internal(&state, &profile_id, force)
}

#[tauri::command]
pub(super) fn import_current_agents(
    state: State<'_, AppState>,
    name: String,
) -> Result<AgentsProfile, String> {
    import_current_agents_internal(&state, &name)
}

#[tauri::command]
pub(super) fn delete_agents_profile(
    state: State<'_, AppState>,
    profile_id: String,
) -> Result<(), String> {
    delete_agents_profile_internal(&state, &profile_id)
}

fn get_agents_status_internal(state: &AppState) -> Result<AgentsStatus, String> {
    let connection = open_database(state)?;
    let path = agents_path(state)?;
    let file_content = read_optional_file(&path)?;
    let active_id = file_content
        .as_deref()
        .filter(|content| !content.trim().is_empty())
        .map(|content| unique_profile_id_for_content(&connection, content))
        .transpose()?
        .flatten();
    let file_state = if active_id.is_some() {
        "managed"
    } else if file_content
        .as_deref()
        .is_none_or(|content| content.trim().is_empty())
    {
        "missing"
    } else {
        "unmanaged"
    };

    Ok(AgentsStatus {
        profiles: list_agents_profiles(&connection, active_id.as_deref())?,
        path: path.display().to_string(),
        file_state: file_state.to_string(),
    })
}

fn create_agents_profile_internal(
    state: &AppState,
    name: &str,
    content: &str,
) -> Result<AgentsProfile, String> {
    let connection = open_database(state)?;
    let name = validate_name(&connection, name, None)?;
    validate_content(&connection, content, None)?;
    let id = Uuid::new_v4().to_string();
    let now = now_millis();
    connection
        .execute(
            "INSERT INTO instruction_profiles (id, name, content, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?4)",
            params![id, name, content, now],
        )
        .map_err(database_error)?;
    let active_id = current_agents_profile_id(state, &connection)?;
    get_agents_profile(&connection, &id, active_id.as_deref())
}

fn update_agents_profile_internal(
    state: &AppState,
    profile_id: &str,
    name: &str,
    content: &str,
) -> Result<AgentsProfile, String> {
    let mut connection = open_database(state)?;
    let name = validate_name(&connection, name, Some(profile_id))?;
    get_profile_content(&connection, profile_id)?
        .ok_or_else(|| "提示词方案不存在。".to_string())?;
    validate_content(&connection, content, Some(profile_id))?;
    let path = agents_path(state)?;
    let backup = read_optional_file(&path)?;
    let active_id = backup
        .as_deref()
        .filter(|current| !current.trim().is_empty())
        .map(|current| unique_profile_id_for_content(&connection, current))
        .transpose()?
        .flatten();
    let is_active = active_id.as_deref() == Some(profile_id);
    if is_active {
        write_file_atomically(&path, content)?;
    }

    let now = now_millis();
    let transaction = connection.transaction().map_err(database_error)?;
    let result = transaction
        .execute(
            "UPDATE instruction_profiles SET name = ?1, content = ?2, updated_at = ?3 WHERE id = ?4",
            params![name, content, now, profile_id],
        )
        .map_err(database_error)
        .and_then(|_| transaction.commit().map_err(database_error));
    if let Err(error) = result {
        if is_active {
            restore_optional_file(&path, backup.as_deref())?;
        }
        return Err(error);
    }
    let active_id = current_agents_profile_id(state, &connection)?;
    get_agents_profile(&connection, profile_id, active_id.as_deref())
}

fn activate_agents_profile_internal(
    state: &AppState,
    profile_id: &str,
    force: bool,
) -> Result<AgentsProfile, String> {
    let connection = open_database(state)?;
    let content = get_profile_content(&connection, profile_id)?
        .ok_or_else(|| "提示词方案不存在。".to_string())?;
    validate_content(&connection, &content, Some(profile_id))?;
    let path = agents_path(state)?;
    let backup = read_optional_file(&path)?;
    ensure_unmanaged_file_can_be_replaced(&connection, backup.as_deref(), &content, force)?;
    write_file_atomically(&path, &content)?;
    get_agents_profile(&connection, profile_id, Some(profile_id))
}

fn import_current_agents_internal(state: &AppState, name: &str) -> Result<AgentsProfile, String> {
    let path = agents_path(state)?;
    let content = read_optional_file(&path)?
        .filter(|content| !content.trim().is_empty())
        .ok_or_else(|| "当前 AGENTS.md 为空，无法同步。".to_string())?;
    let mut connection = open_database(state)?;
    let matching_ids = profile_ids_for_content(&connection, &content)?;
    if matching_ids.len() == 1 {
        return get_agents_profile(&connection, &matching_ids[0], Some(&matching_ids[0]));
    }
    if matching_ids.len() > 1 {
        return Err("存在多个内容相同的提示词方案，请先处理重复内容。".to_string());
    }
    let name = validate_name(&connection, name, None)?;
    validate_content(&connection, &content, None)?;
    let id = Uuid::new_v4().to_string();
    let now = now_millis();
    let transaction = connection.transaction().map_err(database_error)?;
    transaction
        .execute(
            "INSERT INTO instruction_profiles (id, name, content, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?4)",
            params![id, name, content, now],
        )
        .map_err(database_error)?;
    transaction.commit().map_err(database_error)?;
    get_agents_profile(&connection, &id, Some(&id))
}

fn delete_agents_profile_internal(state: &AppState, profile_id: &str) -> Result<(), String> {
    let connection = open_database(state)?;
    let changed = connection
        .execute(
            "DELETE FROM instruction_profiles WHERE id = ?1",
            params![profile_id],
        )
        .map_err(database_error)?;
    if changed == 0 {
        return Err("提示词方案不存在。".to_string());
    }
    Ok(())
}

fn ensure_unmanaged_file_can_be_replaced(
    connection: &Connection,
    current: Option<&str>,
    next: &str,
    force: bool,
) -> Result<(), String> {
    let current = current.unwrap_or_default();
    let managed =
        !current.trim().is_empty() && unique_profile_id_for_content(connection, current)?.is_some();
    if !force && !current.trim().is_empty() && !managed && current != next {
        return Err("检测到未纳管的 AGENTS.md。请先同步当前文件，或确认后强制覆盖。".to_string());
    }
    Ok(())
}

fn validate_content(
    connection: &Connection,
    content: &str,
    excluded_id: Option<&str>,
) -> Result<(), String> {
    if content.trim().is_empty() {
        return Err("提示词内容不能为空。".to_string());
    }
    let duplicate = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM instruction_profiles WHERE content = ?1 AND (?2 IS NULL OR id <> ?2))",
            params![content, excluded_id],
            |row| row.get::<_, bool>(0),
        )
        .map_err(database_error)?;
    if duplicate {
        return Err("已存在内容相同的提示词方案。".to_string());
    }
    Ok(())
}

fn profile_ids_for_content(connection: &Connection, content: &str) -> Result<Vec<String>, String> {
    let mut statement = connection
        .prepare(
            "SELECT id FROM instruction_profiles WHERE content = ?1 ORDER BY created_at ASC LIMIT 2",
        )
        .map_err(database_error)?;
    let profiles = statement
        .query_map(params![content], |row| row.get(0))
        .map_err(database_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error)?;
    Ok(profiles)
}

fn unique_profile_id_for_content(
    connection: &Connection,
    content: &str,
) -> Result<Option<String>, String> {
    let ids = profile_ids_for_content(connection, content)?;
    Ok((ids.len() == 1).then(|| ids[0].clone()))
}

fn validate_name(
    connection: &Connection,
    name: &str,
    excluded_id: Option<&str>,
) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("方案名称不能为空。".to_string());
    }
    let exists = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM instruction_profiles WHERE name = ?1 COLLATE NOCASE AND (?2 IS NULL OR id <> ?2))",
            params![name, excluded_id],
            |row| row.get::<_, bool>(0),
        )
        .map_err(database_error)?;
    if exists {
        return Err("方案名称已存在。".to_string());
    }
    Ok(name.to_string())
}

fn get_profile_content(connection: &Connection, id: &str) -> Result<Option<String>, String> {
    connection
        .query_row(
            "SELECT content FROM instruction_profiles WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )
        .optional()
        .map_err(database_error)
}

fn list_agents_profiles(
    connection: &Connection,
    active_id: Option<&str>,
) -> Result<Vec<AgentsProfile>, String> {
    let mut statement = connection
        .prepare("SELECT id, name, content, created_at, updated_at FROM instruction_profiles ORDER BY created_at ASC")
        .map_err(database_error)?;
    let profiles = statement
        .query_map([], |row| agents_profile_from_row(row, active_id))
        .map_err(database_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error)?;
    Ok(profiles)
}

fn get_agents_profile(
    connection: &Connection,
    id: &str,
    active_id: Option<&str>,
) -> Result<AgentsProfile, String> {
    connection
        .query_row(
            "SELECT id, name, content, created_at, updated_at FROM instruction_profiles WHERE id = ?1",
            params![id],
            |row| agents_profile_from_row(row, active_id),
        )
        .optional()
        .map_err(database_error)?
        .ok_or_else(|| "提示词方案不存在。".to_string())
}

fn agents_profile_from_row(
    row: &rusqlite::Row<'_>,
    active_id: Option<&str>,
) -> rusqlite::Result<AgentsProfile> {
    let id: String = row.get(0)?;
    Ok(AgentsProfile {
        is_active: active_id == Some(id.as_str()),
        id,
        name: row.get(1)?,
        content: row.get(2)?,
        created_at: row.get(3)?,
        updated_at: row.get(4)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_state() -> (PathBuf, AppState) {
        let directory =
            std::env::temp_dir().join(format!("cortana-agents-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let state = AppState {
            database_path: directory.join("app.sqlite3"),
            default_codex_home: directory.join(".codex"),
            pending_oauth: Arc::new(Mutex::new(None)),
        };
        initialize_database(&state).unwrap();
        (directory, state)
    }

    #[test]
    fn creates_inactive_then_activates_exact_content() {
        let (directory, state) = test_state();
        let profile = create_agents_profile_internal(&state, "Default", "# Rules\n").unwrap();
        assert!(!agents_path(&state).unwrap().exists());
        update_agents_profile_internal(&state, &profile.id, "Default", "# Updated\n").unwrap();
        assert!(!agents_path(&state).unwrap().exists());

        activate_agents_profile_internal(&state, &profile.id, false).unwrap();

        assert_eq!(
            fs::read_to_string(agents_path(&state).unwrap()).unwrap(),
            "# Updated\n"
        );
        let status = get_agents_status_internal(&state).unwrap();
        assert_eq!(status.file_state, "managed");
        assert!(
            status
                .profiles
                .iter()
                .find(|item| item.id == profile.id)
                .unwrap()
                .is_active
        );
        assert_eq!(
            get_setting(&open_database(&state).unwrap(), "active_agents_profile_id").unwrap(),
            None
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn protects_and_can_import_external_content() {
        let (directory, state) = test_state();
        let first = create_agents_profile_internal(&state, "First", "first").unwrap();
        let second = create_agents_profile_internal(&state, "Second", "second").unwrap();
        activate_agents_profile_internal(&state, &first.id, false).unwrap();
        write_file_atomically(&agents_path(&state).unwrap(), "external").unwrap();

        assert!(activate_agents_profile_internal(&state, &second.id, false)
            .unwrap_err()
            .contains("未纳管"));
        let imported = import_current_agents_internal(&state, "Imported").unwrap();
        assert_eq!(imported.content, "external");
        assert!(imported.is_active);
        write_file_atomically(&agents_path(&state).unwrap(), "external-again").unwrap();
        activate_agents_profile_internal(&state, &second.id, true).unwrap();
        assert_eq!(
            fs::read_to_string(agents_path(&state).unwrap()).unwrap(),
            "second"
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn updates_active_file_and_leaves_it_when_deleted() {
        let (directory, state) = test_state();
        let profile = create_agents_profile_internal(&state, "Default", "old").unwrap();
        activate_agents_profile_internal(&state, &profile.id, false).unwrap();

        update_agents_profile_internal(&state, &profile.id, "Renamed", "new").unwrap();
        delete_agents_profile_internal(&state, &profile.id).unwrap();

        assert_eq!(
            fs::read_to_string(agents_path(&state).unwrap()).unwrap(),
            "new"
        );
        assert_eq!(
            get_agents_status_internal(&state).unwrap().file_state,
            "unmanaged"
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn follows_external_switch_to_an_existing_profile() {
        let (directory, state) = test_state();
        let first = create_agents_profile_internal(&state, "First", "first").unwrap();
        let second = create_agents_profile_internal(&state, "Second", "second").unwrap();
        activate_agents_profile_internal(&state, &first.id, false).unwrap();

        write_file_atomically(&agents_path(&state).unwrap(), "second").unwrap();

        let status = get_agents_status_internal(&state).unwrap();
        assert_eq!(status.file_state, "managed");
        assert!(
            !status
                .profiles
                .iter()
                .find(|profile| profile.id == first.id)
                .unwrap()
                .is_active
        );
        assert!(
            status
                .profiles
                .iter()
                .find(|profile| profile.id == second.id)
                .unwrap()
                .is_active
        );
        write_file_atomically(&agents_path(&state).unwrap(), "second\n").unwrap();
        assert_eq!(
            get_agents_status_internal(&state).unwrap().file_state,
            "unmanaged"
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn rejects_empty_and_duplicate_content() {
        let (directory, state) = test_state();
        assert!(create_agents_profile_internal(&state, "Empty", " \n").is_err());
        create_agents_profile_internal(&state, "First", "same").unwrap();
        assert!(create_agents_profile_internal(&state, "Duplicate", "same").is_err());
        let second = create_agents_profile_internal(&state, "Second", "other").unwrap();
        assert!(update_agents_profile_internal(&state, &second.id, "Second", "same").is_err());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn leaves_legacy_duplicate_content_unmanaged() {
        let (directory, state) = test_state();
        let connection = open_database(&state).unwrap();
        connection
            .execute_batch(
                "INSERT INTO instruction_profiles VALUES ('first', 'First', 'same', 1, 1);
                 INSERT INTO instruction_profiles VALUES ('second', 'Second', 'same', 2, 2);",
            )
            .unwrap();
        drop(connection);
        write_file_atomically(&agents_path(&state).unwrap(), "same").unwrap();

        let status = get_agents_status_internal(&state).unwrap();
        assert_eq!(status.file_state, "unmanaged");
        assert!(status.profiles.iter().all(|profile| !profile.is_active));
        assert!(activate_agents_profile_internal(&state, "first", false)
            .unwrap_err()
            .contains("内容相同"));
        assert!(import_current_agents_internal(&state, "Imported")
            .unwrap_err()
            .contains("内容相同"));
        fs::remove_dir_all(directory).unwrap();
    }
}
