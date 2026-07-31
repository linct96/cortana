use super::types::AgentsProfile;
use crate::platform::{db::database_error, state::AccountProduct};
use rusqlite::{params, Connection, OptionalExtension};

pub(super) fn get_profile_content(
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

pub(super) fn list_agents_profiles(
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

pub(super) fn get_agents_profile(
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
