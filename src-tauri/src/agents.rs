use super::{codex::*, db::*, *};

const ANTIGRAVITY_MAX_INSTRUCTION_CHARS: usize = 12_000;

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
    unmanaged_content: Option<String>,
}

fn instruction_path(state: &AppState, product: AccountProduct) -> Result<PathBuf, String> {
    let user_home = state
        .default_codex_home
        .parent()
        .unwrap_or(&state.default_codex_home);
    Ok(match product {
        AccountProduct::Codex => auth_path(state)?.with_file_name("AGENTS.md"),
        AccountProduct::Claude => std::env::var_os("CLAUDE_CONFIG_DIR")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| user_home.join(".claude"))
            .join("CLAUDE.md"),
        AccountProduct::Antigravity => user_home.join(".gemini/GEMINI.md"),
        AccountProduct::Grok => env::grok_home(state).join("AGENTS.md"),
    })
}

fn instruction_filename(product: AccountProduct) -> &'static str {
    match product {
        AccountProduct::Claude => "CLAUDE.md",
        AccountProduct::Antigravity => "GEMINI.md",
        AccountProduct::Codex | AccountProduct::Grok => "AGENTS.md",
    }
}

fn current_instruction_profile_id(
    state: &AppState,
    connection: &Connection,
    product: AccountProduct,
) -> Result<Option<String>, String> {
    let content = read_optional_file(&instruction_path(state, product)?)?;
    content
        .as_deref()
        .filter(|content| !content.trim().is_empty())
        .map(|content| unique_profile_id_for_content(connection, product, content))
        .transpose()
        .map(Option::flatten)
}

#[tauri::command]
pub(super) fn get_agents_status(
    state: State<'_, AppState>,
    product: AccountProduct,
) -> Result<AgentsStatus, String> {
    get_agents_status_internal(&state, product)
}

#[tauri::command]
pub(super) fn create_agents_profile(
    state: State<'_, AppState>,
    product: AccountProduct,
    name: String,
    content: String,
) -> Result<AgentsProfile, String> {
    create_agents_profile_internal(&state, product, &name, &content)
}

#[tauri::command]
pub(super) fn update_agents_profile(
    state: State<'_, AppState>,
    product: AccountProduct,
    profile_id: String,
    name: String,
    content: String,
) -> Result<AgentsProfile, String> {
    update_agents_profile_internal(&state, product, &profile_id, &name, &content)
}

#[tauri::command]
pub(super) fn activate_agents_profile(
    state: State<'_, AppState>,
    product: AccountProduct,
    profile_id: String,
    force: bool,
) -> Result<AgentsProfile, String> {
    activate_agents_profile_internal(&state, product, &profile_id, force)
}

#[tauri::command]
pub(super) fn import_current_agents(
    state: State<'_, AppState>,
    product: AccountProduct,
    name: String,
) -> Result<AgentsProfile, String> {
    import_current_agents_internal(&state, product, &name)
}

#[tauri::command]
pub(super) fn delete_agents_profile(
    state: State<'_, AppState>,
    product: AccountProduct,
    profile_id: String,
) -> Result<(), String> {
    delete_agents_profile_internal(&state, product, &profile_id)
}

fn get_agents_status_internal(
    state: &AppState,
    product: AccountProduct,
) -> Result<AgentsStatus, String> {
    let connection = open_database(state)?;
    let path = instruction_path(state, product)?;
    let file_content = read_optional_file(&path)?;
    let active_id = file_content
        .as_deref()
        .filter(|content| !content.trim().is_empty())
        .map(|content| unique_profile_id_for_content(&connection, product, content))
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
        profiles: list_agents_profiles(&connection, product, active_id.as_deref())?,
        path: path.display().to_string(),
        file_state: file_state.to_string(),
        unmanaged_content: (file_state == "unmanaged").then(|| file_content.unwrap_or_default()),
    })
}

fn create_agents_profile_internal(
    state: &AppState,
    product: AccountProduct,
    name: &str,
    content: &str,
) -> Result<AgentsProfile, String> {
    let connection = open_database(state)?;
    let name = validate_name(&connection, product, name, None)?;
    validate_content(&connection, product, content, None)?;
    let id = Uuid::new_v4().to_string();
    let now = now_millis();
    connection
        .execute(
            "INSERT INTO instruction_profiles (id, product, name, content, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
            params![id, product.as_str(), name, content, now],
        )
        .map_err(database_error)?;
    let active_id = current_instruction_profile_id(state, &connection, product)?;
    get_agents_profile(&connection, product, &id, active_id.as_deref())
}

