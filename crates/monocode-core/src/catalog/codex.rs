use super::{model, Model};
use serde_json::Value;
use std::{collections::HashSet, io::Read, time::Duration};

const MAX_CATALOG_BYTES: u64 = 16 * 1024 * 1024;

pub(super) fn custom_models(config: &Value) -> Result<Option<Vec<Model>>, String> {
    let Some(provider_id) = config["model_provider"].as_str() else {
        return Ok(None);
    };
    let provider = &config["model_providers"][provider_id];
    let Some(base_url) = provider["base_url"].as_str() else {
        return Ok(None);
    };
    let name = provider["name"].as_str().unwrap_or(provider_id);
    let mut url = url::Url::parse(base_url)
        .map_err(|_| format!("{name} has an invalid model provider URL"))?;
    url.set_path(&format!("{}/models", url.path().trim_end_matches('/')));
    for (key, value) in provider["query_params"].as_object().into_iter().flatten() {
        if let Some(value) = value.as_str() {
            url.query_pairs_mut().append_pair(key, value);
        }
    }
    let catalog = fetch_catalog(provider, &url, name)?;
    let selected = config["model"].as_str().filter(|id| !id.is_empty());
    let mut models = parse_models(&catalog, selected);
    if models.is_empty() {
        return Err(format!("{name} returned an empty model catalog"));
    }
    if url.host_str() == Some("ai-gateway.vercel.sh") && url.path() == "/codex/v1/models" {
        url.set_path("/v1/models");
        if let Ok(catalog) = fetch_catalog(provider, &url, name) {
            add_gateway_aliases(&mut models, &catalog, selected);
        }
    }
    if let Some(selected) = selected.filter(|id| !models.iter().any(|model| model.id == *id)) {
        let mut configured = model(selected, selected);
        configured.default = true;
        models.push(configured);
    }
    for model in &mut models {
        model.provider_name = Some(name.to_owned());
    }
    models.sort_by_key(|model| !model.default);
    Ok(Some(models))
}

pub(super) fn merge_custom_models(models: &mut Vec<Model>, custom_models: Vec<Model>) {
    let custom_default = custom_models.iter().any(|model| model.default);
    if custom_default {
        for model in models.iter_mut() {
            model.default = false;
        }
    }
    for mut custom in custom_models {
        if let Some(index) = models.iter().position(|model| model.id == custom.id) {
            custom.default |= models[index].default;
            models[index] = custom;
        } else {
            models.push(custom);
        }
    }
}

fn fetch_catalog(provider: &Value, url: &url::Url, name: &str) -> Result<Value, String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(10))
        .redirects(0)
        .build();
    let mut request = agent.get(url.as_str());
    let token = provider["env_key"]
        .as_str()
        .and_then(|key| std::env::var(key).ok())
        .or_else(|| {
            provider["experimental_bearer_token"]
                .as_str()
                .map(str::to_owned)
        });
    if let Some(token) = token.filter(|token| !token.is_empty()) {
        request = request.set("Authorization", &format!("Bearer {token}"));
    }
    for (header, value) in provider["http_headers"].as_object().into_iter().flatten() {
        if let Some(value) = value.as_str() {
            request = request.set(header, value);
        }
    }
    for (header, key) in provider["env_http_headers"]
        .as_object()
        .into_iter()
        .flatten()
    {
        if let Some(value) = key.as_str().and_then(|key| std::env::var(key).ok()) {
            request = request.set(header, &value);
        }
    }
    let response = request
        .call()
        .map_err(|_| format!("Could not load models from {name}"))?;
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(MAX_CATALOG_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| format!("Could not read models from {name}"))?;
    if bytes.len() as u64 > MAX_CATALOG_BYTES {
        return Err(format!("{name} returned a model catalog that is too large"));
    }
    serde_json::from_slice(&bytes).map_err(|_| format!("{name} returned an invalid model catalog"))
}

fn add_gateway_aliases(models: &mut Vec<Model>, catalog: &Value, selected: Option<&str>) {
    for alias in parse_models(catalog, selected) {
        let Some(base_id) = alias.id.strip_suffix("-fast") else {
            continue;
        };
        if models.iter().any(|model| model.id == alias.id) {
            continue;
        }
        let Some(base) = models.iter().find(|model| model.id == base_id) else {
            continue;
        };
        let mut model = base.clone();
        model.id = alias.id;
        model.name = alias.name;
        model.default = alias.default;
        model.tiers = alias.tiers;
        models.push(model);
    }
}

