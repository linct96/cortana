use super::{
    remote::fetch_relay_models_internal,
    store::{model_profiles_status, save_model_profile},
    types::{ModelAssignment, ModelEntry, ModelProfile, ModelProfilesStatus, RelayModelOption},
};
use crate::platform::{
    db::{database_error, open_database},
    state::{AccountProduct, AppState},
};
use rusqlite::params;
use tauri::State;

pub(crate) async fn get_model_profiles_status(
    state: State<'_, AppState>,
    product: AccountProduct,
) -> Result<ModelProfilesStatus, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || model_profiles_status(&state, product))
        .await
        .map_err(|error| error.to_string())?
}

pub(crate) fn create_model_profile(
    state: State<'_, AppState>,
    product: AccountProduct,
    name: String,
    models: Vec<ModelEntry>,
    assignments: Vec<ModelAssignment>,
    force_reassign: bool,
) -> Result<ModelProfile, String> {
    save_model_profile(
        &state,
        product,
        None,
        &name,
        models,
        assignments,
        force_reassign,
    )
}

pub(crate) fn update_model_profile(
    state: State<'_, AppState>,
    product: AccountProduct,
    profile_id: String,
    name: String,
    models: Vec<ModelEntry>,
    assignments: Vec<ModelAssignment>,
    force_reassign: bool,
) -> Result<ModelProfile, String> {
    save_model_profile(
        &state,
        product,
        Some(&profile_id),
        &name,
        models,
        assignments,
        force_reassign,
    )
}

pub(crate) fn delete_model_profile(
    state: State<'_, AppState>,
    product: AccountProduct,
    profile_id: String,
) -> Result<(), String> {
    delete_model_profile_internal(&state, product, &profile_id)
}

pub(super) fn delete_model_profile_internal(
    state: &AppState,
    product: AccountProduct,
    profile_id: &str,
) -> Result<(), String> {
    let connection = open_database(state)?;
    let linked = connection
        .query_row(
            "SELECT COUNT(*) FROM accounts WHERE model_profile_id = ?1",
            params![profile_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(database_error)?;
    if linked > 0 {
        return Err("该模型方案仍有关联账号，请先解除关联。".to_string());
    }
    let changed = connection
        .execute(
            "DELETE FROM model_profiles WHERE id = ?1 AND product = ?2",
            params![profile_id, product.as_str()],
        )
        .map_err(database_error)?;
    if changed == 0 {
        return Err("模型方案不存在。".to_string());
    }
    Ok(())
}

pub(crate) async fn fetch_relay_models(
    state: State<'_, AppState>,
    product: AccountProduct,
    account_id: String,
) -> Result<Vec<RelayModelOption>, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        fetch_relay_models_internal(&state, product, &account_id)
    })
    .await
    .map_err(|error| error.to_string())?
}
