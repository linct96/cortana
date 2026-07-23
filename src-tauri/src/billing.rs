use super::*;

const MODELS_DEV_API_URL: &str = "https://models.dev/api.json";
const MODELS_DEV_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_MODELS_DEV_BYTES: u64 = 8 * 1024 * 1024;
const MAX_PRICING_BATCH: usize = 500;
const MODELS_DEV_PROVIDERS: [&str; 5] = ["openai", "anthropic", "google", "deepseek", "xai"];

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ModelPricing {
    pub(super) model_id: String,
    pub(super) display_name: String,
    pub(super) input_cost_per_million: String,
    pub(super) output_cost_per_million: String,
    pub(super) cache_read_cost_per_million: String,
    pub(super) cache_write_cost_per_million: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ModelsDevPricing {
    provider: String,
    model_id: String,
    display_name: String,
    release_date: String,
    input_cost_per_million: String,
    output_cost_per_million: String,
    cache_read_cost_per_million: String,
    cache_write_cost_per_million: String,
}

#[tauri::command]
pub(super) fn list_model_pricing(state: State<'_, AppState>) -> Result<Vec<ModelPricing>, String> {
    let connection = db::open_database(&state)?;
    list_model_pricing_from(&connection)
}

#[tauri::command]
pub(super) fn save_model_pricing(
    state: State<'_, AppState>,
    items: Vec<ModelPricing>,
) -> Result<(), String> {
    let mut connection = db::open_database(&state)?;
    save_model_pricing_to(&mut connection, items)
}

#[tauri::command]
pub(super) fn delete_model_pricing(
    state: State<'_, AppState>,
    model_id: String,
) -> Result<(), String> {
    let connection = db::open_database(&state)?;
    connection
        .execute(
            "DELETE FROM model_pricing WHERE model_id = ?1",
            params![normalize_model_id(&model_id)],
        )
        .map_err(db::database_error)?;
    Ok(())
}

#[tauri::command]
pub(super) async fn fetch_models_dev_pricing() -> Result<Vec<ModelsDevPricing>, String> {
    tauri::async_runtime::spawn_blocking(fetch_models_dev_pricing_inner)
        .await
        .map_err(|error| error.to_string())?
}

fn fetch_models_dev_pricing_inner() -> Result<Vec<ModelsDevPricing>, String> {
    let client = Client::builder()
        .timeout(MODELS_DEV_TIMEOUT)
        .build()
        .map_err(|error| error.to_string())?;
    let response = client
        .get(MODELS_DEV_API_URL)
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|error| format!("无法加载 models.dev 定价：{error}"))?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_MODELS_DEV_BYTES)
    {
        return Err("models.dev 返回的数据过大。".to_string());
    }
    let mut body = Vec::new();
    response
        .take(MAX_MODELS_DEV_BYTES + 1)
        .read_to_end(&mut body)
        .map_err(|error| format!("无法读取 models.dev 定价：{error}"))?;
    if body.len() as u64 > MAX_MODELS_DEV_BYTES {
        return Err("models.dev 返回的数据过大。".to_string());
    }
    let response = serde_json::from_slice::<Value>(&body)
        .map_err(|error| format!("models.dev 返回了无效数据：{error}"))?;
    parse_models_dev_pricing(&response)
}

fn parse_models_dev_pricing(response: &Value) -> Result<Vec<ModelsDevPricing>, String> {
    let mut pricing = MODELS_DEV_PROVIDERS
        .iter()
        .filter_map(|provider_id| response.get(provider_id))
        .flat_map(|provider| {
            let provider_name = provider
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            provider
                .get("models")
                .and_then(Value::as_object)
                .into_iter()
                .flatten()
                .map(move |model| (provider_name, model))
        })
        .filter_map(|(provider, (model_id, model))| {
            let cost = model.get("cost")?.as_object()?;
            if !cost.get("input").is_some_and(Value::is_number)
                && !cost.get("output").is_some_and(Value::is_number)
            {
                return None;
            }
            Some(ModelsDevPricing {
                provider: provider.to_string(),
                model_id: normalize_model_id(model_id),
                display_name: model
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or(model_id)
                    .to_string(),
                release_date: model
                    .get("release_date")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                input_cost_per_million: json_price(cost.get("input")),
                output_cost_per_million: json_price(cost.get("output")),
                cache_read_cost_per_million: json_price(cost.get("cache_read")),
                cache_write_cost_per_million: json_price(cost.get("cache_write")),
            })
        })
        .collect::<Vec<_>>();
    if pricing.is_empty() {
        return Err("models.dev 中没有常见供应商的模型数据。".to_string());
    }
    pricing.sort_by(|left, right| {
        left.provider.cmp(&right.provider).then_with(|| {
            right
                .release_date
                .cmp(&left.release_date)
                .then_with(|| left.display_name.cmp(&right.display_name))
        })
    });
    Ok(pricing)
}