fn parse_models(catalog: &Value, selected: Option<&str>) -> Vec<Model> {
    let mut seen = HashSet::new();
    let mut models: Vec<_> = catalog["models"]
        .as_array()
        .or_else(|| catalog["data"].as_array())
        .into_iter()
        .flatten()
        .filter_map(|row| {
            let id = row["slug"]
                .as_str()
                .or_else(|| row["model"].as_str())
                .or_else(|| row["id"].as_str())?;
            if row["hidden"] == true
                || row["visibility"]
                    .as_str()
                    .is_some_and(|value| value != "list")
                || row["supported_in_api"] == false
                || row["type"]
                    .as_str()
                    .is_some_and(|value| value != "language")
                || !seen.insert(id.to_owned())
            {
                return None;
            }
            let name = row["display_name"]
                .as_str()
                .or_else(|| row["displayName"].as_str())
                .or_else(|| row["name"].as_str())
                .unwrap_or(id);
            let mut model = model(id, name);
            model.default =
                selected == Some(id) || (selected.is_none() && row["isDefault"] == true);
            model.efforts = row["supported_reasoning_levels"]
                .as_array()
                .or_else(|| row["supportedReasoningEfforts"].as_array())
                .into_iter()
                .flatten()
                .filter_map(|effort| {
                    effort["effort"]
                        .as_str()
                        .or_else(|| effort["reasoningEffort"].as_str())
                        .or_else(|| effort.as_str())
                        .map(str::to_owned)
                })
                .collect();
            model.default_effort = row["default_reasoning_level"]
                .as_str()
                .or_else(|| row["defaultReasoningEffort"].as_str())
                .map(str::to_owned);
            model.tiers = row["service_tiers"]
                .as_array()
                .or_else(|| row["serviceTiers"].as_array())
                .filter(|tiers| !tiers.is_empty())
                .or_else(|| row["additional_speed_tiers"].as_array())
                .or_else(|| row["additionalSpeedTiers"].as_array())
                .into_iter()
                .flatten()
                .filter_map(|tier| {
                    let id = tier["id"].as_str().or_else(|| tier.as_str())?;
                    (id != "default").then(|| {
                        (
                            id.into(),
                            tier["name"]
                                .as_str()
                                .unwrap_or(if id == "fast" { "Fast" } else { id })
                                .into(),
                        )
                    })
                })
                .collect();
            Some(model)
        })
        .collect();
    models.sort_by_key(|model| !model.default);
    models
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn merging_keeps_builtin_models_and_custom_identity_without_duplicate_ids() {
        let mut builtin = model("gpt-6.1-sol", "GPT-6.1 Sol");
        builtin.default = true;
        let mut models = vec![builtin, model("openai/gpt-6.1-sol", "Old name")];
        let mut custom = model("openai/gpt-6.1-sol", "Gateway model");
        custom.provider_name = Some("Vercel AI Gateway".into());
        custom.default = true;
        merge_custom_models(&mut models, vec![custom]);
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "gpt-6.1-sol");
        assert!(models[0].provider_name.is_none());
        assert!(!models[0].default);
        assert_eq!(models[1].name, "Gateway model");
        assert_eq!(
            models[1].provider_name.as_deref(),
            Some("Vercel AI Gateway")
        );
        assert!(models[1].default);
    }

    #[test]
    fn gateway_catalog_preserves_ids_controls_and_configured_default() {
        let models = parse_models(
            &json!({"models":[
                {"slug":"openai/gpt-6.1-sol","display_name":"GPT-6.1 Sol","visibility":"list","supported_in_api":true,"supported_reasoning_levels":[{"effort":"low"},{"effort":"high"}],"default_reasoning_level":"low","service_tiers":[{"id":"priority","name":"Fast"}]},
                {"slug":"openai/gpt-6.1-sol-fast","display_name":"GPT-6.1 Sol (Fast)"},
                {"slug":"hidden","visibility":"hide"},
                {"slug":"unsupported","supported_in_api":false},
                {"slug":"openai/gpt-6.1-sol"}
            ]}),
            Some("openai/gpt-6.1-sol-fast"),
        );
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "openai/gpt-6.1-sol-fast");
        assert!(models[0].default);
        assert_eq!(models[0].name, "GPT-6.1 Sol (Fast)");
        assert_eq!(models[1].efforts, ["low", "high"]);
        assert_eq!(models[1].default_effort.as_deref(), Some("low"));
        assert_eq!(models[1].tiers, [("priority".into(), "Fast".into())]);
    }

    #[test]
    fn openai_compatible_catalog_excludes_non_language_models() {
        let models = parse_models(
            &json!({"data":[
                {"id":"custom/model","name":"Custom model"},
                {"id":"embedding","type":"embedding"},
                {"id":"hidden","hidden":true}
            ]}),
            None,
        );
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "custom/model");
        assert_eq!(models[0].name, "Custom model");
    }

    #[test]
    fn gateway_fast_aliases_keep_base_controls_without_duplicate_tiers() {
        let mut models = parse_models(
            &json!({"models":[{
                "slug":"openai/gpt-6.1-sol","supported_reasoning_levels":[{"effort":"low"},{"effort":"ultra"}],
                "default_reasoning_level":"low","service_tiers":[{"id":"priority","name":"Fast"}]
            }]}),
            Some("openai/gpt-6.1-sol-fast"),
        );
        let aliases = json!({"data":[
            {"id":"openai/gpt-6.1-sol-fast","name":"GPT-6.1 Sol (Fast)","type":"language"},
            {"id":"unsupported-fast","type":"language"},
            {"id":"openai/gpt-6.1-sol-batch","type":"language"}
        ]});
        add_gateway_aliases(&mut models, &aliases, Some("openai/gpt-6.1-sol-fast"));
        add_gateway_aliases(&mut models, &aliases, Some("openai/gpt-6.1-sol-fast"));
        assert_eq!(models.len(), 2);
        assert_eq!(models[1].id, "openai/gpt-6.1-sol-fast");
        assert_eq!(models[1].name, "GPT-6.1 Sol (Fast)");
        assert!(models[1].default);
        assert_eq!(models[1].efforts, ["low", "ultra"]);
        assert_eq!(models[1].default_effort.as_deref(), Some("low"));
        assert!(models[1].tiers.is_empty());
    }
}
