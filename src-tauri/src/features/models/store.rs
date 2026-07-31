use super::{
    remote::normalize_models,
    types::{ModelAssignment, ModelEntry, ModelProfile, ModelProfilesStatus},
};
use crate::{
    platform::{
        db::{database_error, open_database},
        state::{now_millis, AccountProduct, AppState, ACCOUNT_TYPE_RELAY},
    },
    products::grok as product_grok,
};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use std::collections::HashSet;
use uuid::Uuid;

pub(crate) fn model_profiles_status(
    state: &AppState,
    product: AccountProduct,
) -> Result<ModelProfilesStatus, String> {
    if !matches!(
        product,
        AccountProduct::Codex | AccountProduct::Claude | AccountProduct::Grok
    ) {
        return Err("该产品暂不支持自定义模型。".to_string());
    }
    let connection = open_database(state)?;
    Ok(ModelProfilesStatus {
        profiles: list_model_profiles(&connection, product)?,
        relay_accounts: list_relay_accounts(&connection, product)?,
    })
}

pub(crate) fn set_account_model_profile(
    state: &AppState,
    product: AccountProduct,
    account_id: &str,
    model_profile_id: Option<&str>,
    default_model_id: Option<&str>,
) -> Result<(), String> {
    let mut connection = open_database(state)?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(database_error)?;
    let account_type = transaction
        .query_row(
            "SELECT account_type FROM accounts WHERE id = ?1 AND product = ?2",
            params![account_id, product.as_str()],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(database_error)?
        .ok_or_else(|| "账户不存在。".to_string())?;
    if account_type != ACCOUNT_TYPE_RELAY {
        return Err(format!(
            "只有 {} 中转账号可以关联模型方案。",
            product.display_name()
        ));
    }
    validate_model_profile_selection(&transaction, product, model_profile_id, default_model_id)?;
    transaction
        .execute(
            "UPDATE accounts SET model_profile_id = ?1, default_model_id = ?2, updated_at = ?3 WHERE id = ?4",
            params![model_profile_id, default_model_id, now_millis(), account_id],
        )
        .map_err(database_error)?;
    transaction.commit().map_err(database_error)
}

pub(crate) fn validate_model_selection(
    state: &AppState,
    product: AccountProduct,
    model_profile_id: Option<&str>,
    default_model_id: Option<&str>,
) -> Result<(), String> {
    validate_model_profile_selection(
        &open_database(state)?,
        product,
        model_profile_id,
        default_model_id,
    )
}

pub(super) fn validate_model_profile_selection(
    connection: &Connection,
    product: AccountProduct,
    model_profile_id: Option<&str>,
    default_model_id: Option<&str>,
) -> Result<(), String> {
    if let Some(profile_id) = model_profile_id {
        let models_json = connection
            .query_row(
                "SELECT models_json FROM model_profiles WHERE id = ?1 AND product = ?2",
                params![profile_id, product.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(database_error)?
            .ok_or_else(|| "模型方案不存在。".to_string())?;
        let models: Vec<ModelEntry> =
            serde_json::from_str(&models_json).map_err(|_| "模型方案数据已损坏。".to_string())?;
        if models.is_empty() {
            return Err("模型方案至少需要一个模型。".to_string());
        }
        if !default_model_id.is_some_and(|id| models.iter().any(|model| model.id == id)) {
            return Err("请选择方案内的默认模型。".to_string());
        }
    } else if default_model_id.is_some() {
        return Err("未选择模型方案时不能设置默认模型。".to_string());
    }
    Ok(())
}

pub(crate) fn save_model_profile(
    state: &AppState,
    product: AccountProduct,
    profile_id: Option<&str>,
    requested_name: &str,
    models: Vec<ModelEntry>,
    assignments: Vec<ModelAssignment>,
    force_reassign: bool,
) -> Result<ModelProfile, String> {
    let _grok_guard = if product == AccountProduct::Grok {
        Some(product_grok::lock_configuration()?)
    } else {
        None
    };
    let name = requested_name.trim();
    if name.is_empty() {
        return Err("模型方案名称不能为空。".to_string());
    }
    let models = normalize_models(product, models)?;
    validate_assignments(&models, &assignments)?;
    let mut connection = open_database(state)?;
    let mut enabled_grok_accounts = if product == AccountProduct::Grok {
        product_grok::enabled_relay_profile_ids(state, &connection)?
    } else {
        Vec::new()
    };
    let had_enabled_grok_accounts = !enabled_grok_accounts.is_empty();
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(database_error)?;
    validate_account_assignments(
        &transaction,
        product,
        profile_id,
        &assignments,
        force_reassign,
    )?;
    let id = profile_id
        .map(str::to_string)
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let models_json = serde_json::to_string(&models).map_err(|error| error.to_string())?;
    let now = now_millis();
    let result = if profile_id.is_some() {
        transaction.execute(
            "UPDATE model_profiles SET name = ?1, models_json = ?2, updated_at = ?3 WHERE id = ?4 AND product = ?5",
            params![name, models_json, now, id, product.as_str()],
        )
    } else {
        transaction.execute(
            "INSERT INTO model_profiles (id, product, name, models_json, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
            params![id, product.as_str(), name, models_json, now],
        )
    };
    let changed = result.map_err(|error| {
        if error.to_string().contains("UNIQUE") {
            "模型方案名称已存在。".to_string()
        } else {
            database_error(error)
        }
    })?;
    if changed == 0 {
        return Err("模型方案不存在。".to_string());
    }
    transaction
        .execute(
            "UPDATE accounts SET model_profile_id = NULL, default_model_id = NULL WHERE model_profile_id = ?1",
            params![id],
        )
        .map_err(database_error)?;
    for assignment in &assignments {
        transaction
            .execute(
                "UPDATE accounts SET model_profile_id = ?1, default_model_id = ?2, updated_at = ?3 WHERE id = ?4",
                params![id, assignment.default_model_id, now, assignment.account_id],
            )
            .map_err(database_error)?;
    }
    let grok_backup = if had_enabled_grok_accounts {
        enabled_grok_accounts = enabled_grok_accounts
            .into_iter()
            .map(|account_id| {
                transaction
                .query_row(
                    "SELECT model_profile_id IS NOT NULL AND default_model_id IS NOT NULL FROM accounts WHERE id = ?1 AND product = 'grok' AND account_type = 'relay'",
                    params![&account_id],
                    |row| row.get::<_, bool>(0),
                )
                .optional()
                .map_err(database_error)
                .map(|enabled| enabled.unwrap_or(false).then_some(account_id))
            })
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .collect();
        let backup = product_grok::rebuild_enabled_configuration(
            state,
            &transaction,
            &enabled_grok_accounts,
        )?;
        if let Err(error) =
            product_grok::set_enabled_relay_profile_ids(&transaction, &enabled_grok_accounts)
        {
            product_grok::restore_configuration(state, &backup)?;
            return Err(error);
        }
        Some(backup)
    } else {
        None
    };
    if let Err(error) = transaction.commit().map_err(database_error) {
        if let Some(backup) = grok_backup.as_ref() {
            product_grok::restore_configuration(state, backup)?;
        }
        return Err(error);
    }
    list_model_profiles(&connection, product)?
        .into_iter()
        .find(|profile| profile.id == id)
        .ok_or_else(|| "模型方案不存在。".to_string())
}

pub(super) fn validate_assignments(
    models: &[ModelEntry],
    assignments: &[ModelAssignment],
) -> Result<(), String> {
    let model_ids = models
        .iter()
        .map(|model| model.id.as_str())
        .collect::<HashSet<_>>();
    let mut account_ids = HashSet::new();
    for assignment in assignments {
        if !account_ids.insert(assignment.account_id.as_str()) {
            return Err("关联账号重复。".to_string());
        }
        if !assignment
            .default_model_id
            .as_deref()
            .is_some_and(|id| model_ids.contains(id))
        {
            return Err(format!(
                "账号“{}”必须选择方案内的默认模型。",
                assignment.account_alias
            ));
        }
    }
    Ok(())
}

pub(super) fn validate_account_assignments(
    transaction: &Transaction<'_>,
    product: AccountProduct,
    profile_id: Option<&str>,
    assignments: &[ModelAssignment],
    force_reassign: bool,
) -> Result<(), String> {
    for assignment in assignments {
        let row = transaction
            .query_row(
                "SELECT alias, account_type, model_profile_id FROM accounts WHERE id = ?1 AND product = ?2",
                params![assignment.account_id, product.as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(database_error)?
            .ok_or_else(|| "关联账号不存在。".to_string())?;
        if row.1 != ACCOUNT_TYPE_RELAY {
            return Err(format!(
                "账号“{}”不是 {} 中转账号。",
                row.0,
                product.display_name()
            ));
        }
        if !force_reassign && row.2.as_deref().is_some_and(|id| Some(id) != profile_id) {
            return Err(format!("账号“{}”已关联其他模型方案。", row.0));
        }
    }
    Ok(())
}

pub(super) fn list_model_profiles(
    connection: &Connection,
    product: AccountProduct,
) -> Result<Vec<ModelProfile>, String> {
    let rows = connection
        .prepare(
            "SELECT id, name, models_json FROM model_profiles WHERE product = ?1 ORDER BY created_at ASC",
        )
        .map_err(database_error)?
        .query_map(params![product.as_str()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(database_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error)?;
    rows.into_iter()
        .map(|(id, name, models_json)| {
            Ok(ModelProfile {
                assignments: assignments_for_profile(connection, product, &id)?,
                id,
                name,
                models: serde_json::from_str(&models_json)
                    .map_err(|_| "模型方案数据已损坏。".to_string())?,
            })
        })
        .collect()
}

pub(super) fn assignments_for_profile(
    connection: &Connection,
    product: AccountProduct,
    profile_id: &str,
) -> Result<Vec<ModelAssignment>, String> {
    connection
        .prepare(
            "SELECT id, alias, default_model_id FROM accounts WHERE product = ?1 AND account_type = 'relay' AND model_profile_id = ?2 ORDER BY sort_order, created_at",
        )
        .map_err(database_error)?
        .query_map(params![product.as_str(), profile_id], |row| {
            Ok(ModelAssignment {
                account_id: row.get(0)?,
                account_alias: row.get(1)?,
                default_model_id: row.get(2)?,
            })
        })
        .map_err(database_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error)
}

pub(super) fn list_relay_accounts(
    connection: &Connection,
    product: AccountProduct,
) -> Result<Vec<ModelAssignment>, String> {
    connection
        .prepare(
            "SELECT id, alias, default_model_id FROM accounts WHERE product = ?1 AND account_type = 'relay' ORDER BY sort_order, created_at",
        )
        .map_err(database_error)?
        .query_map(params![product.as_str()], |row| {
            Ok(ModelAssignment {
                account_id: row.get(0)?,
                account_alias: row.get(1)?,
                default_model_id: row.get(2)?,
            })
        })
        .map_err(database_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error)
}