fn json_price(value: Option<&Value>) -> String {
    let value = value.and_then(Value::as_f64).unwrap_or_default();
    if !value.is_finite() || value < 0.0 {
        return "0".to_string();
    }
    let formatted = format!("{value:.6}");
    let trimmed = formatted.trim_end_matches('0').trim_end_matches('.');
    if trimmed.is_empty() {
        "0".to_string()
    } else {
        trimmed.to_string()
    }
}

fn list_model_pricing_from(connection: &Connection) -> Result<Vec<ModelPricing>, String> {
    let mut statement = connection
        .prepare(
            "SELECT model_id, display_name, input_cost_per_million, output_cost_per_million,
                    cache_read_cost_per_million, cache_write_cost_per_million
             FROM model_pricing ORDER BY display_name COLLATE NOCASE, model_id",
        )
        .map_err(db::database_error)?;
    let pricing = statement
        .query_map([], |row| {
            Ok(ModelPricing {
                model_id: row.get(0)?,
                display_name: row.get(1)?,
                input_cost_per_million: row.get(2)?,
                output_cost_per_million: row.get(3)?,
                cache_read_cost_per_million: row.get(4)?,
                cache_write_cost_per_million: row.get(5)?,
            })
        })
        .map_err(db::database_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db::database_error)?;
    Ok(pricing)
}

fn save_model_pricing_to(
    connection: &mut Connection,
    items: Vec<ModelPricing>,
) -> Result<(), String> {
    if items.len() > MAX_PRICING_BATCH {
        return Err(format!("每次最多保存 {MAX_PRICING_BATCH} 个模型定价。"));
    }
    let items = items
        .into_iter()
        .map(validate_pricing)
        .collect::<Result<Vec<_>, _>>()?;
    let transaction = connection.transaction().map_err(db::database_error)?;
    for item in items {
        transaction
            .execute(
                "INSERT INTO model_pricing (
                    model_id, display_name, input_cost_per_million, output_cost_per_million,
                    cache_read_cost_per_million, cache_write_cost_per_million
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(model_id) DO UPDATE SET
                    display_name = excluded.display_name,
                    input_cost_per_million = excluded.input_cost_per_million,
                    output_cost_per_million = excluded.output_cost_per_million,
                    cache_read_cost_per_million = excluded.cache_read_cost_per_million,
                    cache_write_cost_per_million = excluded.cache_write_cost_per_million",
                params![
                    item.model_id,
                    item.display_name,
                    item.input_cost_per_million,
                    item.output_cost_per_million,
                    item.cache_read_cost_per_million,
                    item.cache_write_cost_per_million,
                ],
            )
            .map_err(db::database_error)?;
    }
    transaction.commit().map_err(db::database_error)
}

fn validate_pricing(mut item: ModelPricing) -> Result<ModelPricing, String> {
    item.model_id = normalize_model_id(&item.model_id);
    item.display_name = item.display_name.trim().to_string();
    if item.model_id.is_empty() {
        return Err("模型 ID 不能为空。".to_string());
    }
    if item.model_id.len() > 200 {
        return Err("模型 ID 不能超过 200 个字符。".to_string());
    }
    if item.display_name.is_empty() {
        return Err("显示名称不能为空。".to_string());
    }
    if item.display_name.len() > 200 {
        return Err("显示名称不能超过 200 个字符。".to_string());
    }
    for (label, value) in [
        ("输入", &mut item.input_cost_per_million),
        ("输出", &mut item.output_cost_per_million),
        ("缓存读取", &mut item.cache_read_cost_per_million),
        ("缓存写入", &mut item.cache_write_cost_per_million),
    ] {
        *value = value.trim().to_string();
        if value.len() > 32 || !is_decimal(value) {
            return Err(format!("{label}价格必须是非负数。"));
        }
        let parsed = value
            .parse::<f64>()
            .map_err(|_| format!("{label}价格必须是非负数。"))?;
        if !parsed.is_finite() || parsed < 0.0 {
            return Err(format!("{label}价格必须是非负数。"));
        }
    }
    Ok(item)
}

fn is_decimal(value: &str) -> bool {
    let mut parts = value.split('.');
    let whole = parts.next().unwrap_or_default();
    let fraction = parts.next();
    !whole.is_empty()
        && whole.bytes().all(|byte| byte.is_ascii_digit())
        && fraction.is_none_or(|digits| {
            !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
        })
        && parts.next().is_none()
}

pub(super) fn normalize_model_id(model_id: &str) -> String {
    model_id
        .rsplit('/')
        .next()
        .unwrap_or(model_id)
        .split(':')
        .next()
        .unwrap_or(model_id)
        .trim()
        .to_ascii_lowercase()
}