fn update_agents_profile_internal(
    state: &AppState,
    product: AccountProduct,
    profile_id: &str,
    name: &str,
    content: &str,
) -> Result<AgentsProfile, String> {
    let mut connection = open_database(state)?;
    let name = validate_name(&connection, product, name, Some(profile_id))?;
    get_profile_content(&connection, product, profile_id)?
        .ok_or_else(|| "提示词方案不存在。".to_string())?;
    validate_content(&connection, product, content, Some(profile_id))?;
    let path = instruction_path(state, product)?;
    let backup = read_optional_file(&path)?;
    let active_id = backup
        .as_deref()
        .filter(|current| !current.trim().is_empty())
        .map(|current| unique_profile_id_for_content(&connection, product, current))
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
            "UPDATE instruction_profiles SET name = ?1, content = ?2, updated_at = ?3 WHERE id = ?4 AND product = ?5",
            params![name, content, now, profile_id, product.as_str()],
        )
        .map_err(database_error)
        .and_then(|_| transaction.commit().map_err(database_error));
    if let Err(error) = result {
        if is_active {
            restore_optional_file(&path, backup.as_deref())?;
        }
        return Err(error);
    }
    let active_id = current_instruction_profile_id(state, &connection, product)?;
    get_agents_profile(&connection, product, profile_id, active_id.as_deref())
}

fn activate_agents_profile_internal(
    state: &AppState,
    product: AccountProduct,
    profile_id: &str,
    force: bool,
) -> Result<AgentsProfile, String> {
    let connection = open_database(state)?;
    let content = get_profile_content(&connection, product, profile_id)?
        .ok_or_else(|| "提示词方案不存在。".to_string())?;
    validate_content(&connection, product, &content, Some(profile_id))?;
    let path = instruction_path(state, product)?;
    let backup = read_optional_file(&path)?;
    ensure_unmanaged_file_can_be_replaced(
        &connection,
        product,
        backup.as_deref(),
        &content,
        force,
    )?;
    write_file_atomically(&path, &content)?;
    get_agents_profile(&connection, product, profile_id, Some(profile_id))
}

fn import_current_agents_internal(
    state: &AppState,
    product: AccountProduct,
    name: &str,
) -> Result<AgentsProfile, String> {
    let path = instruction_path(state, product)?;
    let content = read_optional_file(&path)?
        .filter(|content| !content.trim().is_empty())
        .ok_or_else(|| format!("当前 {} 为空，无法同步。", instruction_filename(product)))?;
    let mut connection = open_database(state)?;
    validate_content_length(product, &content)?;
    let matching_ids = profile_ids_for_content(&connection, product, &content)?;
    if matching_ids.len() == 1 {
        return get_agents_profile(
            &connection,
            product,
            &matching_ids[0],
            Some(&matching_ids[0]),
        );
    }
    if matching_ids.len() > 1 {
        return Err("存在多个内容相同的提示词方案，请先处理重复内容。".to_string());
    }
    let name = validate_name(&connection, product, name, None)?;
    validate_content(&connection, product, &content, None)?;
    let id = Uuid::new_v4().to_string();
    let now = now_millis();
    let transaction = connection.transaction().map_err(database_error)?;
    transaction
        .execute(
            "INSERT INTO instruction_profiles (id, product, name, content, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
            params![id, product.as_str(), name, content, now],
        )
        .map_err(database_error)?;
    transaction.commit().map_err(database_error)?;
    get_agents_profile(&connection, product, &id, Some(&id))
}

fn delete_agents_profile_internal(
    state: &AppState,
    product: AccountProduct,
    profile_id: &str,
) -> Result<(), String> {
    let connection = open_database(state)?;
    let changed = connection
        .execute(
            "DELETE FROM instruction_profiles WHERE id = ?1 AND product = ?2",
            params![profile_id, product.as_str()],
        )
        .map_err(database_error)?;
    if changed == 0 {
        return Err("提示词方案不存在。".to_string());
    }
    Ok(())
}

fn ensure_unmanaged_file_can_be_replaced(
    connection: &Connection,
    product: AccountProduct,
    current: Option<&str>,
    next: &str,
    force: bool,
) -> Result<(), String> {
    let current = current.unwrap_or_default();
    let managed = !current.trim().is_empty()
        && unique_profile_id_for_content(connection, product, current)?.is_some();
    if !force && !current.trim().is_empty() && !managed && current != next {
        return Err(format!(
            "检测到未纳管的 {}。请先同步当前文件，或确认后强制覆盖。",
            instruction_filename(product)
        ));
    }
    Ok(())
}

fn validate_content(
    connection: &Connection,
    product: AccountProduct,
    content: &str,
    excluded_id: Option<&str>,
) -> Result<(), String> {
    if content.trim().is_empty() {
        return Err("提示词内容不能为空。".to_string());
    }
    validate_content_length(product, content)?;
    let duplicate = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM instruction_profiles WHERE product = ?1 AND content = ?2 AND (?3 IS NULL OR id <> ?3))",
            params![product.as_str(), content, excluded_id],
            |row| row.get::<_, bool>(0),
        )
        .map_err(database_error)?;
    if duplicate {
        return Err("已存在内容相同的提示词方案。".to_string());
    }
    Ok(())
}

fn validate_content_length(product: AccountProduct, content: &str) -> Result<(), String> {
    if product == AccountProduct::Antigravity
        && content.chars().count() > ANTIGRAVITY_MAX_INSTRUCTION_CHARS
    {
        return Err(format!(
            "Antigravity 提示词不能超过 {ANTIGRAVITY_MAX_INSTRUCTION_CHARS} 个字符。"
        ));
    }
    Ok(())
}

fn profile_ids_for_content(
    connection: &Connection,
    product: AccountProduct,
    content: &str,
) -> Result<Vec<String>, String> {
    let mut statement = connection
        .prepare(
            "SELECT id FROM instruction_profiles WHERE product = ?1 AND content = ?2 ORDER BY created_at ASC LIMIT 2",
        )
        .map_err(database_error)?;
    let profiles = statement
        .query_map(params![product.as_str(), content], |row| row.get(0))
        .map_err(database_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error)?;
    Ok(profiles)
}

fn unique_profile_id_for_content(
    connection: &Connection,
    product: AccountProduct,
    content: &str,
) -> Result<Option<String>, String> {
    let ids = profile_ids_for_content(connection, product, content)?;
    Ok((ids.len() == 1).then(|| ids[0].clone()))
}

fn validate_name(
    connection: &Connection,
    product: AccountProduct,
    name: &str,
    excluded_id: Option<&str>,
) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("方案名称不能为空。".to_string());
    }
    let exists = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM instruction_profiles WHERE product = ?1 AND name = ?2 COLLATE NOCASE AND (?3 IS NULL OR id <> ?3))",
            params![product.as_str(), name, excluded_id],
            |row| row.get::<_, bool>(0),
        )
        .map_err(database_error)?;
    if exists {
        return Err("方案名称已存在。".to_string());
    }
    Ok(name.to_string())
}

fn get_profile_content(
    connection: &Connection,
    product: AccountProduct,
    id: &str,
) -> Result<Option<String>, String> {
    connection
        .query_row(
            "SELECT content FROM instruction_profiles WHERE id = ?1 AND product = ?2",
            params![id, product.as_str()],
            |row| row.get(0),
        )
        .optional()
        .map_err(database_error)
}

fn list_agents_profiles(
    connection: &Connection,
    product: AccountProduct,
    active_id: Option<&str>,
) -> Result<Vec<AgentsProfile>, String> {
    let mut statement = connection
        .prepare("SELECT id, name, content, created_at, updated_at FROM instruction_profiles WHERE product = ?1 ORDER BY created_at ASC")
        .map_err(database_error)?;
    let profiles = statement
        .query_map(params![product.as_str()], |row| {
            agents_profile_from_row(row, active_id)
        })
        .map_err(database_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error)?;
    Ok(profiles)
}