pub(super) fn load_pricing(
    connection: &Connection,
) -> Result<HashMap<String, ModelPricing>, String> {
    Ok(list_model_pricing_from(connection)?
        .into_iter()
        .map(|pricing| (pricing.model_id.clone(), pricing))
        .collect())
}

pub(super) fn estimated_cost(
    pricing: &HashMap<String, ModelPricing>,
    model_id: &str,
    input_tokens: u64,
    cached_input_tokens: u64,
    cache_write_input_tokens: u64,
    output_tokens: u64,
) -> Option<f64> {
    let normalized = normalize_model_id(model_id);
    let price = pricing.get(&normalized).or_else(|| {
        normalized
            .split_once('@')
            .and_then(|(base, _)| pricing.get(base))
    })?;
    let input_price = price.input_cost_per_million.parse::<f64>().ok()?;
    let output_price = price.output_cost_per_million.parse::<f64>().ok()?;
    let cache_read_price = price.cache_read_cost_per_million.parse::<f64>().ok()?;
    let cache_write_price = price.cache_write_cost_per_million.parse::<f64>().ok()?;
    let fresh_input_tokens = input_tokens
        .saturating_sub(cached_input_tokens)
        .saturating_sub(cache_write_input_tokens);
    Some(
        (fresh_input_tokens as f64 * input_price
            + cached_input_tokens as f64 * cache_read_price
            + cache_write_input_tokens as f64 * cache_write_price
            + output_tokens as f64 * output_price)
            / 1_000_000.0,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pricing(model_id: &str, input: &str, output: &str, cache_read: &str) -> ModelPricing {
        ModelPricing {
            model_id: model_id.to_string(),
            display_name: model_id.to_string(),
            input_cost_per_million: input.to_string(),
            output_cost_per_million: output.to_string(),
            cache_read_cost_per_million: cache_read.to_string(),
            cache_write_cost_per_million: "0".to_string(),
        }
    }

    #[test]
    fn saves_overwrites_and_rejects_invalid_batches() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE model_pricing (
                    model_id TEXT PRIMARY KEY NOT NULL, display_name TEXT NOT NULL,
                    input_cost_per_million TEXT NOT NULL, output_cost_per_million TEXT NOT NULL,
                    cache_read_cost_per_million TEXT NOT NULL, cache_write_cost_per_million TEXT NOT NULL
                )",
            )
            .unwrap();

        save_model_pricing_to(
            &mut connection,
            vec![pricing("OpenAI/GPT-5.4", "2.5", "15", "0.25")],
        )
        .unwrap();
        save_model_pricing_to(&mut connection, vec![pricing("gpt-5.4", "3", "16", "0.3")]).unwrap();
        assert_eq!(
            list_model_pricing_from(&connection).unwrap()[0].input_cost_per_million,
            "3"
        );

        let mut invalid = pricing("gpt-5", "1", "2", "0");
        invalid.output_cost_per_million = "-1".to_string();
        assert!(save_model_pricing_to(
            &mut connection,
            vec![pricing("gpt-5-mini", "1", "2", "0"), invalid],
        )
        .is_err());
        assert_eq!(list_model_pricing_from(&connection).unwrap().len(), 1);
    }

    #[test]
    fn parses_models_dev_base_prices_and_calculates_cached_input_once() {
        let response = json!({
            "openai": { "name": "OpenAI", "models": {
                "gpt-5.4": {
                    "name": "GPT-5.4",
                    "release_date": "2026-03-05",
                    "cost": { "input": 2.5, "output": 15, "cache_read": 0.25,
                              "tiers": [{ "input": 5 }] }
                }
            }},
            "anthropic": { "name": "Anthropic", "models": {
                "claude-sonnet": {
                    "name": "Claude Sonnet",
                    "release_date": "2026-02-01",
                    "cost": { "input": 3, "output": 15 }
                }
            }}
        });
        let parsed = parse_models_dev_pricing(&response).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].provider, "Anthropic");
        assert_eq!(parsed[1].input_cost_per_million, "2.5");
        assert_eq!(parsed[1].cache_write_cost_per_million, "0");

        let prices = HashMap::from([(
            "gpt-5.4".to_string(),
            pricing("gpt-5.4", "2.5", "15", "0.25"),
        )]);
        let cost = estimated_cost(
            &prices,
            "openai/gpt-5.4@high",
            1_000_000,
            200_000,
            0,
            100_000,
        )
        .unwrap();
        assert!((cost - 3.55).abs() < f64::EPSILON);

        let mut price = pricing("gpt-5.4", "2.5", "15", "0.25");
        price.cache_write_cost_per_million = "1".to_string();
        let prices = HashMap::from([("gpt-5.4".to_string(), price)]);
        let cost =
            estimated_cost(&prices, "gpt-5.4", 1_000_000, 200_000, 100_000, 100_000).unwrap();
        assert!((cost - 3.4).abs() < f64::EPSILON);
    }
}