fn get_agents_profile(
    connection: &Connection,
    product: AccountProduct,
    id: &str,
    active_id: Option<&str>,
) -> Result<AgentsProfile, String> {
    connection
        .query_row(
            "SELECT id, name, content, created_at, updated_at FROM instruction_profiles WHERE id = ?1 AND product = ?2",
            params![id, product.as_str()],
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

    const CODEX: AccountProduct = AccountProduct::Codex;

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

    fn create(state: &AppState, name: &str, content: &str) -> AgentsProfile {
        create_agents_profile_internal(state, CODEX, name, content).unwrap()
    }

    fn codex_path(state: &AppState) -> PathBuf {
        instruction_path(state, CODEX).unwrap()
    }

    #[test]
    fn creates_inactive_then_activates_exact_content() {
        let (directory, state) = test_state();
        let profile = create(&state, "Default", "# Rules\n");
        assert!(!codex_path(&state).exists());
        update_agents_profile_internal(&state, CODEX, &profile.id, "Default", "# Updated\n")
            .unwrap();
        assert!(!codex_path(&state).exists());

        activate_agents_profile_internal(&state, CODEX, &profile.id, false).unwrap();

        assert_eq!(
            fs::read_to_string(codex_path(&state)).unwrap(),
            "# Updated\n"
        );
        let status = get_agents_status_internal(&state, CODEX).unwrap();
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
        let first = create(&state, "First", "first");
        let second = create(&state, "Second", "second");
        activate_agents_profile_internal(&state, CODEX, &first.id, false).unwrap();
        write_file_atomically(&codex_path(&state), "external").unwrap();

        assert!(
            activate_agents_profile_internal(&state, CODEX, &second.id, false)
                .unwrap_err()
                .contains("未纳管")
        );
        let imported = import_current_agents_internal(&state, CODEX, "Imported").unwrap();
        assert_eq!(imported.content, "external");
        assert!(imported.is_active);
        write_file_atomically(&codex_path(&state), "external-again").unwrap();
        activate_agents_profile_internal(&state, CODEX, &second.id, true).unwrap();
        assert_eq!(fs::read_to_string(codex_path(&state)).unwrap(), "second");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn updates_active_file_and_leaves_it_when_deleted() {
        let (directory, state) = test_state();
        let profile = create(&state, "Default", "old");
        activate_agents_profile_internal(&state, CODEX, &profile.id, false).unwrap();

        update_agents_profile_internal(&state, CODEX, &profile.id, "Renamed", "new").unwrap();
        delete_agents_profile_internal(&state, CODEX, &profile.id).unwrap();

        assert_eq!(fs::read_to_string(codex_path(&state)).unwrap(), "new");
        assert_eq!(
            get_agents_status_internal(&state, CODEX)
                .unwrap()
                .file_state,
            "unmanaged"
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn rejects_empty_and_duplicate_content() {
        let (directory, state) = test_state();
        assert!(create_agents_profile_internal(&state, CODEX, "Empty", " \n").is_err());
        create(&state, "First", "same");
        assert!(create_agents_profile_internal(&state, CODEX, "Duplicate", "same").is_err());
        let second = create(&state, "Second", "other");
        assert!(
            update_agents_profile_internal(&state, CODEX, &second.id, "Second", "same").is_err()
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn isolates_profiles_and_files_by_product() {
        let (directory, state) = test_state();
        let codex = create_agents_profile_internal(&state, CODEX, "Default", "same").unwrap();
        let antigravity =
            create_agents_profile_internal(&state, AccountProduct::Antigravity, "Default", "same")
                .unwrap();

        activate_agents_profile_internal(&state, CODEX, &codex.id, false).unwrap();
        activate_agents_profile_internal(
            &state,
            AccountProduct::Antigravity,
            &antigravity.id,
            false,
        )
        .unwrap();

        assert_eq!(
            get_agents_status_internal(&state, CODEX)
                .unwrap()
                .profiles
                .len(),
            1
        );
        assert_eq!(
            get_agents_status_internal(&state, AccountProduct::Antigravity)
                .unwrap()
                .profiles
                .len(),
            1
        );
        assert_eq!(fs::read_to_string(codex_path(&state)).unwrap(), "same");
        assert_eq!(
            fs::read_to_string(instruction_path(&state, AccountProduct::Antigravity).unwrap())
                .unwrap(),
            "same"
        );
        assert!(delete_agents_profile_internal(&state, AccountProduct::Grok, &codex.id).is_err());
        fs::remove_dir_all(directory).unwrap();
    }
}
